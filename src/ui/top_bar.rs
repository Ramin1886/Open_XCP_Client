// SPDX-License-Identifier: GPL-2.0-only
//! Top navigation bar with primary Tab switcher (`Logging` | `Calibration`),
//! connection status badge, Connect/Disconnect button, and the `⚙ Settings` menu button.

use crate::app_state::{ActiveTab, AppController};
use crate::ui::theme::{COLOR_CONNECTED, COLOR_DISCONNECTED};
use egui::{RichText, Ui};

/// Renders the application top navigation bar.
pub fn render_top_bar(ui: &mut Ui, controller: &mut AppController) {
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Open_XCP_Client").strong());
        ui.separator();

        // Tab Switcher
        let logging_selected = controller.active_tab == ActiveTab::Logging;
        if ui
            .selectable_label(logging_selected, RichText::new("● Logging").strong())
            .clicked()
        {
            controller.active_tab = ActiveTab::Logging;
        }

        let cal_selected = controller.active_tab == ActiveTab::Calibration;
        if ui
            .selectable_label(cal_selected, RichText::new("◈ Calibration").strong())
            .clicked()
        {
            controller.active_tab = ActiveTab::Calibration;
        }

        ui.separator();

        // Connection Status Pill
        if controller.connected {
            ui.colored_label(
                COLOR_CONNECTED,
                format!("● Connected: {}", controller.target_endpoint),
            );
            if let Some(model) = &controller.a2l_model {
                ui.label(format!(
                    "[{} Meas | {} Events | {} Cal]",
                    model.measurements.len(),
                    model.events.len(),
                    model.characteristics.len()
                ));
            }
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
                    "Open Settings Menu (Target IP/Port, Logfile Split Size MB, Presets)",
                )
                .clicked()
            {
                controller.settings_menu_open = !controller.settings_menu_open;
            }
        });
    });
}
