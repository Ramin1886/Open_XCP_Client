// SPDX-License-Identifier: GPL-2.0-only
//! Regression tests for the XCP `u8` DAQ allocation limits.
//!
//! `ALLOC_ODT`, `ALLOC_ODT_ENTRY` and `SET_DAQ_PTR` carry the ODT number and the ODT entry
//! number in a single byte. A plan that exceeds either limit used to be truncated by an
//! `as u8` cast, which produced duplicate ODT numbers and an entry count the slave rejected
//! with `ERR_MEMORY_OVERFLOW (0x22)` — observed on a real target with 1780 measurements, where
//! a single ODT was planned with more than 255 one-byte entries.

use open_xcp_client::a2l::model::{A2lDataType, A2lEvent, A2lModel, Measurement};
use open_xcp_client::xcp::daq_planner::{
    DaqPlanner, MAX_DAQ_LISTS, MAX_ODTS_PER_DAQ_LIST, MAX_ODT_ENTRIES,
};

/// Builds a model with `count` one-byte scalar measurements on a single event channel.
fn model_with_byte_measurements(count: usize) -> A2lModel {
    let mut model = A2lModel {
        project_name: "LimitProbe".to_string(),
        module_name: "LimitProbe".to_string(),
        ..A2lModel::default()
    };
    model.events.insert(
        0x19,
        A2lEvent {
            channel_id: 0x19,
            name: "Globals".to_string(),
            short_name: "Glb".to_string(),
            cycle_time_us: 10_000,
        },
    );
    for i in 0..count {
        model.measurements.push(Measurement {
            name: format!("Flag_{:04}", i),
            description: String::new(),
            data_type: A2lDataType::UByte,
            compu_method: "NO_COMPU_METHOD".to_string(),
            lower_limit: 0.0,
            upper_limit: 255.0,
            address: 0x6800_0000 + i as u32,
            address_extension: 1,
            matrix_dim: Vec::new(),
            unit: "-".to_string(),
            event_id: Some(0x19),
        });
    }
    model
}

#[test]
fn test_odt_entry_count_never_exceeds_255() {
    // 1000 one-byte measurements fit a 1018-byte DTO payload by size, so only the entry
    // limit can split them. Before the fix this produced one ODT of 1000 entries, whose
    // count truncated to 232 in ALLOC_ODT_ENTRY.
    let model = model_with_byte_measurements(1000);
    let planner = DaqPlanner::from_a2l(&model, 1018);

    assert_eq!(planner.lists.len(), 1, "one DAQ list per event channel");
    let list = &planner.lists[0];
    assert_eq!(list.event_channel_id, 0x19);

    for odt in &list.odts {
        assert!(
            odt.entries.len() <= MAX_ODT_ENTRIES,
            "ODT {} holds {} entries, over the {} limit",
            odt.odt_num,
            odt.entries.len(),
            MAX_ODT_ENTRIES
        );
        assert_eq!(
            odt.entries.len(),
            odt.entries.len() as u8 as usize,
            "entry count must survive the u8 cast in ALLOC_ODT_ENTRY"
        );
    }

    // Every measurement is still acquired, and every ODT number is distinct.
    assert_eq!(list.measurement_indices.len(), 1000);
    assert!(list.dropped_measurement_indices.is_empty());
    assert!(planner.dropped_measurement_indices().is_empty());
    assert_eq!(planner.total_odt_entries(), 1000);
    assert_eq!(planner.total_daq_bytes(), 1000);

    let mut nums: Vec<u8> = list.odts.iter().map(|o| o.odt_num).collect();
    let before = nums.len();
    nums.sort_unstable();
    nums.dedup();
    assert_eq!(nums.len(), before, "ODT numbers must be unique");
    assert_eq!(
        list.odts.len(),
        list.odts.len() as u8 as usize,
        "ODT count must survive the u8 cast in ALLOC_ODT"
    );

    // Exactly one ODT carries the DAQ timestamp, and it is ODT 0.
    let stamped: Vec<u8> = list
        .odts
        .iter()
        .filter(|o| o.has_timestamp)
        .map(|o| o.odt_num)
        .collect();
    assert_eq!(stamped, vec![0]);
}

#[test]
fn test_first_odt_reserves_room_for_the_daq_timestamp() {
    // With a 32-byte payload the first ODT may only carry 28 bytes of measurement data,
    // because the slave prepends a 4-byte DAQ timestamp to it.
    let model = model_with_byte_measurements(200);
    let planner = DaqPlanner::from_a2l(&model, 32);
    let list = &planner.lists[0];

    let first = &list.odts[0];
    assert!(first.has_timestamp);
    let first_bytes: usize = first.entries.iter().map(|e| e.byte_size as usize).sum();
    assert!(
        first_bytes + 4 <= 32,
        "first ODT carries {first_bytes} data bytes plus a 4-byte timestamp, over MAX_DTO"
    );

    for odt in list.odts.iter().skip(1) {
        let bytes: usize = odt.entries.iter().map(|e| e.byte_size as usize).sum();
        assert!(bytes <= 32, "ODT {} carries {} bytes", odt.odt_num, bytes);
    }
}

#[test]
fn test_measurements_beyond_the_odt_ceiling_are_dropped_not_truncated() {
    // A 16-byte payload allows 12 data bytes in ODT 0 and 16 afterwards, so the ODT ceiling
    // holds far fewer than 8000 one-byte measurements: the remainder must be reported as dropped
    // rather than silently folded into a malformed allocation.
    let model = model_with_byte_measurements(8000);
    let planner = DaqPlanner::from_a2l(&model, 16);
    let list = &planner.lists[0];

    assert_eq!(list.odts.len(), MAX_ODTS_PER_DAQ_LIST);
    assert_eq!(
        list.odts.len(),
        list.odts.len() as u8 as usize,
        "ODT count must survive the u8 cast in ALLOC_ODT"
    );

    let dropped = planner.dropped_measurement_indices();
    assert!(!dropped.is_empty(), "the overflow must be reported");
    assert_eq!(
        list.measurement_indices.len() + dropped.len(),
        8000,
        "every measurement is either acquired or reported as dropped"
    );

    // No measurement is both acquired and dropped, and none is half-acquired.
    for idx in &dropped {
        assert!(!list.measurement_indices.contains(idx));
        assert!(
            !list
                .odts
                .iter()
                .any(|o| o.entries.iter().any(|e| e.measurement_index == *idx)),
            "dropped measurement {idx} still has ODT entries"
        );
    }
    for odt in &list.odts {
        for entry in &odt.entries {
            assert!(list.measurement_indices.contains(&entry.measurement_index));
        }
    }
}

#[test]
fn test_large_arrays_are_chunked_without_exceeding_limits() {
    // One measurement larger than a single ODT must be split across ODTs with contiguous
    // byte offsets covering it exactly once.
    let mut model = model_with_byte_measurements(0);
    model.measurements.push(Measurement {
        name: "BigArray".to_string(),
        description: String::new(),
        data_type: A2lDataType::Float32Ieee,
        compu_method: "NO_COMPU_METHOD".to_string(),
        lower_limit: 0.0,
        upper_limit: 1.0,
        address: 0x6900_0000,
        address_extension: 1,
        matrix_dim: vec![400],
        unit: "m".to_string(),
        event_id: Some(0x19),
    });

    let planner = DaqPlanner::from_a2l(&model, 64);
    let list = &planner.lists[0];
    assert!(list.odts.len() > 1, "1600 bytes cannot fit one 64-byte ODT");
    assert!(list.dropped_measurement_indices.is_empty());

    let mut offsets: Vec<usize> = Vec::new();
    for odt in &list.odts {
        assert!(odt.entries.len() <= MAX_ODT_ENTRIES);
        for e in &odt.entries {
            assert_eq!(e.measurement_index, 0);
            assert_eq!(e.address, 0x6900_0000 + e.element_byte_offset as u32);
            offsets.push(e.element_byte_offset);
        }
    }
    let total: usize = list
        .odts
        .iter()
        .flat_map(|o| o.entries.iter())
        .map(|e| e.byte_size as usize)
        .sum();
    assert_eq!(total, 1600, "the whole array is covered exactly once");
    let entry_count: usize = list.odts.iter().map(|o| o.entries.len()).sum();
    offsets.sort_unstable();
    let distinct = offsets.len();
    offsets.dedup();
    assert_eq!(offsets.len(), distinct, "byte offsets must not repeat");
    assert_eq!(offsets.len(), entry_count);
    assert_eq!(offsets[0], 0);
    assert!(
        *offsets.last().unwrap() < total,
        "the last chunk must start inside the array"
    );
}

#[test]
fn test_odt_numbers_stay_clear_of_reserved_packet_ids() {
    // With absolute ODT numbering the ODT number doubles as the DTO packet identifier, and
    // 0xFC..=0xFF are reserved for SERV / EV / ERR / RES. A full list must therefore stop at 251.
    let model = model_with_byte_measurements(8000);
    let planner = DaqPlanner::from_a2l(&model, 16);
    let list = &planner.lists[0];
    assert_eq!(list.odts.len(), MAX_ODTS_PER_DAQ_LIST);
    for odt in &list.odts {
        assert!(
            odt.odt_num < 0xFC,
            "ODT number {} collides with a reserved packet identifier",
            odt.odt_num
        );
    }
}

#[test]
fn test_event_channels_beyond_256_lists_are_reported_dropped_not_aliased() {
    // The DTO header names the DAQ list in one byte, so a 300-event A2L cannot be fully
    // acquired: lists 0..=255 are planned and the remaining channels are reported as dropped.
    let mut model = A2lModel {
        project_name: "ManyEvents".to_string(),
        module_name: "ManyEvents".to_string(),
        ..A2lModel::default()
    };
    for ev in 0..300u16 {
        model.events.insert(
            ev,
            A2lEvent {
                channel_id: ev,
                name: format!("Ev_{ev}"),
                short_name: format!("E{ev}"),
                cycle_time_us: 10_000,
            },
        );
        model.measurements.push(Measurement {
            name: format!("Sig_{ev}"),
            description: String::new(),
            data_type: A2lDataType::UByte,
            compu_method: "NO_COMPU_METHOD".to_string(),
            lower_limit: 0.0,
            upper_limit: 255.0,
            address: 0x7000_0000 + ev as u32,
            address_extension: 0,
            matrix_dim: Vec::new(),
            unit: "-".to_string(),
            event_id: Some(ev),
        });
    }
    let planner = DaqPlanner::from_a2l(&model, 256);
    assert_eq!(planner.lists.len(), MAX_DAQ_LISTS);
    assert!(planner
        .lists
        .iter()
        .all(|l| usize::from(l.daq_list_num as u8) == l.daq_list_num as usize));
    let dropped = planner.dropped_measurement_indices();
    assert_eq!(dropped.len(), 300 - MAX_DAQ_LISTS);
    assert!(dropped
        .iter()
        .all(|&i| model.measurements[i].event_id.unwrap() >= 256));
    assert_eq!(planner.total_odt_entries(), MAX_DAQ_LISTS);
}
