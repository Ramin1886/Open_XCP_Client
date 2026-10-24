// SPDX-License-Identifier: GPL-2.0-only
//! Top navigation bar with `Live` / `Replay` mode switch, 5-tab selector
//! (`2D Live Plot`, `Numerical`, `Time Plot`, `Calibration`, `XCP Info`),
//! Live Recording bar (`.sdf` + `.mp4`), `.sdf` Replay bar, and `⚙ Settings` menu button.

use crate::app_state::{ActiveTab, AppController, AppMode};
use crate::replay::REPLAY_SPEEDS;
use crate::ui::theme::{COLOR_CONNECTED, COLOR_DISCONNECTED, COLOR_RECORDING};
use egui::{Color32, RichText, Slider, Ui};
use std::path::PathBuf;

/// Renders the application top navigation bar and context-sensitive Live/Replay bar.
pub fn render_top_bar(ui: &mut Ui, controller: &mut AppController) {
    // Handle keyboard shortcuts (Ctrl+1..5 for tabs, Ctrl+R for recording)
    let input_shortcut = ui.ctx().input(|i| {
        let mut tab = None;
        if i.modifiers.ctrl && i.key_pressed(egui::Key::Num1) {
            tab = Some(ActiveTab::PlanePlot);
        } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Num2) {
            tab = Some(ActiveTab::Numerical);
        } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Num3) {
            tab = Some(ActiveTab::TimePlot);
        } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Num4) {
            tab = Some(ActiveTab::Calibration);
        } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Num5) {
            tab = Some(ActiveTab::XcpInfo);
        }
        let toggle_rec = i.modifiers.ctrl && i.key_pressed(egui::Key::R);
        (tab, toggle_rec)
    });
    if let Some(t) = input_shortcut.0 {
        controller.active_tab = t;
    }
    if input_shortcut.1 && controller.mode == AppMode::Live {
        controller.toggle_recording();
    }

    // Row 1: Brand, Mode Switch, 5 Tabs, Connection Status, Settings Button
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Open_XCP_Client").strong());
        ui.separator();

        // Mode Switch: Live vs Replay
        let is_live = controller.mode == AppMode::Live;
        if ui
            .selectable_label(is_live, RichText::new("🟢 Live").strong())
            .on_hover_text("Live XCP target streaming, recording (.sdf + .mp4), and calibration")
            .clicked()
        {
            controller.set_mode(AppMode::Live);
        }
        if ui
            .selectable_label(!is_live, RichText::new("🎞 Replay").strong())
            .on_hover_text("Replay single or multi-part .sdf recordings offline")
            .clicked()
        {
            controller.set_mode(AppMode::Replay);
        }

        ui.separator();

        // 5-Tab Switcher
        let tabs = [
            (ActiveTab::PlanePlot, "📐 2D Live Plot", "Ctrl+1"),
            (ActiveTab::Numerical, "🔢 Numerical", "Ctrl+2"),
            (ActiveTab::TimePlot, "📈 Time Plot", "Ctrl+3"),
            (ActiveTab::Calibration, "🎛 Calibration", "Ctrl+4"),
            (ActiveTab::XcpInfo, "📡 XCP Info", "Ctrl+5"),
        ];
        for (tab, label, shortcut) in tabs {
            let selected = controller.active_tab == tab;
            if ui
                .selectable_label(selected, RichText::new(label).strong())
                .on_hover_text(format!("Switch to {} ({})", label, shortcut))
                .clicked()
            {
                controller.active_tab = tab;
            }
        }

        ui.separator();

        // Connection Status Pill
        if controller.connected {
            ui.colored_label(
                COLOR_CONNECTED,
                format!("● Connected: {}", controller.target_endpoint),
            );
            if ui.button("Disconnect").clicked() {
                controller.request_disconnect();
            }
        } else {
            ui.colored_label(
                COLOR_DISCONNECTED,
                format!(
                    "○ Disconnected ({}:{} {})",
                    controller.settings.connection.target_ip,
                    controller.settings.connection.target_port,
                    controller.settings.connection.transport.as_str()
                ),
            );
            if ui.button(RichText::new("Connect").strong()).clicked() {
                controller.request_connect();
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("⚙ Settings").strong())
                .on_hover_text(
                    "Open Settings Menu (Target IP/Port, Logfile Split Size MB, Ego Vehicle Dimensions, Video Options)",
                )
                .clicked()
            {
                controller.settings_menu_open = !controller.settings_menu_open;
            }
        });
    });

    ui.separator();

    // Row 2: Context-sensitive Live Recording Bar OR Replay Control Bar
    match controller.mode {
        AppMode::Live => render_live_record_bar(ui, controller),
        AppMode::Replay => render_replay_control_bar(ui, controller),
    }
}

fn render_live_record_bar(ui: &mut Ui, controller: &mut AppController) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Folder:").strong());
        ui.add_enabled(
            !controller.is_recording,
            egui::TextEdit::singleline(&mut controller.log_folder_input).desired_width(140.0),
        );

        ui.label(RichText::new("Name Prefix:").strong());
        ui.add_enabled(
            !controller.is_recording,
            egui::TextEdit::singleline(&mut controller.log_prefix_input).desired_width(130.0),
        )
        .on_hover_text(format!(
            "Next file: {}",
            controller.preview_next_logfile_name()
        ));

        ui.separator();

        ui.checkbox(
            &mut controller.settings.recording.record_video_mp4,
            "2D Video (.mp4)",
        )
        .on_hover_text("Simultaneously record the 2D Live Plot as an H.264 .mp4 video file with synchronized size splitting");

        ui.label("FPS:");
        if ui
            .add(
                egui::DragValue::new(&mut controller.settings.recording.video_fps)
                    .range(1..=30)
                    .suffix(" Hz"),
            )
            .changed()
        {
            controller.sync_video_scene_to_worker();
        }

        ui.separator();

        let (btn_text, btn_color) = if controller.is_recording {
            (
                RichText::new("⏹ Stop Recording (Ctrl+R)")
                    .strong()
                    .color(Color32::WHITE),
                COLOR_RECORDING,
            )
        } else {
            (
                RichText::new("⏺ Start Recording (Ctrl+R)")
                    .strong()
                    .color(Color32::WHITE),
                Color32::from_rgb(22, 163, 74),
            )
        };

        if ui
            .add(egui::Button::new(btn_text).fill(btn_color))
            .clicked()
        {
            controller.toggle_recording();
        }

        if controller.is_recording {
            let cur_mb = (controller.current_part_bytes as f64) / 1_000_000.0;
            let max_mb = (controller.max_part_bytes.max(1) as f64) / 1_000_000.0;
            ui.colored_label(
                COLOR_RECORDING,
                format!(
                    "● REC PART{:03} ({:.2} / {:.0} MB | {} samples)",
                    controller.active_part_number,
                    cur_mb,
                    max_mb,
                    controller.total_samples_recorded
                ),
            );
        } else {
            ui.weak(format!(
                "Split Limit: {} MB",
                controller.settings.recording.max_logfile_size_mb
            ));
        }
    });
}

/// Loads `.sdf` replay files from `controller.replay_file_input` and updates `status_message` on failure.
pub fn trigger_replay_load_from_input(controller: &mut AppController) {
    let paths = resolve_replay_input_paths(&controller.replay_file_input);
    if paths.is_empty() {
        controller.status_message = "No .sdf files found at the specified path.".to_string();
    } else if let Err(err) = controller.load_replay_files(&paths) {
        controller.status_message = format!("Replay load error: {err}");
    }
}

fn render_replay_control_bar(ui: &mut Ui, controller: &mut AppController) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(".sdf File(s) / Folder:").strong());
        ui.add(
            egui::TextEdit::singleline(&mut controller.replay_file_input)
                .desired_width(240.0)
                .hint_text("Path to .sdf file(s) (comma-separated) or folder"),
        );

        if ui.button(RichText::new("📂 Load .sdf").strong()).clicked() {
            trigger_replay_load_from_input(controller);
        }

        ui.separator();

        let (has_player, is_playing, mut pos_s, dur_s, cur_utc_s, cur_speed, part_count) =
            if let Some(p) = &controller.replay_player {
                (
                    true,
                    p.is_playing(),
                    p.position_s(),
                    p.duration_s(),
                    p.current_utc_s(),
                    p.speed(),
                    p.loaded_files().len(),
                )
            } else {
                (false, false, 0.0, 0.0, 0.0, 1.0, 0)
            };

        let play_label = if is_playing { "⏸ Pause" } else { "▶ Play" };
        if ui
            .add_enabled(
                has_player,
                egui::Button::new(RichText::new(play_label).strong()),
            )
            .clicked()
        {
            if let Some(p) = &mut controller.replay_player {
                p.toggle_play_pause();
            }
            controller.last_replay_tick = std::time::Instant::now();
        }

        if ui
            .add_enabled(has_player, egui::Button::new("■ Stop"))
            .clicked()
        {
            if let Some(p) = &mut controller.replay_player {
                p.stop();
            }
            controller.replay_seek(0.0);
        }

        // Speed selector pills
        ui.label("Speed:");
        for &(spd, lbl) in REPLAY_SPEEDS {
            if ui
                .selectable_label((spd - cur_speed).abs() < 1e-3, lbl)
                .clicked()
            {
                if let Some(p) = &mut controller.replay_player {
                    p.set_speed(spd);
                }
            }
        }

        ui.separator();

        // Continuous Time Slider across all parts
        let slider_max = dur_s.max(0.01);
        if ui
            .add_enabled(
                has_player,
                Slider::new(&mut pos_s, 0.0..=slider_max)
                    .show_value(false)
                    .trailing_fill(true),
            )
            .changed()
        {
            controller.replay_seek(pos_s);
        }

        ui.monospace(format!("{:.2}s / {:.2}s", pos_s, dur_s));
        if has_player {
            ui.weak(format!("[{} part(s) | UTC {:.3}s]", part_count, cur_utc_s));
        }
    });
}

/// Resolves user-provided `.sdf` file or folder input string into sorted `.sdf` [`PathBuf`]s.
pub fn resolve_replay_input_paths(input: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw_token in input.split([',', ';']) {
        let trimmed = raw_token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("sdf"))
                        .unwrap_or(false)
                    {
                        out.push(p);
                    }
                }
            }
        } else if path.exists() {
            out.push(path);
        }
    }
    out.sort();
    out.dedup();
    out
}
