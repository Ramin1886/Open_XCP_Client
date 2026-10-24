// SPDX-License-Identifier: GPL-2.0-only
//! Regression tests for DAQ data-integrity accounting.
//!
//! Two defects motivate these: the 16-bit XCP-on-Ethernet `CTR` was parsed and discarded, so a
//! dropped UDP datagram was invisible; and a DAQ cycle was emitted whenever the highest *planned*
//! ODT number arrived, so a lost ODT produced a sample silently stitched from two different event
//! firings and indistinguishable from a good one.

use open_xcp_client::a2l::model::{A2lDataType, A2lEvent, A2lModel, Measurement};
use open_xcp_client::xcp::clock_sync::TargetClockSync;
use open_xcp_client::xcp::daq_planner::{CycleDiscardReason, DaqPlanner, DecodedValue};
use open_xcp_client::xcp::link_stats::{CtrObservation, LinkStats};

/// Anchors a clock so `resolve_sample_utc` yields a target-derived timestamp.
fn anchored_clock() -> TargetClockSync {
    let mut clock = TargetClockSync::new(true);
    clock.anchor_target_clock(1_792_716_000_000_000_000, 1_792_716_000_000_000_000);
    clock
}

/// Builds a model whose single event holds `count` one-byte measurements.
fn model_with(count: usize) -> A2lModel {
    let mut model = A2lModel {
        project_name: "LossProbe".to_string(),
        module_name: "LossProbe".to_string(),
        ..A2lModel::default()
    };
    model.events.insert(
        7,
        A2lEvent {
            channel_id: 7,
            name: "Cyclic".to_string(),
            short_name: "Cyc".to_string(),
            cycle_time_us: 10_000,
        },
    );
    for i in 0..count {
        model.measurements.push(Measurement {
            name: format!("Sig_{:03}", i),
            description: String::new(),
            data_type: A2lDataType::UByte,
            compu_method: "NO_COMPU_METHOD".to_string(),
            lower_limit: 0.0,
            upper_limit: 255.0,
            address: 0x4000_0000 + i as u32,
            address_extension: 0,
            matrix_dim: Vec::new(),
            unit: String::new(),
            event_id: Some(7),
        });
    }
    model
}

/// Builds the payload for one ODT of `planner`'s only list, filling every entry with `fill`.
fn odt_payload(planner: &DaqPlanner, odt_num: u8, timestamp: u32, fill: u8) -> Vec<u8> {
    let odt = planner.lists[0]
        .odts
        .iter()
        .find(|o| o.odt_num == odt_num)
        .expect("planned ODT");
    let mut payload = Vec::new();
    if odt.has_timestamp {
        payload.extend_from_slice(&timestamp.to_le_bytes());
    }
    for entry in &odt.entries {
        payload.extend(std::iter::repeat_n(fill, entry.byte_size as usize));
    }
    payload
}

/// Feeds one ODT of DAQ list 0 into `planner` at link generation `gen`.
fn feed(
    planner: &mut DaqPlanner,
    model: &A2lModel,
    clock: &mut TargetClockSync,
    odt_num: u8,
    gen: u64,
    timestamp: u32,
    fill: u8,
) -> Option<open_xcp_client::xcp::daq_planner::DaqEventSample> {
    let payload = odt_payload(planner, odt_num, timestamp, fill);
    planner.process_dto_packet(odt_num, 0, &payload, gen, model, clock)
}

/// Builds a planner whose single DAQ list has exactly `odt_count` ODTs.
fn planner_with_odts(odt_count: usize) -> (A2lModel, DaqPlanner) {
    // `from_a2l` clamps the ODT payload to at least 16 bytes. With one-byte measurements that is
    // 16 entries per ODT, less the 4 bytes ODT 0 reserves for the DAQ timestamp. Pick the smallest
    // measurement count that spills exactly into the requested number of ODTs.
    const PAYLOAD: usize = 16;
    const FIRST_ODT_ENTRIES: usize = PAYLOAD - 4;
    let count = if odt_count <= 1 {
        FIRST_ODT_ENTRIES
    } else {
        FIRST_ODT_ENTRIES + (odt_count - 2) * PAYLOAD + 1
    };
    let model = model_with(count);
    let planner = DaqPlanner::from_a2l(&model, PAYLOAD);
    assert_eq!(
        planner.lists[0].odts.len(),
        odt_count,
        "fixture must produce exactly {odt_count} ODTs"
    );
    (model, planner)
}

#[test]
fn test_complete_in_order_cycle_emits_one_sample() {
    let (model, mut planner) = planner_with_odts(3);
    let mut clock = anchored_clock();

    let mut emitted = 0;
    for odt in 0..3u8 {
        if let Some(sample) = feed(&mut planner, &model, &mut clock, odt, 0, 1_000, 0xAB) {
            emitted += 1;
            assert_eq!(sample.event_channel_id, 7);
            assert_eq!(sample.values.len(), model.measurements.len());
            for val in sample.values.values() {
                assert_eq!(*val, DecodedValue::Scalar(0xAB as f64));
            }
            assert!(sample.time_utc_s > 1_700_000_000.0, "target timestamp used");
        }
    }
    assert_eq!(emitted, 1, "exactly one sample per complete cycle");
    assert_eq!(planner.loss_stats()[0].cycles_emitted, 1);
    assert!(planner.is_clean());
    assert!(planner.take_cycle_discards().is_empty());
}

#[test]
fn test_single_odt_list_emits_on_odt_zero_alone() {
    let (model, mut planner) = planner_with_odts(1);
    let mut clock = anchored_clock();
    assert!(feed(&mut planner, &model, &mut clock, 0, 0, 500, 0x11).is_some());
    assert!(planner.is_clean());
}

#[test]
fn test_missed_middle_odt_emits_nothing() {
    // ODTs 0,1,3 of a 4-ODT list: ODT 2 is lost. The old design emitted on ODT 3 because it was
    // the highest planned ODT, silently carrying ODT 2's bytes from the previous firing.
    let (model, mut planner) = planner_with_odts(4);
    let mut clock = anchored_clock();

    // The lost ODT consumed a CTR value, so the link generation moves on.
    assert!(feed(&mut planner, &model, &mut clock, 0, 0, 2_000, 0x22).is_none());
    assert!(feed(&mut planner, &model, &mut clock, 1, 0, 2_000, 0x22).is_none());
    assert!(
        feed(&mut planner, &model, &mut clock, 3, 1, 2_000, 0x22).is_none(),
        "an incomplete cycle must never emit"
    );
    assert_eq!(planner.loss_stats()[0].cycles_emitted, 0);

    // Even without a link signal, the gap in ODT numbering must not complete the cycle.
    let (model2, mut planner2) = planner_with_odts(4);
    let mut clock2 = anchored_clock();
    for odt in [0u8, 1, 3] {
        assert!(feed(&mut planner2, &model2, &mut clock2, odt, 0, 2_000, 0x22).is_none());
    }
    assert_eq!(planner2.loss_stats()[0].cycles_emitted, 0);
}

/// The case ODT-0 anchoring alone still gets wrong: `ODTmax` of cycle `N` and ODT 0 of cycle `N+1`
/// are lost in one burst, so cycle `N` stays open and the next firing's ODTs complete it with
/// perfectly consecutive ODT numbers. Both lost datagrams consumed a `CTR`, so the link generation
/// moves — which is what lets the planner reject it.
#[test]
fn test_two_packet_burst_never_fuses_two_cycles() {
    for odt_count in [2usize, 5] {
        let (model, mut planner) = planner_with_odts(odt_count);
        let mut clock = anchored_clock();
        let last = (odt_count - 1) as u8;

        // Cycle N: every ODT except the final one, all in generation 0.
        for odt in 0..last {
            assert!(feed(&mut planner, &model, &mut clock, odt, 0, 3_000, 0xC1).is_none());
        }

        // Burst drop of ODTmax(N) and ODT0(N+1) bumps the generation to 1.
        for odt in 1..=last {
            assert!(
                feed(&mut planner, &model, &mut clock, odt, 1, 4_000, 0xC2).is_none(),
                "a cycle must never be completed across a link discontinuity \
                 (odt_count={odt_count}, odt={odt})"
            );
        }

        assert_eq!(
            planner.loss_stats()[0].cycles_emitted,
            0,
            "no sample may be stitched from two firings (odt_count={odt_count})"
        );
        let discards = planner.take_cycle_discards();
        assert!(
            discards.iter().any(|d| matches!(
                d.reason,
                CycleDiscardReason::LinkDiscontinuity | CycleDiscardReason::MissingHeader
            )),
            "the burst must be reported (odt_count={odt_count}): {discards:?}"
        );
    }
}

#[test]
fn test_lost_odt_zero_discards_without_touching_staging() {
    let (model, mut planner) = planner_with_odts(3);
    let mut clock = anchored_clock();

    // A full good cycle first, so the staging buffer holds known bytes.
    for odt in 0..3u8 {
        feed(&mut planner, &model, &mut clock, odt, 0, 1_000, 0x55);
    }

    // Next cycle loses ODT 0; the surviving ODTs carry 0xEE and must not reach the staging buffer.
    for odt in 1..3u8 {
        assert!(feed(&mut planner, &model, &mut clock, odt, 1, 2_000, 0xEE).is_none());
    }
    assert!(planner.loss_stats()[0].cycles_lost_no_header > 0);

    // A following complete cycle must report only its own bytes, proving 0xEE never landed.
    let mut seen = None;
    for odt in 0..3u8 {
        if let Some(sample) = feed(&mut planner, &model, &mut clock, odt, 2, 3_000, 0x77) {
            seen = Some(sample);
        }
    }
    let sample = seen.expect("the clean cycle emits");
    for val in sample.values.values() {
        assert_eq!(
            *val,
            DecodedValue::Scalar(0x77 as f64),
            "no byte may survive from a discarded cycle"
        );
    }
}

#[test]
fn test_duplicate_and_out_of_order_odt_are_sequence_errors() {
    let (model, mut planner) = planner_with_odts(3);
    let mut clock = anchored_clock();

    // Duplicate ODT 1 within one cycle, same generation so only ordering can reject it.
    for odt in [0u8, 1, 1] {
        assert!(feed(&mut planner, &model, &mut clock, odt, 0, 1_000, 0x33).is_none());
    }
    assert!(planner.loss_stats()[0].odt_sequence_errors > 0);
    assert!(planner
        .take_cycle_discards()
        .iter()
        .any(|d| d.reason == CycleDiscardReason::SequenceError));

    // Out-of-order: ODT 2 then ODT 1 inside one cycle.
    let before = planner.loss_stats()[0].odt_sequence_errors;
    for odt in [0u8, 2, 1] {
        feed(&mut planner, &model, &mut clock, odt, 0, 2_000, 0x44);
    }
    assert!(planner.loss_stats()[0].odt_sequence_errors > before);
}

#[test]
fn test_new_header_supersedes_incomplete_cycle() {
    let (model, mut planner) = planner_with_odts(3);
    let mut clock = anchored_clock();

    feed(&mut planner, &model, &mut clock, 0, 0, 1_000, 0x01);
    // A new ODT 0 before the cycle completed.
    feed(&mut planner, &model, &mut clock, 0, 0, 2_000, 0x02);

    assert_eq!(planner.loss_stats()[0].cycles_discarded, 1);
    assert!(planner
        .take_cycle_discards()
        .iter()
        .any(|d| d.reason == CycleDiscardReason::Superseded));
}

#[test]
fn test_short_payload_discards_before_writing() {
    let (model, mut planner) = planner_with_odts(2);
    let mut clock = anchored_clock();

    // ODT 0 with a truncated timestamp.
    assert!(planner
        .process_dto_packet(0, 0, &[0x01, 0x02], 0, &model, &mut clock)
        .is_none());
    assert!(planner.loss_stats()[0].cycles_discarded > 0);
    assert!(planner
        .take_cycle_discards()
        .iter()
        .any(|d| d.reason == CycleDiscardReason::ShortPayload));

    // ODT 1 shorter than its planned entries.
    feed(&mut planner, &model, &mut clock, 0, 0, 1_000, 0x09);
    assert!(planner
        .process_dto_packet(1, 0, &[], 0, &model, &mut clock)
        .is_none());
    assert!(planner
        .take_cycle_discards()
        .iter()
        .any(|d| d.reason == CycleDiscardReason::ShortPayload));
    assert_eq!(planner.loss_stats()[0].cycles_emitted, 0);
}

#[test]
fn test_link_discontinuity_invalidates_open_cycles() {
    let (model, mut planner) = planner_with_odts(3);
    let mut clock = anchored_clock();

    feed(&mut planner, &model, &mut clock, 0, 0, 1_000, 0x05);
    assert_eq!(
        planner.invalidate_open_cycles(CycleDiscardReason::LinkDiscontinuity),
        1
    );
    // The remaining ODTs now have no open cycle and must not complete one.
    for odt in 1..3u8 {
        assert!(feed(&mut planner, &model, &mut clock, odt, 0, 1_000, 0x05).is_none());
    }
    assert_eq!(planner.loss_stats()[0].cycles_emitted, 0);
    assert_eq!(
        planner.invalidate_open_cycles(CycleDiscardReason::LinkDiscontinuity),
        0
    );
}

#[test]
fn test_unplanned_dto_is_counted() {
    let (model, mut planner) = planner_with_odts(2);
    let mut clock = anchored_clock();
    // Unknown DAQ list.
    assert!(planner
        .process_dto_packet(0, 9, &[0; 16], 0, &model, &mut clock)
        .is_none());
    // Known list, ODT number beyond the plan.
    assert!(planner
        .process_dto_packet(200, 0, &[0; 16], 0, &model, &mut clock)
        .is_none());
    assert_eq!(planner.unplanned_dto_count(), 2);
}

#[test]
fn test_ctr_wrap_is_not_loss() {
    let mut stats = LinkStats::new();
    assert_eq!(stats.observe_ctr(0xFFFD), CtrObservation::First);
    assert_eq!(stats.observe_ctr(0xFFFE), CtrObservation::InOrder);
    assert_eq!(stats.observe_ctr(0xFFFF), CtrObservation::InOrder);
    assert_eq!(stats.observe_ctr(0x0000), CtrObservation::InOrder);
    assert_eq!(stats.observe_ctr(0x0001), CtrObservation::InOrder);
    assert!(stats.is_clean(), "a 16-bit rollover is not loss");
    assert_eq!(stats.frames_received, 5);
    assert_eq!(stats.generation, 0);
}

#[test]
fn test_ctr_gaps_reorder_and_duplicate() {
    let mut stats = LinkStats::new();
    stats.observe_ctr(10);
    // One frame missing.
    assert_eq!(stats.observe_ctr(12), CtrObservation::Gap { lost: 1 });
    assert_eq!(stats.frames_lost_estimate, 1);
    assert_eq!(stats.gaps_detected, 1);
    // Resynchronized: the next in-order frame is not counted again.
    assert_eq!(stats.observe_ctr(13), CtrObservation::InOrder);
    assert_eq!(stats.gaps_detected, 1);
    // A larger burst.
    assert_eq!(stats.observe_ctr(20), CtrObservation::Gap { lost: 6 });
    assert_eq!(stats.frames_lost_estimate, 7);
    // Duplicate of the frame just seen.
    assert_eq!(stats.observe_ctr(20), CtrObservation::Duplicate);
    assert_eq!(stats.duplicated, 1);
    // Still expecting 21, so an old frame is reordering.
    assert_eq!(stats.observe_ctr(15), CtrObservation::Reordered);
    assert_eq!(stats.reordered, 1);

    assert!(!stats.is_clean());
    assert_eq!(
        stats.generation, 4,
        "every discontinuity bumps the generation"
    );

    stats.note_channel_overflow(3);
    stats.note_unplanned_dto();
    assert_eq!(stats.channel_overflow_drops, 3);
    assert_eq!(stats.unplanned_dto, 1);
    assert!(stats.summary().contains("lost ~7"));

    // A rollover across a gap boundary is still a forward gap, not reordering.
    let mut wrapped = LinkStats::new();
    wrapped.observe_ctr(0xFFFE);
    assert_eq!(wrapped.observe_ctr(0x0002), CtrObservation::Gap { lost: 3 });
}

#[test]
fn test_ctr_observation_discontinuity_predicate() {
    assert!(!CtrObservation::First.is_discontinuity());
    assert!(!CtrObservation::InOrder.is_discontinuity());
    assert!(CtrObservation::Gap { lost: 1 }.is_discontinuity());
    assert!(CtrObservation::Duplicate.is_discontinuity());
    assert!(CtrObservation::Reordered.is_discontinuity());
    assert_eq!(
        CycleDiscardReason::MissingHeader.label(),
        "missing ODT 0 (no timestamp)"
    );
    assert!(!CycleDiscardReason::Superseded.label().is_empty());
    assert!(!CycleDiscardReason::SequenceError.label().is_empty());
    assert!(!CycleDiscardReason::LinkDiscontinuity.label().is_empty());
    assert!(!CycleDiscardReason::ShortPayload.label().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Provenance recorded into the `.sdf`, controller plumbing, and the XCP Info surface.
// ---------------------------------------------------------------------------------------------

#[test]
fn test_sdf_records_gap_records_and_integrity_counters() {
    use open_xcp_client::recording::sdf_writer::{SdfFileReader, SdfIntegritySummary};
    use open_xcp_client::recording::SdfSessionRecorder;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let model = model_with(4);
    let mut rec = SdfSessionRecorder::new(
        &model,
        dir.path(),
        "Integrity",
        chrono::Utc::now(),
        500_000_000,
    )
    .unwrap();

    // Two consecutive discards of the same event for the same reason coalesce into one record.
    rec.note_discarded_cycle(7, "Cyclic", 100.5, "missing ODT 0 (no timestamp)");
    rec.note_discarded_cycle(7, "Cyclic", 100.6, "missing ODT 0 (no timestamp)");
    // A different reason starts a new record.
    rec.note_discarded_cycle(7, "Cyclic", 100.7, "ODT out of order or duplicated");
    // A different event likewise.
    rec.note_discarded_cycle(9, "Other", 100.8, "ODT out of order or duplicated");

    assert_eq!(rec.gaps().len(), 3);
    assert_eq!(rec.gaps()[0].cycles, 2, "same event+reason coalesces");
    assert_eq!(rec.gaps()[0].event_channel_id, 7);
    assert_eq!(rec.gaps()[2].event_channel_id, 9);
    assert_eq!(rec.integrity().cycles_discarded, 4);
    assert!(!rec.integrity().is_clean());

    rec.set_integrity_counters(SdfIntegritySummary {
        link_gaps_detected: 2,
        link_frames_lost_estimate: 5,
        link_reordered: 1,
        link_duplicated: 1,
        channel_overflow_drops: 7,
        unplanned_dto: 3,
        cycles_lost_no_header: 2,
        odt_sequence_errors: 1,
        measurements_not_acquired: 11,
        ..SdfIntegritySummary::default()
    });
    // The per-part discard tally survives being overwritten by link-level totals.
    assert_eq!(rec.integrity().cycles_discarded, 4);
    assert_eq!(rec.integrity().channel_overflow_drops, 7);

    let files = rec.finish().unwrap();
    let reader = SdfFileReader::open(&files[0]).unwrap();
    let integrity = &reader.metadata.integrity;
    assert_eq!(integrity.link_frames_lost_estimate, 5);
    assert_eq!(integrity.channel_overflow_drops, 7);
    assert_eq!(integrity.measurements_not_acquired, 11);
    assert_eq!(integrity.cycles_discarded, 4);
    assert_eq!(reader.metadata.gaps.len(), 3);
    assert_eq!(reader.metadata.gaps[0].cycles, 2);
    assert_eq!(
        reader.metadata.gaps[0].reason,
        "missing ODT 0 (no timestamp)"
    );

    // A default summary reads as clean.
    assert!(SdfIntegritySummary::default().is_clean());
}

#[test]
fn test_provenance_resets_between_parts() {
    use open_xcp_client::recording::SdfSessionRecorder;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let model = model_with(4);
    let mut rec = SdfSessionRecorder::new(
        &model,
        dir.path(),
        "Rotate",
        chrono::Utc::now(),
        500_000_000,
    )
    .unwrap();
    rec.note_discarded_cycle(7, "Cyclic", 1.0, "missing ODT 0 (no timestamp)");
    assert_eq!(rec.gaps().len(), 1);
    rec.force_rotate_part().unwrap();
    assert!(
        rec.gaps().is_empty() && rec.integrity().is_clean(),
        "provenance is per part"
    );
}

#[test]
fn test_controller_handles_link_integrity_event() {
    use open_xcp_client::app_state::{AppController, WorkerEvent, MAX_RECENT_CYCLE_DISCARDS};
    use open_xcp_client::config_store::ConfigStore;
    use open_xcp_client::plot_profile::ProfileStore;
    use open_xcp_client::xcp::daq_planner::{CycleDiscard, DaqListLossStats};
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let mut controller = AppController::new_with_profile_store(
        ConfigStore::new(dir.path().join("settings.json")),
        ProfileStore::new(dir.path().join("profiles")),
    );
    assert!(
        controller.daq_integrity_is_clean(),
        "clean before any report"
    );

    // A clean report must not raise a warning.
    controller.status_message = "untouched".to_string();
    controller.handle_worker_event(WorkerEvent::LinkIntegrity {
        link: LinkStats::new(),
        per_event: vec![(7, DaqListLossStats::default())],
        recent_discards: Vec::new(),
        unplanned_dto: 0,
    });
    assert_eq!(controller.status_message, "untouched");
    assert!(controller.daq_integrity_is_clean());

    // A lossy report warns once, on the transition.
    let mut link = LinkStats::new();
    link.observe_ctr(1);
    link.observe_ctr(5);
    link.note_channel_overflow(2);
    controller.handle_worker_event(WorkerEvent::LinkIntegrity {
        link: link.clone(),
        per_event: vec![(
            7,
            DaqListLossStats {
                cycles_emitted: 100,
                cycles_discarded: 3,
                cycles_lost_no_header: 2,
                odt_sequence_errors: 1,
            },
        )],
        recent_discards: vec![CycleDiscard {
            daq_list_num: 0,
            event_channel_id: 7,
            reason: CycleDiscardReason::MissingHeader,
        }],
        unplanned_dto: 4,
    });
    assert!(!controller.daq_integrity_is_clean());
    assert!(controller.status_message.contains("DAQ integrity"));
    assert!(controller.status_message.contains("cycles discarded 3"));
    assert_eq!(controller.link_stats.unplanned_dto, 4);
    assert_eq!(controller.event_loss[&7].cycles_discarded, 3);
    assert_eq!(controller.recent_cycle_discards.len(), 1);
    assert!(controller
        .integrity_summary()
        .contains("ODT sequence errors 1"));

    // A second lossy report must not overwrite an unrelated message.
    controller.status_message = "something else".to_string();
    controller.handle_worker_event(WorkerEvent::LinkIntegrity {
        link,
        per_event: vec![(
            7,
            DaqListLossStats {
                cycles_discarded: 4,
                ..DaqListLossStats::default()
            },
        )],
        recent_discards: Vec::new(),
        unplanned_dto: 4,
    });
    assert_eq!(controller.status_message, "something else");

    // The discard history is capped.
    for _ in 0..(MAX_RECENT_CYCLE_DISCARDS + 20) {
        controller.handle_worker_event(WorkerEvent::LinkIntegrity {
            link: LinkStats::new(),
            per_event: Vec::new(),
            recent_discards: vec![CycleDiscard {
                daq_list_num: 0,
                event_channel_id: 7,
                reason: CycleDiscardReason::SequenceError,
            }],
            unplanned_dto: 0,
        });
    }
    assert_eq!(
        controller.recent_cycle_discards.len(),
        MAX_RECENT_CYCLE_DISCARDS
    );
}

#[test]
fn test_xcp_info_tab_renders_integrity() {
    use open_xcp_client::app_state::{ActiveTab, AppController, WorkerEvent};
    use open_xcp_client::config_store::ConfigStore;
    use open_xcp_client::plot_profile::ProfileStore;
    use open_xcp_client::ui::OpenXcpClientApp;
    use open_xcp_client::xcp::daq_planner::{CycleDiscard, DaqListLossStats};
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let mut controller = AppController::new_with_profile_store(
        ConfigStore::new(dir.path().join("settings.json")),
        ProfileStore::new(dir.path().join("profiles")),
    );
    let model = model_with(4);
    controller.a2l_model = Some(model.clone());
    controller.connected = true;

    // Populate event telemetry by ingesting two samples whose spacing is far from the A2L cycle,
    // so the cycle-deviation warning renders too.
    let (_m, mut planner) = planner_with_odts(2);
    let mut clock = anchored_clock();
    for round in 0..2u32 {
        for odt in 0..2u8 {
            if let Some(sample) = feed(
                &mut planner,
                &_m,
                &mut clock,
                odt,
                round as u64,
                1_000 + round * 500_000,
                0x10,
            ) {
                controller.ingest_daq_sample(sample);
            }
        }
    }
    assert!(!controller.event_telemetry.is_empty());

    let mut link = LinkStats::new();
    link.observe_ctr(1);
    link.observe_ctr(9);
    link.observe_ctr(9);
    link.note_channel_overflow(5);
    let event_id = *controller.event_telemetry.keys().next().unwrap();
    controller.handle_worker_event(WorkerEvent::LinkIntegrity {
        link,
        per_event: vec![(
            event_id,
            DaqListLossStats {
                cycles_emitted: 10,
                cycles_discarded: 2,
                cycles_lost_no_header: 1,
                odt_sequence_errors: 3,
            },
        )],
        recent_discards: vec![CycleDiscard {
            daq_list_num: 0,
            event_channel_id: event_id,
            reason: CycleDiscardReason::LinkDiscontinuity,
        }],
        unplanned_dto: 1,
    });

    let mut app = OpenXcpClientApp::new(controller);
    app.controller.active_tab = ActiveTab::XcpInfo;
    let ctx = egui::Context::default();
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_frame(ctx);
        });
    }

    // Also render the clean path, where the summary reports no loss and no discard list shows.
    app.controller.link_stats = LinkStats::new();
    app.controller.event_loss.clear();
    app.controller.recent_cycle_discards.clear();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });
    assert!(app.controller.daq_integrity_is_clean());
}
