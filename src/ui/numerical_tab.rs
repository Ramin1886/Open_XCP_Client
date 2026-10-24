// SPDX-License-Identifier: GPL-2.0-only
//! `Numerical` Tab (`src/ui/numerical_tab.rs`).
//!
//! Displays a filterable, expandable hierarchical tree of all A2L / `.sdf` measurement
//! variables showing their current physical value (with `COMPU_VTAB` verbal enum resolution
//! or array summary `[N] v0, v1, ...`), engineering unit, raw value, XCP event rate,
//! and a 1.0-second visual highlight whenever a value changes.

use crate::a2l::model::{A2lModel, StructTreeNode};
use crate::app_state::{AppController, VALUE_CHANGE_HIGHLIGHT_SECONDS};
use crate::xcp::daq_planner::DecodedValue;
use egui::{Color32, RichText, ScrollArea, Ui};
use std::collections::BTreeMap;
use std::time::Instant;

/// Renders the `Numerical` tab (`ActiveTab::Numerical`).
pub fn render_numerical_tab(ui: &mut Ui, controller: &mut AppController) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Numerical Signal Inspector").strong());
            ui.separator();
            ui.label("🔍 Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut controller.measurement_search_filter)
                    .desired_width(240.0)
                    .hint_text("Filter by signal or struct name..."),
            );
            if ui.button("Clear").clicked() {
                controller.measurement_search_filter.clear();
            }
            ui.separator();
            if ui.button("➕ Expand All").clicked() {
                controller.numerical_expand_all = Some(true);
            }
            if ui.button("➖ Collapse All").clicked() {
                controller.numerical_expand_all = Some(false);
            }
            if let Some(model) = &controller.a2l_model {
                ui.weak(format!(
                    "({} signals across {} events)",
                    model.measurements.len(),
                    model.events.len()
                ));
            }
        });
        ui.separator();

        let expand_override = controller.numerical_expand_all.take();

        if let Some(model) = &controller.a2l_model {
            let filter = controller.measurement_search_filter.trim().to_lowercase();
            let now = Instant::now();
            ScrollArea::vertical()
                .id_salt("numerical_hierarchy_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_struct_tree_node(
                        ui,
                        &model.hierarchy_root,
                        model,
                        &controller.latest_measurement_values,
                        &controller.signal_changed_at,
                        now,
                        &filter,
                        expand_override,
                    );
                });
        } else {
            ui.label(
                "No signals loaded yet. Click 'Connect' in Live mode (or enable Mock Server in '⚙ Settings') or load an .sdf file in Replay mode.",
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn render_struct_tree_node(
    ui: &mut Ui,
    node: &StructTreeNode,
    model: &A2lModel,
    latest_values: &BTreeMap<usize, DecodedValue>,
    changed_at: &BTreeMap<usize, Instant>,
    now: Instant,
    filter: &str,
    expand_override: Option<bool>,
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

            let recently_changed = changed_at
                .get(&m_idx)
                .map(|t| now.duration_since(*t).as_secs_f64() < VALUE_CHANGE_HIGHLIGHT_SECONDS)
                .unwrap_or(false);

            let (val_str, raw_str) = if let Some(val) = latest_values.get(&m_idx) {
                let primary = val.primary_scalar();
                let raw_val = model.physical_to_raw(&m.compu_method, primary);
                let enum_lbl = model
                    .enum_table_for(&m.compu_method)
                    .and_then(|vt| vt.get(&(raw_val.round() as i64)))
                    .cloned();
                let formatted = if let Some(lbl) = enum_lbl {
                    format!("{} ({:.0})", lbl, primary)
                } else {
                    val.format_compact()
                };
                (formatted, format!("raw: {:.2}", raw_val))
            } else {
                ("—".to_string(), "raw: —".to_string())
            };

            let val_color = if recently_changed {
                Color32::from_rgb(56, 189, 248)
            } else {
                ui.visuals().text_color()
            };

            ui.horizontal(|ui| {
                ui.label(RichText::new(&node.name).monospace().strong());
                ui.label(
                    RichText::new(format!("[{}{}]", m.data_type.as_str(), dim_str))
                        .small()
                        .weak(),
                );
                ui.label(RichText::new(rate_str).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(raw_str).monospace().small().weak());
                    if !m.unit.is_empty() {
                        ui.label(RichText::new(&m.unit).weak());
                    }
                    ui.label(RichText::new(val_str).monospace().strong().color(val_color));
                });
            });
        }
        return;
    }

    if !filter.is_empty() && !subtree_matches_filter(node, model, filter) {
        return;
    }

    let mut header = egui::CollapsingHeader::new(RichText::new(&node.name).strong())
        .id_salt(&node.full_path)
        .default_open(true);
    if let Some(open_state) = expand_override {
        header = header.open(Some(open_state));
    }
    header.show(ui, |ui| {
        for child in node.children.values() {
            render_struct_tree_node(
                ui,
                child,
                model,
                latest_values,
                changed_at,
                now,
                filter,
                expand_override,
            );
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
