// SPDX-License-Identifier: GPL-2.0-only
//! End-to-End integration tests against `MockXcpServer` over both UDP and TCP:
//! - Automatic A2L upload via `GET_ID (IDT_ASAM_UPLOAD = 4)` + `UPLOAD`
//! - Multi-rate DAQ measurement streaming into `.sdf` (HDF5 v2) files with part splitting
//! - Concurrent live calibration reads (`SHORT_UPLOAD`) and writes (`SHORT_DOWNLOAD` / `DOWNLOAD`)
//!   with readback verification while DAQ recording is active.

use chrono::Utc;
use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::app_state::AppController;
use open_xcp_client::calibration::{
    decode_characteristic_to_display, validate_and_encode_characteristic,
};
use open_xcp_client::config_store::{ConfigStore, TransportProtocol};
use open_xcp_client::mock_server::MockXcpServer;
use open_xcp_client::recording::{SdfFileReader, SdfSessionRecorder};
use open_xcp_client::xcp::client::XcpClient;
use open_xcp_client::xcp::daq_planner::DaqPlanner;
use open_xcp_client::xcp::daq_planner::DTO_HEADER_RESERVE_BYTES;
use std::time::Duration;
use tempfile::tempdir;

async fn run_full_protocol_flow(protocol: TransportProtocol) {
    let server = MockXcpServer::start(0).await.unwrap();
    let target_addr = match protocol {
        TransportProtocol::Udp => server.udp_addr,
        TransportProtocol::Tcp => server.tcp_addr,
    };

    let (client, mut clock_sync, mut daq_rx) =
        XcpClient::connect(target_addr, protocol, Duration::from_secs(2), true)
            .await
            .expect("Should connect to mock XCPlite 2.2.2 server");

    // 1. Fetch A2L dynamically from target via XCP GET_ID + UPLOAD
    let a2l_text = client
        .fetch_a2l(None)
        .await
        .expect("Should upload A2L from target");
    let model = parse_a2l(&a2l_text).expect("Uploaded A2L should parse cleanly");
    assert!(model.measurements.len() >= 10);
    assert!(model.characteristics.len() >= 6);

    // 2. Start DAQ streaming for all measurements grouped by Event refresh rate
    let mut planner = DaqPlanner::from_a2l(
        &model,
        client.max_dto().saturating_sub(DTO_HEADER_RESERVE_BYTES),
    );
    client
        .configure_and_start_daq(&planner)
        .await
        .expect("Should configure and start DAQ lists");

    let dir = tempdir().unwrap();
    let mut recorder =
        SdfSessionRecorder::new(&model, dir.path(), "Measurement", Utc::now(), 16_000).unwrap();

    // 3. Concurrently perform calibration reads & writes WHILE receiving DAQ packets!
    let kp_char = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.kp_gain")
        .unwrap();
    let initial_kp_bytes = client.read_characteristic(kp_char).await.unwrap();
    let initial_kp_str = decode_characteristic_to_display(kp_char, &model, &initial_kp_bytes);
    assert_eq!(initial_kp_str, "2.5");

    // Write new kp_gain = 14.75 and verify readback
    let new_kp_bytes = validate_and_encode_characteristic(kp_char, &model, "14.75").unwrap();
    client
        .write_memory(
            kp_char.address,
            kp_char.address_extension,
            &new_kp_bytes,
            true,
        )
        .await
        .expect("Concurrent SHORT_DOWNLOAD + readback should succeed during DAQ");

    // Also test multi-element array write (16 bytes -> SET_MTA + DOWNLOAD)
    let curve_char = model
        .characteristics
        .iter()
        .find(|c| c.name == "Cal.Controller.speed_lookup_curve")
        .unwrap();
    let new_curve_bytes =
        validate_and_encode_characteristic(curve_char, &model, "[2.0, 4.0, 6.0, 8.0]").unwrap();
    client
        .write_memory(
            curve_char.address,
            curve_char.address_extension,
            &new_curve_bytes,
            true,
        )
        .await
        .expect("Concurrent block DOWNLOAD + readback should succeed during DAQ");

    client.set_calibration_page(0, 1).await.unwrap();

    // 4. Collect DAQ samples into the SDF recorder
    let mut collected_samples = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while collected_samples < 6 && tokio::time::Instant::now() < deadline {
        if let Ok(Some(dto)) = tokio::time::timeout(Duration::from_millis(100), daq_rx.recv()).await
        {
            if let Some(sample) = planner.process_dto_packet(
                dto.odt_num,
                dto.daq_list_num,
                &dto.payload,
                0,
                &model,
                &mut clock_sync,
            ) {
                recorder.record_event_sample(&sample).unwrap();
                collected_samples += 1;
            }
        }
    }
    assert!(
        collected_samples >= 4,
        "Expected at least 4 decoded DAQ event samples over {:?}, got {}",
        protocol,
        collected_samples
    );

    client.stop_daq().await.unwrap();
    client.disconnect().await.unwrap();
    server.stop();

    let sdf_files = recorder.finish().unwrap();
    assert!(!sdf_files.is_empty());
    let reader = SdfFileReader::open(&sdf_files[0]).unwrap();
    let dist_series = reader
        .read_f64_dataset("/Measurements/Perception/RadarFront/targets_0/distance_m")
        .unwrap();
    assert!(!dist_series.is_empty());
}

#[tokio::test]
async fn test_end_to_end_udp_and_tcp_xcp_flows() {
    run_full_protocol_flow(TransportProtocol::Udp).await;
    run_full_protocol_flow(TransportProtocol::Tcp).await;
}

#[test]
fn test_app_controller_background_worker_lifecycle() {
    let dir = tempdir().unwrap();
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let mut controller = AppController::new(store);
    controller.settings.ui.enable_mock_server = true;
    controller.log_folder_input = dir.path().display().to_string();
    controller.log_prefix_input = "Measurement".to_string();

    controller.request_connect();
    let start = std::time::Instant::now();
    while !controller.connected && start.elapsed() < Duration::from_secs(3) {
        controller.poll_worker_events();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(
        controller.connected,
        "Controller should connect to mock server"
    );
    assert!(controller.a2l_model.is_some());

    // Add calibration variable and start recording concurrently
    controller.add_calibration_variable("Cal.Controller.kp_gain");
    controller.toggle_recording();

    let rec_wait = std::time::Instant::now();
    while controller.total_samples_recorded < 3 && rec_wait.elapsed() < Duration::from_secs(3) {
        controller.poll_worker_events();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(controller.is_recording);
    assert!(controller.total_samples_recorded >= 3);

    // Modify calibration variable while recording is active
    controller
        .calibration_manager
        .update_input_buffer("Cal.Controller.kp_gain", "9.5".to_string());
    controller
        .submit_calibration_on_enter("Cal.Controller.kp_gain")
        .unwrap();

    // Stop recording and disconnect
    controller.toggle_recording();
    let stop_wait = std::time::Instant::now();
    while controller.is_recording && stop_wait.elapsed() < Duration::from_secs(2) {
        controller.poll_worker_events();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(!controller.is_recording);
    assert!(!controller.completed_files.is_empty());

    controller.request_disconnect();
    let disc_wait = std::time::Instant::now();
    while controller.connected && disc_wait.elapsed() < Duration::from_secs(2) {
        controller.poll_worker_events();
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(!controller.connected);
}
