// SPDX-License-Identifier: GPL-2.0-only
//! Comprehensive branch and edge-case tests across `config_store`, `a2l`, `xcp`,
//! `recording`, `calibration`, `mock_server`, `app_state`, and `ui` to guarantee >=95% line coverage.

use chrono::Utc;
use open_xcp_client::a2l::model::A2lModel;
use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::app_state::{ActiveTab, AppController, WorkerCommand, WorkerEvent};
use open_xcp_client::calibration::manager::CalibrationSyncStatus;
use open_xcp_client::config_store::{AppSettings, ConfigStore, TransportProtocol};
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::mock_server::MockXcpServer;
use open_xcp_client::recording::{SdfFileReader, SdfSessionRecorder};
use open_xcp_client::ui::OpenXcpClientApp;
use open_xcp_client::xcp::client::XcpClient;
use open_xcp_client::xcp::clock_sync::TargetClockSync;
use open_xcp_client::xcp::daq_planner::{DaqEventSample, DaqPlanner, DecodedValue};
use open_xcp_client::xcp::packet::cto;
use open_xcp_client::xcp::transport::XcpTransport;
use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_config_store_error_branches_and_display() {
    let dir = tempdir().unwrap();

    // 1. Missing file on load_from_path -> ConfigError::Io
    let store = ConfigStore::new(dir.path().join("sub/settings.json"));
    let io_err = store
        .load_from_path(&dir.path().join("does_not_exist.json"))
        .unwrap_err();
    assert!(!io_err.to_string().is_empty());
    assert!(io_err.source().is_some());

    // 2. Malformed JSON -> ConfigError::Json
    let bad_json_path = dir.path().join("bad.json");
    std::fs::write(&bad_json_path, "{ invalid json ").unwrap();
    let json_err = store.load_from_path(&bad_json_path).unwrap_err();
    assert!(!json_err.to_string().is_empty());
    assert!(json_err.source().is_some());

    // 3. Validation error Display & source()
    let mut bad_settings = AppSettings::default();
    bad_settings.recording.max_logfile_size_mb = 0;
    let val_err = store.save(&bad_settings).unwrap_err();
    assert!(val_err.to_string().contains("at least 1 MB"));
    assert!(val_err.source().is_none());
}

#[tokio::test]
async fn test_xcp_client_and_mock_server_extended_commands_and_fallbacks() {
    let server = MockXcpServer::start(0).await.unwrap();
    let timeout = Duration::from_millis(1000);

    // Connect low-level transport to test auxiliary mock server commands
    let (transport, _daq_rx) = XcpTransport::connect(server.udp_addr, TransportProtocol::Udp)
        .await
        .unwrap();

    let status_res = transport
        .send_command(&cto::get_status(), timeout)
        .await
        .unwrap();
    assert_eq!(status_res.len(), 5);

    let comm_res = transport
        .send_command(&cto::get_comm_mode_info(), timeout)
        .await
        .unwrap();
    assert!(!comm_res.is_empty());

    // GetId with mode 0 (IDT_ASCII) returns 0 length
    let id0_res = transport
        .send_command(&cto::get_id(0), timeout)
        .await
        .unwrap();
    assert!(!id0_res.is_empty());

    // SetCalPage and GetCalPage
    transport
        .send_command(&cto::set_cal_page(3, 0, 1), timeout)
        .await
        .unwrap();
    let cal_res = transport
        .send_command(&[0xEA, 0, 0], timeout)
        .await
        .unwrap();
    assert_eq!(cal_res.last().copied(), Some(1));

    // Unknown command returns SlaveError(0x20)
    let unknown_err = transport.send_command(&[0x01], timeout).await;
    assert!(unknown_err.is_err());

    // Malformed short commands exercise slave error responses (0xFE 0x22)
    for short_cmd in [
        vec![0xF6],
        vec![0xF4],
        vec![0xF0],
        vec![0xED],
        vec![0xD5],
        vec![0xD4],
        vec![0xD3],
        vec![0xE2],
        vec![0xE1],
        vec![0xE0],
        vec![0xDE],
    ] {
        assert!(transport.send_command(&short_cmd, timeout).await.is_err());
    }
    transport.close();

    // Now test XcpClient large read_memory (> max_short -> SET_MTA + UPLOAD) and empty DAQ planner
    let (client, _clock_sync, _rx) =
        XcpClient::connect(server.tcp_addr, TransportProtocol::Tcp, timeout, true)
            .await
            .unwrap();

    let large_read = client.read_memory(0x1000, 0, 128).await.unwrap();
    assert_eq!(large_read.len(), 128);

    // Empty DAQ planner should FreeDaq and return Ok(())
    let empty_model = A2lModel::default();
    let empty_planner = DaqPlanner::from_a2l(&empty_model, 64);
    client
        .configure_and_start_daq(&empty_planner)
        .await
        .unwrap();

    // Local A2L fallback file test
    let dir = tempdir().unwrap();
    let local_a2l = dir.path().join("fallback.a2l");
    std::fs::write(&local_a2l, sample_adas_a2l_text()).unwrap();
    let loaded = client
        .fetch_a2l(Some(local_a2l.to_str().unwrap()))
        .await
        .unwrap();
    assert!(loaded.contains("OpenXcpAdasProject"));

    client.disconnect().await.unwrap();
    server.stop();
}

#[test]
fn test_ui_all_states_and_interactions_headless() {
    let dir = tempdir().unwrap();
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let controller = AppController::new(store);
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();

    // 1. Render disconnected state on both tabs (triggers the "Not connected" info labels)
    let mut app = OpenXcpClientApp::new(controller);
    let ctx = egui::Context::default();

    app.controller.active_tab = ActiveTab::Numerical;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    app.controller.active_tab = ActiveTab::Calibration;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // 2. Render connected + active recording state + all CalibrationSyncStatus variants
    app.controller.connected = true;
    app.controller.is_recording = true;
    app.controller.active_recording_file = Some(PathBuf::from("./logfile/Measurement_PART001.sdf"));
    app.controller.current_part_bytes = 250_000_000;
    app.controller.max_part_bytes = 500_000_000;
    app.controller.a2l_model = Some(model.clone());

    for (idx, _) in model.measurements.iter().enumerate() {
        app.controller
            .latest_measurement_values
            .insert(idx, DecodedValue::Scalar(idx as f64 * 2.5));
    }

    app.controller
        .add_calibration_variable("Cal.Controller.kp_gain");
    app.controller
        .add_calibration_variable("Cal.Controller.max_steering_deg");
    app.controller
        .add_calibration_variable("Cal.Controller.max_tracked_objects");
    app.controller
        .add_calibration_variable("Cal.Controller.enable_aeb");
    app.controller
        .add_calibration_variable("Cal.Controller.operating_mode");

    // Set each entry to a distinct CalibrationSyncStatus variant so every status badge renders
    if let Some(e) = app
        .controller
        .calibration_manager
        .entries_mut()
        .get_mut("Cal.Controller.kp_gain")
    {
        e.status = CalibrationSyncStatus::Synced;
    }
    if let Some(e) = app
        .controller
        .calibration_manager
        .entries_mut()
        .get_mut("Cal.Controller.max_steering_deg")
    {
        e.status = CalibrationSyncStatus::Modified;
    }
    if let Some(e) = app
        .controller
        .calibration_manager
        .entries_mut()
        .get_mut("Cal.Controller.max_tracked_objects")
    {
        e.status = CalibrationSyncStatus::Sending;
    }
    if let Some(e) = app
        .controller
        .calibration_manager
        .entries_mut()
        .get_mut("Cal.Controller.enable_aeb")
    {
        e.status = CalibrationSyncStatus::PendingRead;
    }
    if let Some(e) = app
        .controller
        .calibration_manager
        .entries_mut()
        .get_mut("Cal.Controller.operating_mode")
    {
        e.status = CalibrationSyncStatus::Error("Out of bounds".to_string());
    }

    app.controller.active_tab = ActiveTab::Numerical;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    app.controller.active_tab = ActiveTab::Calibration;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Test invalid calibration submission error branch on controller
    app.controller
        .calibration_manager
        .update_input_buffer("Cal.Controller.kp_gain", "99999.0".to_string());
    assert!(app
        .controller
        .submit_calibration_on_enter("Cal.Controller.kp_gain")
        .is_err());
}

#[test]
fn test_sdf_reader_type_mismatch_and_daq_short_packet_edges() {
    let dir = tempdir().unwrap();
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let mut rec =
        SdfSessionRecorder::new(&model, dir.path(), "Measurement", Utc::now(), 50_000).unwrap();

    let mut values = BTreeMap::new();
    values.insert(0, DecodedValue::Scalar(1.0));
    let sample = DaqEventSample {
        daq_list_num: 0,
        event_channel_id: 0,
        event_name: "Event_10ms".to_string(),
        time_utc_s: 1_792_720_800.0,
        time_utc_ns: 1_792_720_800_000_000_000,
        values,
    };
    rec.record_event_sample(&sample).unwrap();
    assert!(rec.current_part_bytes() > 0);
    let files = rec.finish().unwrap();

    let reader = SdfFileReader::open(&files[0]).unwrap();
    // Calling read_i64_dataset on a float64 dataset returns None
    assert!(reader
        .read_i64_dataset("/Events/Event_10ms/time_utc_s")
        .is_none());
    // Calling read_f64_dataset on an int64 dataset returns None
    assert!(reader
        .read_f64_dataset("/Events/Event_10ms/time_utc_ns")
        .is_none());

    // Short DTO packet (<4 bytes when timestamp is expected) returns None
    let mut planner = DaqPlanner::from_a2l(&model, 128);
    let mut clock = TargetClockSync::new(true);
    assert!(planner
        .process_dto_packet(0, 0, &[1, 2], 0, &model, &mut clock)
        .is_none());
    assert!(planner
        .process_dto_packet(0, 0, &[0, 0, 0, 0, 1], 0, &model, &mut clock)
        .is_none());
}

#[tokio::test]
async fn test_app_controller_worker_all_events_and_error_paths() {
    let dir = tempdir().unwrap();
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let mut controller = AppController::new(store);
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();

    // 1. Start recording when not connected -> emits StatusMessage
    controller.toggle_recording();
    std::thread::sleep(Duration::from_millis(30));
    controller.poll_worker_events();
    assert!(controller
        .status_message
        .contains("Connect to an XCP target"));

    // 2. Connect with invalid IP address -> emits Disconnected
    controller.settings.ui.enable_mock_server = false;
    controller.settings.connection.target_ip = "not_an_ip_address".to_string();
    controller.request_connect();
    std::thread::sleep(Duration::from_millis(30));
    controller.poll_worker_events();
    assert!(controller.status_message.contains("Invalid target IP/port"));

    // 2b. Connect to closed TCP port -> emits XCP connection failed
    controller.settings.connection.target_ip = "127.0.0.1".to_string();
    controller.settings.connection.target_port = 1;
    controller.settings.connection.transport = TransportProtocol::Tcp;
    controller.settings.connection.connect_timeout_ms = 200;
    controller.request_connect();
    tokio::time::sleep(Duration::from_millis(80)).await;
    controller.poll_worker_events();
    assert!(controller.status_message.contains("XCP connection failed"));

    // 3. Connect to real MockXcpServer via explicit IP/port (enable_mock_server = false)
    let srv = MockXcpServer::start(0).await.unwrap();
    controller.settings.connection.target_ip = srv.tcp_addr.ip().to_string();
    controller.settings.connection.target_port = srv.tcp_addr.port();
    controller.settings.connection.transport = TransportProtocol::Tcp;
    let fallback_file = dir.path().join("local.a2l");
    std::fs::write(&fallback_file, sample_adas_a2l_text()).unwrap();
    controller.settings.connection.local_a2l_fallback_path = fallback_file.display().to_string();
    controller.request_connect();

    let wait = std::time::Instant::now();
    while !controller.connected && wait.elapsed() < Duration::from_secs(2) {
        controller.poll_worker_events();
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(controller.connected);

    // Send SetCalPage command through worker
    controller.send_worker_command(WorkerCommand::SetCalPage { page: 1 });
    // Attempt StartRecording on an invalid path (/dev/null/impossible) to exercise SdfSessionRecorder error branch
    controller.send_worker_command(WorkerCommand::StartRecording {
        folder: "/dev/null/impossible_dir".to_string(),
        prefix: "Measurement".to_string(),
        max_size_bytes: 500_000_000,
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    controller.poll_worker_events();
    assert!(controller
        .status_message
        .contains("Failed to initialize SDF recorder"));

    // Exercise all remaining WorkerEvent branches directly on handle_worker_event
    controller.add_calibration_variable("Cal.Controller.kp_gain");
    controller.handle_worker_event(WorkerEvent::RecordingPartSplit {
        completed_file: PathBuf::from("./logfile/Measurement_PART001.sdf"),
        new_active_file: PathBuf::from("./logfile/Measurement_PART002.sdf"),
        new_part_number: 2,
    });
    assert_eq!(controller.active_part_number, 2);

    controller.handle_worker_event(WorkerEvent::CharacteristicWriteFailed {
        name: "Cal.Controller.kp_gain".to_string(),
        error: "Target locked".to_string(),
    });
    assert!(controller.status_message.contains("Target locked"));

    controller.handle_worker_event(WorkerEvent::StatusMessage("Custom status".to_string()));
    assert_eq!(controller.status_message, "Custom status");

    // Exercise parser layout name inference for all scalar types and struct component overrides
    let layout_a2l = r#"
    /begin PROJECT P "P"
      /begin MODULE M "M"
        /begin RECORD_LAYOUT L_U8 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_I8 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_U16 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_I16 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_U32 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_I32 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_U64 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_I64 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_F32 /end RECORD_LAYOUT
        /begin RECORD_LAYOUT L_F64 /end RECORD_LAYOUT
        /begin TYPEDEF_MEASUREMENT TM_NoDesc "" ULONG CM_NONE 0 0 0 100 /end TYPEDEF_MEASUREMENT
        /begin TYPEDEF_CHARACTERISTIC TC_NoDesc "" VALUE L_F32 0 CM_NONE 0 100 /end TYPEDEF_CHARACTERISTIC
        /begin TYPEDEF_STRUCTURE TS_CompArr "StructDesc" 0x20
          /begin STRUCTURE_COMPONENT m_vec TM_NoDesc 0x00 MATRIX_DIM 2 /end STRUCTURE_COMPONENT
          /begin STRUCTURE_COMPONENT c_vec TC_NoDesc 0x08 MATRIX_DIM 2 /end STRUCTURE_COMPONENT
        /end TYPEDEF_STRUCTURE
        /begin INSTANCE InstOverride "InstDesc" TS_CompArr 0x6000 EVENT 99 /end INSTANCE
        /begin MEASUREMENT M_Arr "Arr" ULONG CM_NONE 0 0 0 100 ARRAY_SIZE 4 DEFAULT_EVENT_LIST 0 /end MEASUREMENT
      /end MODULE
    /end PROJECT
    "#;
    let parsed_layouts = parse_a2l(layout_a2l).unwrap();
    assert_eq!(parsed_layouts.record_layouts.len(), 10);
    assert_eq!(parsed_layouts.measurements[0].matrix_dim, vec![4]);
    assert_eq!(parsed_layouts.measurements[1].matrix_dim, vec![2]);
    assert_eq!(parsed_layouts.characteristics[0].matrix_dim, vec![2]);

    // Also test SdfSessionRecorder with an unmapped event ID (99) and missing trailer error
    let mut rec_unmapped =
        SdfSessionRecorder::new(&parsed_layouts, dir.path(), "Meas", Utc::now(), 50_000).unwrap();
    let mut vals = BTreeMap::new();
    vals.insert(1, DecodedValue::Array(vec![1.0, 2.0]));
    rec_unmapped
        .record_event_sample(&DaqEventSample {
            daq_list_num: 0,
            event_channel_id: 77,
            event_name: "DynamicEv".to_string(),
            time_utc_s: 1.0,
            time_utc_ns: 1_000_000_000,
            values: vals,
        })
        .unwrap();
    let unmapped_files = rec_unmapped.finish().unwrap();
    let mut raw_sdf = std::fs::read(&unmapped_files[0]).unwrap();
    let len = raw_sdf.len();
    raw_sdf[len - 1] ^= 0xFF; // corrupt trailer magic
    assert!(SdfFileReader::from_bytes(raw_sdf).is_err());

    // Start recording, then reconnect while recording is active to exercise cleanup on reconnect
    controller.log_folder_input = dir.path().display().to_string();
    controller.toggle_recording();
    tokio::time::sleep(Duration::from_millis(40)).await;
    controller.poll_worker_events();
    controller.request_connect();
    tokio::time::sleep(Duration::from_millis(60)).await;
    controller.poll_worker_events();

    // Start recording again and disconnect while recording is active
    controller.toggle_recording();
    tokio::time::sleep(Duration::from_millis(40)).await;
    controller.request_disconnect();
    tokio::time::sleep(Duration::from_millis(40)).await;
    controller.poll_worker_events();
    let _ = model;
    srv.stop();
}

#[tokio::test]
async fn test_remaining_config_client_validator_and_worker_branches() {
    let dir = tempdir().unwrap();

    // 1. ConfigStore & AppSettings edge cases
    assert!(ConfigStore::default_path().ends_with("settings.json"));
    let store = ConfigStore::new(dir.path().join("cfg.json"));
    assert!(store
        .load_from_path(std::path::Path::new("/nonexistent/does_not_exist.json"))
        .is_err());
    let bad_json = dir.path().join("bad.json");
    std::fs::write(&bad_json, "{ invalid json ").unwrap();
    assert!(store.load_from_path(&bad_json).is_err());

    let mut bad_settings = AppSettings::default();
    bad_settings.recording.default_log_folder = "   ".to_string();
    assert!(bad_settings.validate().is_err());
    let mut bad_settings2 = AppSettings::default();
    bad_settings2.recording.default_file_prefix = "".to_string();
    assert!(bad_settings2.validate().is_err());

    // 2. Calibration validator edge cases (integer enum key out of range, NaN/inf, OutOfRawTypeBounds, short raw decode)
    use open_xcp_client::calibration::validator::{
        decode_characteristic_to_display, validate_and_encode_characteristic,
    };
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let enum_ch = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.operating_mode")
        .unwrap();
    assert!(validate_and_encode_characteristic(enum_ch, &model, "99").is_err());

    let float_ch = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.kp_gain")
        .unwrap();
    assert!(validate_and_encode_characteristic(float_ch, &model, "inf").is_err());
    assert_eq!(decode_characteristic_to_display(float_ch, &model, &[]), "0");

    let mut overflow_ch = enum_ch.clone();
    overflow_ch.compu_method = "CM_IDENT".to_string();
    overflow_ch.upper_limit = 1000.0;
    assert!(validate_and_encode_characteristic(&overflow_ch, &model, "500").is_err());

    // 3. XcpClient large read_memory (> max_short) and large write_memory (> max_short_data via SET_MTA + DOWNLOAD)
    let srv = MockXcpServer::start(0).await.unwrap();
    let (client, _clock, _daq_rx) = XcpClient::connect(
        srv.udp_addr,
        TransportProtocol::Udp,
        Duration::from_millis(500),
        true,
    )
    .await
    .unwrap();

    // Nonexistent local fallback falls back to XCP GET_ID + UPLOAD
    let uploaded = client
        .fetch_a2l(Some("/nonexistent/fallback.a2l"))
        .await
        .unwrap();
    assert!(uploaded.contains("OpenXcpAdasProject"));

    // Empty DAQ planner
    let empty_planner = DaqPlanner::from_a2l(&open_xcp_client::a2l::model::A2lModel::default(), 64);
    client
        .configure_and_start_daq(&empty_planner)
        .await
        .unwrap();

    // Large write (128 bytes > max_short_data) + readback verification (128 bytes > max_short)
    let large_payload: Vec<u8> = (0..128u8).collect();
    client
        .write_memory(0x2200, 0, &large_payload, true)
        .await
        .unwrap();
    let large_read = client.read_memory(0x2200, 0, 128).await.unwrap();
    assert_eq!(large_read, large_payload);
    client.disconnect().await.unwrap();
    srv.stop();

    // 4. AppController offline calibration & mock TCP / bad A2L fallback worker branches
    let mut ctrl = AppController::new(store);
    // Calling submit_calibration_on_enter before A2L is loaded
    assert!(ctrl
        .submit_calibration_on_enter("Cal.Controller.kp_gain")
        .is_err());

    // Load A2L while disconnected and submit calibration offline
    ctrl.a2l_model = Some(model.clone());
    ctrl.add_calibration_variable("Cal.Controller.kp_gain");
    ctrl.calibration_manager
        .update_input_buffer("Cal.Controller.kp_gain", "2.25".to_string());
    ctrl.submit_calibration_on_enter("Cal.Controller.kp_gain")
        .unwrap();
    assert!(ctrl.status_message.contains("Sent calibration update"));

    // Connect with built-in mock server over TCP + broken local A2L fallback -> triggers A2L parse error
    let broken_a2l = dir.path().join("broken.a2l");
    std::fs::write(&broken_a2l, "/begin MEASUREMENT Unclosed").unwrap();
    ctrl.settings.ui.enable_mock_server = true;
    ctrl.settings.connection.transport = TransportProtocol::Tcp;
    ctrl.settings.connection.local_a2l_fallback_path = broken_a2l.display().to_string();
    ctrl.request_connect();
    tokio::time::sleep(Duration::from_millis(80)).await;
    ctrl.poll_worker_events();
    assert!(ctrl.status_message.contains("A2L parse error"));

    // 5. A2L parser short/malformed block guards, event time units, and typedef/instance keyword branches
    assert!(parse_a2l("/begin").is_err());
    let edge_a2l = r#"
    /begin PROJECT P "P"
      /begin MODULE M "M"
        /begin EVENT E_Short /end EVENT
        /begin EVENT E_U3 "E3" 1 DAQ 1 5 3 0 /end EVENT
        /begin EVENT E_U4 "E4" 2 DAQ 1 5 4 0 /end EVENT
        /begin EVENT E_U5 "E5" 3 DAQ 1 5 5 0 /end EVENT
        /begin EVENT E_U7 "E7" 4 DAQ 1 5 7 0 /end EVENT
        /begin EVENT E_U8 "E8" 5 DAQ 1 5 8 0 /end EVENT
        /begin EVENT E_U9 "E9" 6 DAQ 1 5 9 0 /end EVENT
        /begin EVENT E_UX "EX" 7 DAQ 1 5 99 0 /end EVENT
        /begin COMPU_METHOD CM_Short /end COMPU_METHOD
        /begin COMPU_VTAB V_Short /end COMPU_VTAB
        /begin RECORD_LAYOUT /end RECORD_LAYOUT
        /begin RECORD_LAYOUT RL_CustomUnknown /end RECORD_LAYOUT
        /begin MEASUREMENT M_Short /end MEASUREMENT
        /begin CHARACTERISTIC C_Short /end CHARACTERISTIC
        /begin TYPEDEF_MEASUREMENT TM_Short /end TYPEDEF_MEASUREMENT
        /begin TYPEDEF_CHARACTERISTIC TC_Short /end TYPEDEF_CHARACTERISTIC
        /begin TYPEDEF_STRUCTURE TS_Short /end TYPEDEF_STRUCTURE
        /begin INSTANCE I_Short /end INSTANCE
        /begin TYPEDEF_MEASUREMENT TM_Full "Desc" FLOAT32_IEEE CM_NONE 0 0 -10 10 PHYS_UNIT "m/s" MATRIX_DIM 3 EXTRA_KW /end TYPEDEF_MEASUREMENT
        /begin TYPEDEF_CHARACTERISTIC TC_Full "Desc" VALUE RL_CustomUnknown 0 CM_NONE -10 10 PHYS_UNIT "deg" MATRIX_DIM 2 EXTRA_KW /end TYPEDEF_CHARACTERISTIC
        /begin TYPEDEF_STRUCTURE TS_Full "Desc" 0x20
          /begin STRUCTURE_COMPONENT m TM_Full 0x00 MATRIX_DIM 2 EXTRA_KW /end STRUCTURE_COMPONENT
          /begin STRUCTURE_COMPONENT c TC_Full 0x10 /end STRUCTURE_COMPONENT
        /end TYPEDEF_STRUCTURE
        /begin INSTANCE I_Full "Desc" TS_Full 0x7000 ECU_ADDRESS_EXTENSION 2 MATRIX_DIM 2 EVENT 1 EVENT BAD_EV EXTRA_KW /end INSTANCE
        /begin MEASUREMENT M_BadEv "Desc" UBYTE CM_NONE 0 0 0 10 ECU_ADDRESS 0x7100 ECU_ADDRESS_EXTENSION 1 PHYS_UNIT "s" MATRIX_DIM 2 EVENT BAD_EV /end MEASUREMENT
      /end MODULE
    /end PROJECT
    "#;
    let edge_model = parse_a2l(edge_a2l).unwrap();
    assert_eq!(edge_model.events.len(), 7);
    assert!(!edge_model.measurements.is_empty());
    assert!(!edge_model.characteristics.is_empty());
}
