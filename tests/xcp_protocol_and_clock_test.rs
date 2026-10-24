// SPDX-License-Identifier: GPL-2.0-only
//! Unit tests for XCP on Ethernet framing, CTO builders, target clock correlation
//! (`GET_DAQ_CLOCK` and DTO 32-bit rollover), and `DaqPlanner` multi-rate unpacking.

use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::xcp::clock_sync::TargetClockSync;
use open_xcp_client::xcp::daq_planner::{DaqPlanner, DecodedValue};
use open_xcp_client::xcp::packet::{
    cto, encode_xcp_ethernet_frame, unpack_xcp_ethernet_frames, XcpPacketKind, IDT_ASAM_UPLOAD,
};

#[test]
fn test_xcp_ethernet_framing_and_packet_classification() {
    let f1 = encode_xcp_ethernet_frame(1, &[0xFF, 0x05, 0x00]);
    let f2 = encode_xcp_ethernet_frame(2, &[0xFE, 0x22, 0x01]);
    let f3 = encode_xcp_ethernet_frame(3, &[0xFD, 0x01]);
    let f4 = encode_xcp_ethernet_frame(4, &[0xFC, 0x02]);
    let f5 = encode_xcp_ethernet_frame(5, &[0x00, 0x01, 0xAA, 0xBB]);

    let mut combined = Vec::new();
    combined.extend_from_slice(&f1);
    combined.extend_from_slice(&f2);
    combined.extend_from_slice(&f3);
    combined.extend_from_slice(&f4);
    combined.extend_from_slice(&f5);

    let (frames, consumed) = unpack_xcp_ethernet_frames(&combined);
    assert_eq!(consumed, combined.len());
    assert_eq!(frames.len(), 5);
    assert_eq!(
        frames[0].classify(),
        Some(XcpPacketKind::Response(vec![0x05, 0x00]))
    );
    assert_eq!(
        frames[1].classify(),
        Some(XcpPacketKind::Error {
            code: 0x22,
            data: vec![0x01]
        })
    );
    assert_eq!(frames[2].classify(), Some(XcpPacketKind::Event(vec![0x01])));
    assert_eq!(
        frames[3].classify(),
        Some(XcpPacketKind::Service(vec![0x02]))
    );
    assert_eq!(
        frames[4].classify(),
        Some(XcpPacketKind::DaqDto {
            odt_num: 0,
            daq_list_num: 1,
            payload: vec![0xAA, 0xBB]
        })
    );

    // Verify all CTO helper builders produce expected command codes
    assert_eq!(cto::connect(0)[0], 0xFF);
    assert_eq!(cto::disconnect()[0], 0xFE);
    assert_eq!(cto::get_status()[0], 0xFD);
    assert_eq!(cto::get_comm_mode_info()[0], 0xFB);
    assert_eq!(cto::get_id(IDT_ASAM_UPLOAD), vec![0xFA, 4]);
    assert_eq!(cto::set_mta(0x1000, 0)[0], 0xF6);
    assert_eq!(cto::upload(32), vec![0xF5, 32]);
    assert_eq!(cto::short_upload(4, 0x1000, 0)[0], 0xF4);
    assert_eq!(cto::download(&[1, 2, 3, 4])[0], 0xF0);
    assert_eq!(cto::short_download(0x1000, 0, &[1, 2])[0], 0xED);
    assert_eq!(cto::set_cal_page(3, 0, 1), vec![0xEB, 3, 0, 1]);
    assert_eq!(cto::get_daq_clock(), vec![0xDC]);
    assert_eq!(cto::free_daq(), vec![0xD6]);
    assert_eq!(cto::alloc_daq(2)[0], 0xD5);
    assert_eq!(cto::alloc_odt(0, 1)[0], 0xD4);
    assert_eq!(cto::alloc_odt_entry(0, 0, 4)[0], 0xD3);
    assert_eq!(cto::set_daq_ptr(0, 0, 0)[0], 0xE2);
    assert_eq!(cto::write_daq(0xFF, 4, 0, 0x1000)[0], 0xE1);
    assert_eq!(cto::set_daq_list_mode(0x10, 0, 0, 1, 0)[0], 0xE0);
    assert_eq!(cto::start_stop_daq_list(2, 0)[0], 0xDE);
    assert_eq!(cto::start_stop_synch(1), vec![0xDD, 1]);
}

#[test]
fn test_target_clock_sync_native_utc_and_monotonic_rollover() {
    let mut sync = TargetClockSync::new(true);
    assert!(!sync.is_synchronized());

    // 1. Test 64-bit native UTC target clock response (>= 2020-01-01)
    let target_utc_ns: u64 = 1_792_716_000_000_000_000;
    let mut payload_64 = vec![0x00, 0x00, 0x02];
    payload_64.extend_from_slice(&target_utc_ns.to_le_bytes());
    sync.update_from_get_daq_clock_response(&payload_64, target_utc_ns as i64);
    assert!(sync.is_synchronized());
    assert!(sync.is_target_native_utc());

    let low32 = (target_utc_ns & 0xFFFF_FFFF) as u32;
    let (s, ns) = sync.resolve_sample_utc(Some(low32.wrapping_add(1_000_000)));
    assert_eq!(ns, (target_utc_ns + 1_000_000) as i64);
    assert!((s - (ns as f64 * 1e-9)).abs() < 1e-6);

    // 2. Test 32-bit monotonic clock response + 32-bit rollover handling
    let mut mono_sync = TargetClockSync::new(true);
    let host_anchor_ns: i64 = 1_792_716_000_000_000_000;
    let init_ticks: u32 = 0xFFFF_FF00;
    let mut payload_32 = vec![0, 0, 0];
    payload_32.extend_from_slice(&init_ticks.to_le_bytes());
    mono_sync.update_from_get_daq_clock_response(&payload_32, host_anchor_ns);
    assert!(mono_sync.is_synchronized());
    assert!(!mono_sync.is_target_native_utc());

    // Advance across 32-bit boundary (from 0xFFFF_FF00 -> 0x0000_0100 = +512 ns)
    let (_, after_rollover_ns) = mono_sync.resolve_sample_utc(Some(0x0000_0100));
    assert_eq!(after_rollover_ns - host_anchor_ns, 512);

    // 3. Host fallback when prefer_target_utc is false
    let mut host_only = TargetClockSync::new(false);
    let (hs, hns) = host_only.resolve_sample_utc(Some(12345));
    assert!(hs > 1_700_000_000.0);
    assert!(hns > 1_700_000_000_000_000_000);

    // 4. Replies shorter than the 7-byte 32-bit form carry no timestamp (bytes 0..3 are
    //    reserved / trigger info) and must leave the sync unanchored; a None DTO timestamp
    //    then falls back to the host clock.
    let mut short_sync = TargetClockSync::default();
    short_sync.update_from_get_daq_clock_response(&[1, 2], 1_792_716_000_000_000_000);
    assert!(!short_sync.is_synchronized());
    short_sync
        .update_from_get_daq_clock_response(&1000u32.to_le_bytes(), 1_792_716_000_000_000_000);
    assert!(
        !short_sync.is_synchronized(),
        "a 4-byte reply has no timestamp field and must not anchor the clock"
    );
    short_sync.update_from_get_daq_clock_response(&[0, 0, 0, 0, 0, 0], 1_792_716_000_000_000_000);
    assert!(!short_sync.is_synchronized());
    let (none_s, _) = short_sync.resolve_sample_utc(None);
    assert!(none_s > 1_700_000_000.0);
    let mut payload_7 = vec![0, 0, 0];
    payload_7.extend_from_slice(&1000u32.to_le_bytes());
    short_sync.update_from_get_daq_clock_response(&payload_7, 1_792_716_000_000_000_000);
    assert!(short_sync.is_synchronized());
}

#[test]
fn test_daq_planner_packing_and_dto_decoding() {
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    // Force small ODT payload size (24 bytes) to exercise multi-ODT splitting per DAQ list
    let mut planner = DaqPlanner::from_a2l(&model, 24);
    assert_eq!(planner.lists.len(), 2);
    assert!(planner.lists[0].odts.len() >= 2);

    let mut clock = TargetClockSync::new(true);
    clock.anchor_target_clock(1_792_716_000_000_000_000, 1_792_716_000_000_000_000);

    let list0 = planner.lists[0].clone();
    let mut final_sample = None;
    for odt in &list0.odts {
        let mut dto_payload = Vec::new();
        if odt.has_timestamp {
            dto_payload.extend_from_slice(&1000u32.to_le_bytes());
        }
        for entry in &odt.entries {
            dto_payload.extend(std::iter::repeat_n(0u8, entry.byte_size as usize));
        }
        final_sample = planner.process_dto_packet(
            odt.odt_num,
            list0.daq_list_num as u8,
            &dto_payload,
            0,
            &model,
            &mut clock,
        );
    }

    let sample = final_sample.expect("Final ODT should produce a complete DaqEventSample");
    assert_eq!(sample.event_channel_id, 0);
    assert_eq!(sample.values.len(), list0.measurement_indices.len());
    for val in sample.values.values() {
        assert!(!val.format_compact().is_empty());
        assert_eq!(val.primary_scalar(), 0.0);
    }

    let arr_val = DecodedValue::Array(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    assert!(arr_val.format_compact().contains("5 elems"));
}
