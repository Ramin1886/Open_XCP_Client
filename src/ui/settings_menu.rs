// SPDX-License-Identifier: GPL-2.0-only
//! Dedicated Settings Menu modal (`⚙ Settings`) allowing configuration of
//! Target Connection (IP, Port, UDP/TCP, local A2L fallback, Mock server),
//! Recording & Logfile Split Size (default 500 MB), and Calibration options,
//! persisted in `./config/settings.json`.

use crate::app_state::AppController;
use crate::config_store::{AppSettings, TransportProtocol};
use egui::{Context, RichText, Ui};

/// Renders the Settings Menu modal window when `controller.settings_menu_open == true`.
pub fn render_settings_menu(ctx: &Context, controller: &mut AppController) {
    if !controller.settings_menu_open {
        return;
    }

    let mut open = controller.settings_menu_open;
    egui::Window::new("⚙ Settings")
        .open(&mut open)
        .resizable(true)
        .default_width(480.0)
        .show(ctx, |ui| {
            render_settings_contents(ui, controller);
        });
    controller.settings_menu_open = open;
}

/// Renders the inner form of the Settings Menu (also callable directly in headless UI tests).
pub fn render_settings_contents(ui: &mut Ui, controller: &mut AppController) {
    ui.heading(RichText::new("Target XCP Connection").strong());
    egui::Grid::new("settings_conn_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Target IP Address:");
            ui.text_edit_singleline(&mut controller.settings.connection.target_ip);
            ui.end_row();

            ui.label("Target XCP Port:");
            let mut port_val = controller.settings.connection.target_port as u32;
            if ui
                .add(egui::DragValue::new(&mut port_val).range(1..=65535))
                .changed()
            {
                controller.settings.connection.target_port = port_val as u16;
            }
            ui.end_row();

            ui.label("Transport Protocol:");
            ui.horizontal(|ui| {
                ui.radio_value(
                    &mut controller.settings.connection.transport,
                    TransportProtocol::Udp,
                    "UDP",
                );
                ui.radio_value(
                    &mut controller.settings.connection.transport,
                    TransportProtocol::Tcp,
                    "TCP",
                );
            });
            ui.end_row();

            ui.label("Connect Timeout (ms):");
            ui.add(
                egui::DragValue::new(&mut controller.settings.connection.connect_timeout_ms)
                    .range(200..=30_000),
            );
            ui.end_row();

            ui.label("Local A2L Fallback Path:");
            ui.text_edit_singleline(&mut controller.settings.connection.local_a2l_fallback_path);
            ui.end_row();

            ui.label("Built-in Mock XCPlite Server:");
            ui.checkbox(
                &mut controller.settings.ui.enable_mock_server,
                "Use simulated QNX ADAS target on Connect",
            );
            ui.end_row();
        });

    ui.separator();
    ui.heading(RichText::new("Recording & Logfile Splitting").strong());
    egui::Grid::new("settings_rec_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Logfile Split Size (MB):");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut controller.settings.recording.max_logfile_size_mb)
                        .range(1..=10_000)
                        .suffix(" MB"),
                );
                ui.label("(Default: 500 MB)");
            });
            ui.end_row();

            ui.label("Default Log Folder:");
            ui.text_edit_singleline(&mut controller.settings.recording.default_log_folder);
            ui.end_row();

            ui.label("Default Logfile Prefix:");
            ui.text_edit_singleline(&mut controller.settings.recording.default_file_prefix);
            ui.end_row();

            ui.label("Target UTC Clock:");
            ui.checkbox(
                &mut controller.settings.recording.prefer_target_utc_clock,
                "Prefer target DAQ clock correlated to UTC",
            );
            ui.end_row();
        });

    ui.separator();
    ui.heading(RichText::new("Calibration & Interface").strong());
    egui::Grid::new("settings_cal_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Readback Verification:");
            ui.checkbox(
                &mut controller.settings.calibration.verify_readback,
                "Verify target memory via SHORT_UPLOAD after every calibration write",
            );
            ui.end_row();

            ui.label("Active Calibration Page:");
            ui.add(
                egui::DragValue::new(&mut controller.settings.calibration.active_cal_page)
                    .range(0..=1),
            );
            ui.end_row();

            ui.label("Theme:");
            ui.checkbox(&mut controller.settings.ui.dark_mode, "Dark Mode");
            ui.end_row();
        });

    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .button(RichText::new("💾 Save Settings to JSON").strong())
            .clicked()
        {
            if let Err(err) = controller.save_settings() {
                controller.status_message = format!("Settings validation error: {err}");
            } else {
                controller.log_folder_input =
                    controller.settings.recording.default_log_folder.clone();
                controller.log_prefix_input =
                    controller.settings.recording.default_file_prefix.clone();
            }
        }
        if ui.button("↺ Reset to Defaults").clicked() {
            controller.settings = AppSettings::default();
            controller.log_folder_input = controller.settings.recording.default_log_folder.clone();
            controller.log_prefix_input = controller.settings.recording.default_file_prefix.clone();
            let _ = controller.save_settings();
        }
    });
}
