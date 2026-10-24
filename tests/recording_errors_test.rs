// SPDX-License-Identifier: GPL-2.0-only
//! Recording I/O failures must stop the recording visibly instead of leaving the UI on "REC"
//! while samples vanish, and must never take the live DAQ link down with them.

mod common;

use common::wait_until;
use open_xcp_client::app_state::{AppController, WorkerCommand};
use open_xcp_client::config_store::ConfigStore;
use open_xcp_client::ui::OpenXcpClientApp;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

fn connected_controller(dir: &Path, rec_dir: &Path) -> AppController {
    let store = ConfigStore::new(dir.join("settings.json"));
    let mut controller = AppController::new(store);
    controller.settings.ui.enable_mock_server = true;
    controller.settings.recording.record_video_mp4 = false;
    controller.log_folder_input = rec_dir.display().to_string();
    controller.log_prefix_input = "Fail".to_string();
    controller.request_connect();
    assert!(
        wait_until(&mut controller, Duration::from_secs(3), |c| c.connected),
        "mock target must connect"
    );
    controller
}

/// Replaces the recording folder by a plain file so every later directory creation and part
/// flush fails, on every platform, without touching permissions.
fn sabotage_folder(rec_dir: &Path) {
    std::fs::remove_dir_all(rec_dir).expect("recording folder existed");
    std::fs::write(rec_dir, b"not a directory").unwrap();
}

#[test]
fn sdf_rotation_failure_stops_recording_and_reports_error_while_daq_keeps_running() {
    let dir = tempdir().unwrap();
    let rec_dir = dir.path().join("rec");
    let mut controller = connected_controller(dir.path(), &rec_dir);

    // A tiny split size forces a part rotation (the first disk write) within a few samples.
    controller.send_worker_command(WorkerCommand::StartRecording {
        folder: rec_dir.display().to_string(),
        prefix: "Fail".to_string(),
        max_size_bytes: 20_000,
    });
    assert!(wait_until(&mut controller, Duration::from_secs(3), |c| c.is_recording));
    assert!(wait_until(&mut controller, Duration::from_secs(3), |c| {
        c.total_samples_recorded >= 3
    }));

    sabotage_folder(&rec_dir);

    assert!(
        wait_until(&mut controller, Duration::from_secs(5), |c| !c.is_recording),
        "recording must stop once the disk fails; status: {}",
        controller.status_message
    );
    let err = controller
        .last_recording_error
        .clone()
        .expect("the failure must be remembered for the UI");
    assert!(err.contains("sdf part rotation"), "{err}");
    assert!(controller.status_message.contains("Recording stopped"));
    assert!(
        controller.connected,
        "a disk failure must not drop the XCP link"
    );

    // DAQ keeps flowing: the latest live timestamp keeps advancing after the failure.
    let before = controller.latest_sample_utc_s;
    assert!(wait_until(&mut controller, Duration::from_secs(3), |c| {
        c.latest_sample_utc_s > before
    }));

    // The top bar renders the error while idle, and a fresh start clears it.
    let mut app = OpenXcpClientApp::new(controller);
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.render_frame(ctx));
    let good_dir = dir.path().join("good");
    app.controller.log_folder_input = good_dir.display().to_string();
    app.controller.toggle_recording();
    assert!(wait_until(
        &mut app.controller,
        Duration::from_secs(3),
        |c| { c.is_recording }
    ));
    assert!(app.controller.last_recording_error.is_none());
    app.controller.request_disconnect();
    assert!(wait_until(
        &mut app.controller,
        Duration::from_secs(3),
        |c| { !c.connected }
    ));
}

#[test]
fn stop_recording_with_unwritable_folder_reports_finalize_failure() {
    let dir = tempdir().unwrap();
    let rec_dir = dir.path().join("rec");
    let mut controller = connected_controller(dir.path(), &rec_dir);

    controller.toggle_recording();
    assert!(wait_until(&mut controller, Duration::from_secs(3), |c| c.is_recording));
    assert!(wait_until(&mut controller, Duration::from_secs(3), |c| {
        c.total_samples_recorded >= 3
    }));

    sabotage_folder(&rec_dir);

    // Stopping flushes the part; that write must fail loudly, not vanish.
    controller.toggle_recording();
    assert!(wait_until(&mut controller, Duration::from_secs(5), |c| !c.is_recording));
    let err = controller
        .last_recording_error
        .clone()
        .expect("finalize failure must be reported");
    assert!(err.contains("finalize"), "{err}");
    assert!(controller.connected);
}
