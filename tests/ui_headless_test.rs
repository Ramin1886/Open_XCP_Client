// SPDX-License-Identifier: GPL-2.0-only
//! Headless UI tests exercising `OpenXcpClientApp`, Top Bar, Settings Menu modal,
//! Logging Tab, and Calibration Tab across multiple `egui::Context` frames.

use open_xcp_client::a2l::parser::parse_a2l;
use open_xcp_client::app_state::{ActiveTab, AppController};
use open_xcp_client::config_store::ConfigStore;
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::ui::OpenXcpClientApp;
use tempfile::tempdir;

#[test]
fn test_headless_ui_tabs_and_settings_menu_rendering() {
    let dir = tempdir().unwrap();
    let store = ConfigStore::new(dir.path().join("settings.json"));
    let mut controller = AppController::new(store);

    // Populate A2L model and sample calibration variables to exercise full UI trees and tables
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    controller.a2l_model = Some(model);
    controller.connected = true;
    controller.target_endpoint = "127.0.0.1:5555 (UDP)".to_string();
    controller.add_calibration_variable("Cal.Controller.kp_gain");
    controller.add_calibration_variable("Cal.Controller.operating_mode");
    controller.add_calibration_variable("Cal.Controller.speed_lookup_curve");

    let mut app = OpenXcpClientApp::new(controller);
    let ctx = egui::Context::default();

    // Frame 1: Render Logging Tab + Open Settings Menu modal
    app.controller.active_tab = ActiveTab::Numerical;
    app.controller.settings_menu_open = true;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Frame 2: Filter measurements in Logging Tab & switch to light mode
    app.controller.measurement_search_filter = "RadarFront".to_string();
    app.controller.settings.ui.dark_mode = false;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Frame 3: Switch to Calibration Tab and render finder + active content table
    app.controller.active_tab = ActiveTab::Calibration;
    app.controller.calibration_search_filter = "Controller".to_string();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.render_frame(ctx);
    });

    // Verify Enter submission from controller
    app.controller
        .calibration_manager
        .update_input_buffer("Cal.Controller.kp_gain", "8.25".to_string());
    app.controller.connected = false; // offline: nothing may be written or shown as synced
    assert!(app
        .controller
        .submit_calibration_on_enter("Cal.Controller.kp_gain")
        .is_err());
    assert!(
        app.controller.status_message.contains("Not connected"),
        "status: {}",
        app.controller.status_message
    );
    assert!(app.controller.save_settings().is_ok());
}
