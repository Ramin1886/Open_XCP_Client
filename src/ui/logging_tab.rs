// SPDX-License-Identifier: GPL-2.0-only
//! Logging Tab UI:
//! - Entry field for storing folder (default `./logfile`)
//! - Entry field for logfile name prefix (producing `<Prefix>_<UTC>_PART001.sdf`)
//! - Start / Stop Recording toggle button
//! - Live file size progress toward the configured split limit (default 500 MB)
//! - Hierarchical C/C++ structure measurement & event tree browser

use crate::a2l::model::{A2lModel, StructTreeNode};
use crate::app_state::AppController;
use crate::ui::theme::{COLOR_READY_START, COLOR_RECORDING};
use crate::xcp::daq_planner::DecodedValue;
use egui::{Color32, ProgressBar, RichText, ScrollArea, Ui};
use std::collections::BTreeMap;

/// Renders the Logging Tab inside the central panel.
pub fn render_logging_tab(ui: &mut Ui, controller: &mut AppController) {
    ui.group(|ui| {
        ui.heading(RichText::new("SDF Time-Series Recording Controls").strong());
        ui.add_space(4.0);

        egui::Grid::new("logging_controls_grid")
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label("Storage Folder:");
                ui.add_enabled(
                    !controller.is_recording,
                    egui::TextEdit::singleline(&mut controller.log_folder_input)
                        .desired_width(340.0),
                );
                ui.end_row();

                ui.label("Logfile Name Prefix:");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        !controller.is_recording,
                        egui::TextEdit::singleline(&mut controller.log_prefix_input)
                            .desired_width(220.0),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Preview: {}/{}",
                            controller.log_folder_input.trim_end_matches('/'),
                            controller.preview_next_logfile_name()
                        ))
                        .monospace()
                        .weak(),
                    );
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let (btn_label, btn_color) = if controller.is_recording {
                ("⏹ Stop Recording", COLOR_RECORDING)
            } else {
                ("⏺ Start Recording", COLOR_READY_START)
            };

            let button = egui::Button::new(
                RichText::new(btn_label)
                    .size(16.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(btn_color)
            .min_size(egui::vec2(190.0, 36.0));

            if ui.add(button).clicked() {
                controller.toggle_recording();
            }

            ui.separator();

            let max_mb = controller.settings.recording.max_logfile_size_mb;
            let cur_mb = controller.current_part_bytes as f64 / 1_000_000.0;
            let ratio = if controller.max_part_bytes > 0 {
                (controller.current_part_bytes as f32 / controller.max_part_bytes as f32)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Part: PART{:03} | Samples: {} | Split Limit: {} MB",
                            controller.active_part_number,
                            controller.total_samples_recorded,
                            max_mb
                        ))
                        .strong(),
                    );
                    if let Some(active) = &controller.active_recording_file {
                        ui.label(RichText::new(format!("({})", active.display())).monospace());
                    }
                });
                ui.add(
                    ProgressBar::new(ratio)
                        .desired_width(360.0)
                        .text(format!("{:.2} MB / {} MB", cur_mb, max_mb)),
                );
            });
        });
    });

    ui.add_space(8.0);

    // Measurement & Event Structure Tree
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Recorded Measurement Variables & Hierarchy").strong());
            ui.separator();
            ui.label("Filter:");
            ui.text_edit_singleline(&mut controller.measurement_search_filter);
            if ui.button("Clear").clicked() {
                controller.measurement_search_filter.clear();
            }
        });
        ui.separator();

        if let Some(model) = &controller.a2l_model {
            let filter = controller.measurement_search_filter.trim().to_lowercase();
            ScrollArea::vertical()
                .id_salt("measurement_hierarchy_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_struct_tree_node(
                        ui,
                        &model.hierarchy_root,
                        model,
                        &controller.latest_measurement_values,
                        &filter,
                        true,
                    );
                });
        } else {
            ui.label(
                "Not connected to target. Click 'Connect' in the top bar (or enable Mock Server in '⚙ Settings') to load A2L measurements.",
            );
        }
    });
}

fn render_struct_tree_node(
    ui: &mut Ui,
    node: &StructTreeNode,
    model: &A2lModel,
    latest_values: &BTreeMap<usize, DecodedValue>,
    filter: &str,
    default_open: bool,
) {
    if let Some(m_idx) = node.measurement_index {
        if let Some(m) = model.measurements.get(m_idx) {
            if !filter.is_empty()
                && !m.name.to_lowercase().contains(filter)
                && !m.description.to_lowercase().contains(filter)
            {
                return;
            }
            let rate_str = m
                .event_id
                .and_then(|eid| model.events.get(&eid))
                .map(|ev| format!("{} ({:.1} ms)", ev.name, ev.refresh_rate_ms()))
                .unwrap_or_else(|| "Default Event".to_string());
            let dim_str = if m.matrix_dim.is_empty() {
                String::new()
            } else {
                format!("{:?}", m.matrix_dim)
            };
            let val_str = latest_values
                .get(&m_idx)
                .map(|v| v.format_compact())
                .unwrap_or_else(|| "—".to_string());

            ui.horizontal(|ui| {
                ui.label(RichText::new(&node.name).monospace().strong());
                ui.label(
                    RichText::new(format!("[{}{}]", m.data_type.as_str(), dim_str))
                        .small()
                        .weak(),
                );
                ui.label(RichText::new(rate_str).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !m.unit.is_empty() {
                        ui.label(RichText::new(&m.unit).weak());
                    }
                    ui.label(RichText::new(val_str).monospace().strong());
                });
            });
        }
        return;
    }

    if !filter.is_empty() && !subtree_matches_filter(node, model, filter) {
        return;
    }

    egui::CollapsingHeader::new(RichText::new(&node.name).strong())
        .id_salt(&node.full_path)
        .default_open(default_open)
        .show(ui, |ui| {
            for child in node.children.values() {
                render_struct_tree_node(ui, child, model, latest_values, filter, true);
            }
        });
}

fn subtree_matches_filter(node: &StructTreeNode, model: &A2lModel, filter: &str) -> bool {
    if let Some(m_idx) = node.measurement_index {
        if let Some(m) = model.measurements.get(m_idx) {
            return m.name.to_lowercase().contains(filter)
                || m.description.to_lowercase().contains(filter);
        }
    }
    node.children
        .values()
        .any(|c| subtree_matches_filter(c, model, filter))
}
