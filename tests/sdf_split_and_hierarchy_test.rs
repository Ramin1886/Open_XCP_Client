// SPDX-License-Identifier: GPL-2.0-only
//! Unit and integration tests for `.sdf` (HDF5 v2) recording, hierarchical C/C++ structure
//! preservation, UTC time-series datasets, filename formatting, and configurable file-size splitting.

use chrono::{TimeZone, Utc};
use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::recording::{
    format_utc_for_filename, jenkins_lookup3_checksum, RecordingFileNamer, SdfFileReader,
    SdfSessionRecorder, SizeSplitter, HDF5_SIGNATURE,
};
use open_xcp_client::xcp::daq_planner::{DaqEventSample, DecodedValue};
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn test_recording_file_namer_preserves_start_timestamp_across_parts() {
    let dt = Utc.with_ymd_and_hms(2026, 10, 23, 1, 30, 45).unwrap();
    assert_eq!(format_utc_for_filename(dt), "2026-10-23_01-30-45Z");

    let mut namer = RecordingFileNamer::new("./logfile", "Measurement", dt);
    assert_eq!(namer.current_part(), 1);
    assert_eq!(
        namer.filename_for_part(1),
        "Measurement_2026-10-23_01-30-45Z_PART001.sdf"
    );
    let p2 = namer.advance_part();
    assert_eq!(namer.current_part(), 2);
    assert!(p2
        .to_string_lossy()
        .ends_with("Measurement_2026-10-23_01-30-45Z_PART002.sdf"));
}

#[test]
fn test_size_splitter_default_500mb_and_custom_thresholds() {
    let splitter_500 = SizeSplitter::from_max_bytes(500_000_000, 8192);
    assert_eq!(splitter_500.max_file_size_bytes(), 500_000_000);
    assert!(!splitter_500.should_split_before_adding(1024));

    let mut small_splitter = SizeSplitter::from_max_bytes(10_000, 2_000);
    // First sample never splits an empty file
    assert!(!small_splitter.should_split_before_adding(5_000));
    small_splitter.record_bytes(5_000);
    assert_eq!(small_splitter.current_payload_bytes(), 5_000);
    assert!(small_splitter.fill_ratio() > 0.6);

    // Adding 4_000 more (2_000 overhead + 5_000 + 4_000 = 11_000 > 10_000) triggers split!
    assert!(small_splitter.should_split_before_adding(4_000));
    small_splitter.reset_for_new_part();
    assert_eq!(small_splitter.current_payload_bytes(), 0);
}

#[test]
fn test_sdf_recorder_hierarchy_utc_timeseries_and_part_splitting() {
    let dir = tempdir().unwrap();
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let start_dt = Utc.with_ymd_and_hms(2026, 10, 23, 2, 0, 0).unwrap();

    // Configure a tight max_file_size_bytes (12 KB) to verify automatic part splitting into PART001, PART002, ...
    let max_bytes = 12_000u64;
    let mut recorder =
        SdfSessionRecorder::new(&model, dir.path(), "Measurement", start_dt, max_bytes).unwrap();

    for i in 0..80 {
        let mut values = BTreeMap::new();
        for (m_idx, m) in model.measurements.iter().enumerate() {
            if m.element_count() == 1 {
                values.insert(m_idx, DecodedValue::Scalar(i as f64 * 1.5));
            } else {
                values.insert(
                    m_idx,
                    DecodedValue::Array(vec![i as f64; m.element_count()]),
                );
            }
        }
        let sample = DaqEventSample {
            daq_list_num: 0,
            event_channel_id: 0,
            event_name: "Event_10ms".to_string(),
            time_utc_s: 1_792_720_800.0 + (i as f64 * 0.01),
            time_utc_ns: 1_792_720_800_000_000_000 + (i as i64 * 10_000_000),
            values,
        };
        recorder.record_event_sample(&sample).unwrap();
    }

    let files = recorder.finish().unwrap();
    assert!(
        files.len() >= 2,
        "Expected recording to split into at least 2 parts, got {:?}",
        files
    );

    for (idx, path) in files.iter().enumerate() {
        let part_num = (idx + 1) as u32;
        let expected_suffix = format!("Measurement_2026-10-23_02-00-00Z_PART{:03}.sdf", part_num);
        assert!(path.to_string_lossy().ends_with(&expected_suffix));

        let on_disk_len = std::fs::metadata(path).unwrap().len();
        assert!(
            on_disk_len <= max_bytes,
            "Part file {} size {} exceeded limit {}",
            path.display(),
            on_disk_len,
            max_bytes
        );

        let reader = SdfFileReader::open(path).expect("Should open valid HDF5 .sdf file");
        assert_eq!(reader.metadata.part_number, part_num);
        assert_eq!(reader.metadata.project_name, "OpenXcpAdasProject");

        // Verify Event UTC timestamp series
        let time_s = reader
            .read_f64_dataset("/Events/Event_10ms/time_utc_s")
            .expect("Should read time_utc_s dataset");
        let time_ns = reader
            .read_i64_dataset("/Events/Event_10ms/time_utc_ns")
            .expect("Should read time_utc_ns dataset");
        assert!(!time_s.is_empty());
        assert_eq!(time_s.len(), time_ns.len());

        // Verify nested C/C++ structure path preservation
        let radar_dist = reader
            .read_f64_dataset("/Measurements/Perception/RadarFront/targets_0/distance_m")
            .expect("Should preserve nested struct hierarchy in HDF5 path");
        assert_eq!(radar_dist.len(), time_s.len());

        // Verify multidimensional array shape preservation
        let cov_ds = reader
            .metadata
            .datasets
            .get("/Measurements/EgoMotion/Pose/covariance_diag")
            .expect("Should contain covariance_diag array dataset");
        assert_eq!(cov_ds.shape, vec![time_s.len(), 3]);
    }

    // Verify invalid HDF5 buffer rejection
    assert!(SdfFileReader::from_bytes(vec![0u8; 10]).is_err());
    let mut bad_sig = vec![0u8; 128];
    bad_sig[0..8].copy_from_slice(&HDF5_SIGNATURE);
    bad_sig[8] = 2;
    assert!(SdfFileReader::from_bytes(bad_sig).is_err());
    let mut bad_ver = vec![0u8; 128];
    bad_ver[0..8].copy_from_slice(&HDF5_SIGNATURE);
    bad_ver[8] = 1;
    assert!(SdfFileReader::from_bytes(bad_ver).is_err());
    assert_ne!(
        jenkins_lookup3_checksum(b"test1"),
        jenkins_lookup3_checksum(b"test2")
    );

    // Verify recorder telemetry accessors and sanitize_prefix edge cases
    assert!(recorder.current_part() >= 1);
    assert!(recorder.max_part_bytes() >= 8192);
    assert!(recorder.fill_ratio() >= 0.0);
    assert!(recorder.total_samples_recorded() == 80);
    assert!(!recorder.completed_files().is_empty());
    let namer_empty = RecordingFileNamer::new("./logfile", "   ///   ", start_dt);
    assert!(namer_empty.filename_for_part(1).starts_with("Measurement_"));
    assert!(!namer_empty.start_utc_human().is_empty());
}

#[test]
fn test_sdf_reader_rejects_index_offsets_that_overflow_or_exceed_the_file() {
    let dir = tempdir().unwrap();
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let start = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
    let mut recorder = SdfSessionRecorder::new(&model, dir.path(), "Hostile", start, 50_000_000)
        .expect("recorder");
    let mut values = BTreeMap::new();
    values.insert(0usize, DecodedValue::Scalar(1.0));
    recorder
        .record_event_sample(&DaqEventSample {
            daq_list_num: 0,
            event_channel_id: 0,
            event_name: "Event_10ms".to_string(),
            time_utc_s: 1_767_000_000.0,
            time_utc_ns: 1_767_000_000_000_000_000,
            values,
        })
        .unwrap();
    let files = recorder.finish().unwrap();
    let mut reader = SdfFileReader::open(&files[0]).unwrap();
    let (path, summary) = reader
        .metadata
        .datasets
        .iter()
        .find(|(_, d)| d.dtype == "float64")
        .map(|(p, d)| (p.clone(), d.clone()))
        .expect("a float64 dataset");
    assert!(
        reader.read_f64_dataset(&path).is_some(),
        "sane offsets read fine"
    );

    // Offset + length wraps around usize: must be None, never a panic.
    let mut poisoned = summary.clone();
    poisoned.data_byte_offset = u64::MAX - 4;
    reader.metadata.datasets.insert(path.clone(), poisoned);
    assert!(reader.read_f64_dataset(&path).is_none());

    // Length past the end of the file: also None.
    let mut too_long = summary;
    too_long.data_byte_len = u64::MAX / 2;
    reader.metadata.datasets.insert(path.clone(), too_long);
    assert!(reader.read_f64_dataset(&path).is_none());
}
