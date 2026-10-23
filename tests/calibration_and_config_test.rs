// SPDX-License-Identifier: GPL-2.0-only
//! Unit tests for JSON configuration persistence (`ConfigStore`, including configurable
//! `max_logfile_size_mb` defaulting to 500 MB) and calibration data-type & limit validation.

use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::calibration::{
    decode_characteristic_to_display, validate_and_encode_characteristic, CalibrationManager,
    CalibrationSyncStatus, CalibrationValidationError,
};
use open_xcp_client::config_store::{ConfigStore, TransportProtocol, DEFAULT_MAX_LOGFILE_SIZE_MB};
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use tempfile::tempdir;

#[test]
fn test_config_store_load_save_and_split_size_parameter() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("settings.json");
    let store = ConfigStore::new(&cfg_path);

    let mut settings = store.load_or_default().unwrap();
    assert_eq!(
        settings.recording.max_logfile_size_mb,
        DEFAULT_MAX_LOGFILE_SIZE_MB
    );
    assert_eq!(settings.max_logfile_size_bytes(), 500_000_000);
    assert_eq!(settings.connection.transport.as_str(), "UDP");

    // Customize logfile split size to 250 MB and protocol to TCP, then save and reload
    settings.recording.max_logfile_size_mb = 250;
    settings.connection.transport = TransportProtocol::Tcp;
    settings.connection.target_ip = "192.168.10.42".to_string();
    store.save(&settings).unwrap();

    let reloaded = store.load_or_default().unwrap();
    assert_eq!(reloaded.recording.max_logfile_size_mb, 250);
    assert_eq!(reloaded.max_logfile_size_bytes(), 250_000_000);
    assert_eq!(reloaded.connection.transport, TransportProtocol::Tcp);
    assert_eq!(reloaded.connection.target_ip, "192.168.10.42");

    // Validation errors
    let mut invalid = reloaded.clone();
    invalid.recording.max_logfile_size_mb = 0;
    assert!(invalid.validate().is_err());

    let mut empty_ip = reloaded.clone();
    empty_ip.connection.target_ip = "   ".to_string();
    assert!(empty_ip.validate().is_err());

    let mut zero_port = reloaded.clone();
    zero_port.connection.target_port = 0;
    assert!(zero_port.validate().is_err());

    let mut empty_folder = reloaded.clone();
    empty_folder.recording.default_log_folder = "".to_string();
    assert!(empty_folder.validate().is_err());

    let mut empty_prefix = reloaded.clone();
    empty_prefix.recording.default_file_prefix = "".to_string();
    assert!(empty_prefix.validate().is_err());

    assert!(ConfigStore::default_path().ends_with("settings.json"));
    let default_store = ConfigStore::new(dir.path().join("nonexistent_settings.json"));
    assert!(default_store.load_or_default().is_ok());
}

#[test]
fn test_calibration_validator_and_manager_all_types() {
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();

    // 1. Float32 scalar parameter (Cal.Controller.kp_gain: [0.0 .. 50.0])
    let kp = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.kp_gain")
        .unwrap();
    let bytes = validate_and_encode_characteristic(kp, &model, "12.5").unwrap();
    assert_eq!(decode_characteristic_to_display(kp, &model, &bytes), "12.5");
    assert!(matches!(
        validate_and_encode_characteristic(kp, &model, "99.0"),
        Err(CalibrationValidationError::OutOfPhysicalRange { .. })
    ));
    assert!(matches!(
        validate_and_encode_characteristic(kp, &model, "not_a_float"),
        Err(CalibrationValidationError::InvalidTypeSyntax { .. })
    ));

    // 2. Integer UWORD scalar parameter (Cal.Controller.max_tracked_objects: [1 .. 128])
    let max_obj = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.max_tracked_objects")
        .unwrap();
    let bytes_u16 = validate_and_encode_characteristic(max_obj, &model, "64").unwrap();
    assert_eq!(
        decode_characteristic_to_display(max_obj, &model, &bytes_u16),
        "64"
    );
    // Reject float string for strict integer parameter
    assert!(matches!(
        validate_and_encode_characteristic(max_obj, &model, "12.34"),
        Err(CalibrationValidationError::InvalidTypeSyntax { .. })
    ));

    // 3. Boolean UBYTE parameter (Cal.Controller.enable_aeb: [0 .. 1])
    let aeb = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.enable_aeb")
        .unwrap();
    assert_eq!(
        validate_and_encode_characteristic(aeb, &model, "true").unwrap(),
        vec![1]
    );
    assert_eq!(
        validate_and_encode_characteristic(aeb, &model, "false").unwrap(),
        vec![0]
    );

    // 4. Enum TAB_VERB parameter (Cal.Controller.operating_mode: OFF=0, STANDBY=1, ACTIVE_LKA=2, ACTIVE_ACC=3)
    let mode = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.operating_mode")
        .unwrap();
    let enc_enum = validate_and_encode_characteristic(mode, &model, "ACTIVE_ACC").unwrap();
    assert_eq!(enc_enum, vec![3]);
    assert_eq!(
        decode_characteristic_to_display(mode, &model, &enc_enum),
        "ACTIVE_ACC"
    );
    assert!(matches!(
        validate_and_encode_characteristic(mode, &model, "INVALID_MODE"),
        Err(CalibrationValidationError::InvalidEnumVariant { .. })
    ));

    // 5. 1D Array VAL_BLK parameter (Cal.Controller.speed_lookup_curve: 4 floats in [0.0 .. 10.0])
    let curve = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.speed_lookup_curve")
        .unwrap();
    let enc_arr =
        validate_and_encode_characteristic(curve, &model, "[1.25, 2.5, 3.75, 5.0]").unwrap();
    assert_eq!(enc_arr.len(), 16);
    assert_eq!(
        decode_characteristic_to_display(curve, &model, &enc_arr),
        "[1.25, 2.5, 3.75, 5]"
    );
    assert!(matches!(
        validate_and_encode_characteristic(curve, &model, "[1.0, 2.0]"),
        Err(CalibrationValidationError::ArrayLengthMismatch { .. })
    ));

    // 6. CalibrationManager lifecycle
    let mut mgr = CalibrationManager::new();
    assert!(mgr.add_by_name("Cal.Controller.kp_gain", &model));
    assert!(!mgr.add_by_name("Cal.Controller.kp_gain", &model));
    mgr.update_input_buffer("Cal.Controller.kp_gain", "15.5".to_string());
    assert_eq!(
        mgr.entries()["Cal.Controller.kp_gain"].status,
        CalibrationSyncStatus::Modified
    );
    let (_, raw) = mgr
        .prepare_write_on_enter("Cal.Controller.kp_gain", &model)
        .unwrap();
    mgr.mark_synced_with_raw("Cal.Controller.kp_gain", &model, &raw);
    assert_eq!(
        mgr.entries()["Cal.Controller.kp_gain"].target_value_display,
        "15.5"
    );
    assert!(mgr.remove("Cal.Controller.kp_gain"));
    assert!(!mgr.remove("NonExistent"));
    assert!(!mgr.add_by_name("NonExistent", &model));
    mgr.add_by_name("Cal.Controller.speed_lookup_curve", &model);
    if let Some(entry) = mgr
        .entries_mut()
        .get_mut("Cal.Controller.speed_lookup_curve")
    {
        entry.element_buffers[0] = "3.5".to_string();
        entry.sync_input_from_elements();
    }
    mgr.mark_error(
        "Cal.Controller.speed_lookup_curve",
        "Simulated failure".to_string(),
    );
    mgr.clear();
    assert!(mgr.entries().is_empty());

    // Hex literal and empty input validation checks
    assert_eq!(
        validate_and_encode_characteristic(max_obj, &model, "0x10").unwrap(),
        16u16.to_le_bytes().to_vec()
    );
    assert_eq!(
        validate_and_encode_characteristic(max_obj, &model, "   "),
        Err(CalibrationValidationError::EmptyInput)
    );
    assert_eq!(
        validate_and_encode_characteristic(mode, &model, "2").unwrap(),
        vec![2]
    );
    assert!(matches!(
        validate_and_encode_characteristic(mode, &model, "99"),
        Err(CalibrationValidationError::InvalidEnumVariant { .. })
    ));
}
