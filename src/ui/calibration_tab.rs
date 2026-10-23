// SPDX-License-Identifier: GPL-2.0-only
//! Calibration Tab UI:
//! - Searchable A2L parameter finder (`CHARACTERISTIC`) to add variables to active content
//! - Active Calibration Content table with data-type-specific input fields showing current value
//! - Pressing Enter validates input against data type & limits and sends the new value to target

use crate::app_state::AppController;
use crate::calibration::manager::CalibrationSyncStatus;
use crate::ui::theme::{COLOR_CONNECTED, COLOR_RECORDING, COLOR_WARNING};
use egui::{Key, RichText, ScrollArea, Ui};

/// Renders the Calibration Tab inside the central panel.
pub fn render_calibration_tab(ui: &mut Ui, controller: &mut AppController) {
    let Some(model) = controller.a2l_model.clone() else {
        ui.group(|ui| {
            ui.heading("Calibration Parameters");
            ui.label(
                "No A2L loaded yet. Connect to an XCP target (or enable the built-in Mock Server in '⚙ Settings') to browse and calibrate parameters.",
            );
        });
        return;
    };

    ui.columns(2, |columns| {
        // Left Column: Parameter Finder
        columns[0].group(|ui| {
            ui.heading(RichText::new("1. Find Calibration Variable").strong());
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut controller.calibration_search_filter);
                if ui.button("Clear").clicked() {
                    controller.calibration_search_filter.clear();
                }
            });
            ui.separator();

            let filter = controller.calibration_search_filter.trim().to_lowercase();
            let mut to_add: Option<String> = None;

            ScrollArea::vertical()
                .id_salt("calibration_finder_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for ch in &model.characteristics {
                        if !filter.is_empty()
                            && !ch.name.to_lowercase().contains(&filter)
                            && !ch.description.to_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        let already_added = controller
                            .calibration_manager
                            .entries()
                            .contains_key(&ch.name);

                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!already_added, egui::Button::new("+ Add"))
                                .clicked()
                            {
                                to_add = Some(ch.name.clone());
                            }
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&ch.name).monospace().strong());
                                ui.label(
                                    RichText::new(format!(
                                        "{} [{:.2} .. {:.2}] {} — {}",
                                        ch.data_type.as_str(),
                                        ch.lower_limit,
                                        ch.upper_limit,
                                        ch.unit,
                                        ch.description
                                    ))
                                    .small()
                                    .weak(),
                                );
                            });
                        });
                        ui.separator();
                    }
                });

            if let Some(name) = to_add {
                controller.add_calibration_variable(&name);
            }
        });

        // Right Column: Active Calibration Content Table
        columns[1].group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("2. Active Calibration Content").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear All").clicked() {
                        controller.calibration_manager.clear();
                    }
                    if ui.button("↻ Refresh from Target").clicked() {
                        controller.refresh_all_calibrations();
                    }
                });
            });
            ui.label(
                RichText::new(
                    "Edit any value according to its data type and press Enter to send to target.",
                )
                .small()
                .weak(),
            );
            ui.separator();

            let names: Vec<String> = controller
                .calibration_manager
                .entries()
                .keys()
                .cloned()
                .collect();

            let mut enter_submitted_name: Option<String> = None;
            let mut remove_name: Option<String> = None;

            ScrollArea::vertical()
                .id_salt("calibration_content_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for name in names {
                        let Some(entry) =
                            controller.calibration_manager.entries_mut().get_mut(&name)
                        else {
                            continue;
                        };
                        let ch = entry.characteristic.clone();
                        let enum_vtab = model.enum_table_for(&ch.compu_method).cloned();

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&ch.name).monospace().strong());
                                ui.label(
                                    RichText::new(format!(
                                        "({} | [{} .. {}] {})",
                                        ch.data_type.as_str(),
                                        ch.lower_limit,
                                        ch.upper_limit,
                                        ch.unit
                                    ))
                                    .small()
                                    .weak(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("✖").clicked() {
                                            remove_name = Some(name.clone());
                                        }
                                        match &entry.status {
                                            CalibrationSyncStatus::Synced => {
                                                ui.colored_label(COLOR_CONNECTED, "✓ Synced");
                                            }
                                            CalibrationSyncStatus::Modified => {
                                                ui.colored_label(
                                                    COLOR_WARNING,
                                                    "✎ Modified (Press Enter)",
                                                );
                                            }
                                            CalibrationSyncStatus::Sending => {
                                                ui.colored_label(COLOR_WARNING, "⏳ Sending...");
                                            }
                                            CalibrationSyncStatus::PendingRead => {
                                                ui.weak("Reading...");
                                            }
                                            CalibrationSyncStatus::Error(err) => {
                                                ui.colored_label(
                                                    COLOR_RECORDING,
                                                    format!("⚠ {err}"),
                                                );
                                            }
                                        }
                                    },
                                );
                            });

                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "Current: {}",
                                        entry.target_value_display
                                    ))
                                    .monospace(),
                                );
                                ui.separator();
                                ui.label("New Value:");

                                // Provide quick-select buttons for TAB_VERB enums alongside the text input
                                if let Some(vtab) = &enum_vtab {
                                    for label in vtab.values() {
                                        if ui
                                            .selectable_label(entry.input_buffer == *label, label)
                                            .clicked()
                                        {
                                            entry.input_buffer = label.clone();
                                            entry.status = CalibrationSyncStatus::Modified;
                                            enter_submitted_name = Some(name.clone());
                                        }
                                    }
                                }

                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut entry.input_buffer)
                                        .desired_width(180.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                if resp.changed() {
                                    entry.status = if entry.input_buffer.trim()
                                        == entry.target_value_display.trim()
                                    {
                                        CalibrationSyncStatus::Synced
                                    } else {
                                        CalibrationSyncStatus::Modified
                                    };
                                }
                                let enter_pressed =
                                    resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                                if enter_pressed || ui.small_button("↵ Enter").clicked() {
                                    enter_submitted_name = Some(name.clone());
                                }
                            });
                        });
                    }
                });

            if let Some(rem) = remove_name {
                controller.calibration_manager.remove(&rem);
            }
            if let Some(sub_name) = enter_submitted_name {
                let _ = controller.submit_calibration_on_enter(&sub_name);
            }
        });
    });
}
