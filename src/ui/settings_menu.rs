// SPDX-License-Identifier: GPL-2.0-only
//! Dedicated Settings Menu modal (`⚙ Settings`) allowing configuration of
//! Target Connection (IP, Port, UDP/TCP, local A2L fallback, Mock server),
//! Recording & Logfile/Video Split Size (default 500 MB), 2D Video Recording options,
//! Ego Vehicle dimensions (`length_m`, `width_m`, `rear_axle_to_center_m`),
//! Calibration options, and the Plot Profile selector, persisted in `./config/settings.json`.

use crate::app_state::AppController;
use crate::config_store::{AppSettings, TransportProtocol};
use crate::ui::profile_panel::render_profile_section;
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
        .default_width(520.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                render_settings_contents(ui, controller);
            });
        });
    controller.settings_menu_open = open;
}

/// Renders the inner form of the Settings Menu (also callable directly in headless UI tests).
pub fn render_settings_contents(ui: &mut Ui, controller: &mut AppController) {
    ui.heading(RichText::new("Plot Profiles (Predefined Measurement Views)").strong());
    ui.weak(
        "A profile stores the 2D (X, Y) layers, Time Plot selection, Ego geometry, viewport and \
         calibration watch-list as one .json file in ./config/profiles/ for reuse across sessions.",
    );
    render_profile_section(ui, controller);

    ui.separator();
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
    ui.heading(RichText::new("Recording & Synchronized Size Splitting (.sdf + .mp4)").strong());
    egui::Grid::new("settings_rec_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Logfile / Video Split Size (MB):");
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

            ui.label("Record 2D Plot Video (.mp4):");
            ui.checkbox(
                &mut controller.settings.recording.record_video_mp4,
                "Record H.264 .mp4 video alongside .sdf logfile",
            );
            ui.end_row();

            ui.label("Video Frame Rate (FPS):");
            ui.add(
                egui::DragValue::new(&mut controller.settings.recording.video_fps)
                    .range(1..=30)
                    .suffix(" FPS"),
            );
            ui.end_row();

            ui.label("Video H.264 Encoder:");
            ui.horizontal_wrapped(|ui| {
                for enc in ["libx264", "h264_nvenc", "h264_qsv", "h264_amf", "native"] {
                    ui.selectable_value(
                        &mut controller.settings.recording.video_encoder,
                        enc.to_string(),
                        enc,
                    );
                }
            });
            ui.end_row();
        });

    ui.separator();
    ui.heading(RichText::new("Ego Vehicle Geometry & 2D Coordinate System").strong());
    egui::Grid::new("settings_ego_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Show Ego Vehicle:");
            ui.checkbox(
                &mut controller.settings.visualization.ego.show_ego,
                "Render Ego footprint & rear-axle origin at (0, 0)",
            );
            ui.end_row();

            ui.label("Coordinate Convention:");
            ui.checkbox(
                &mut controller.settings.visualization.ego.iso8855_axes,
                "ISO 8855 Vehicle Axes (+X Forward/Up, +Y Left)",
            );
            ui.end_row();

            ui.label("Ego Length (m):");
            ui.add(
                egui::DragValue::new(&mut controller.settings.visualization.ego.length_m)
                    .range(0.5..=30.0)
                    .speed(0.05)
                    .suffix(" m"),
            );
            ui.end_row();

            ui.label("Ego Width (m):");
            ui.add(
                egui::DragValue::new(&mut controller.settings.visualization.ego.width_m)
                    .range(0.5..=10.0)
                    .speed(0.05)
                    .suffix(" m"),
            );
            ui.end_row();

            ui.label("Rear-Axle to Center (m):");
            ui.add(
                egui::DragValue::new(
                    &mut controller.settings.visualization.ego.rear_axle_to_center_m,
                )
                .range(-15.0..=15.0)
                .speed(0.05)
                .suffix(" m"),
            );
            ui.end_row();
        });
    let ego = &controller.settings.visualization.ego;
    ui.weak(format!(
        "Footprint wrt Rear Axle (0,0): Front = {:+.2} m | Rear = {:+.2} m | Half-Width = ±{:.2} m",
        ego.front_extent_m(),
        ego.rear_extent_m(),
        ego.half_width_m()
    ));

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
