// SPDX-License-Identifier: GPL-2.0-only
//! `Time Plot` Tab (`src/ui/time_plot_tab.rs`).
//!
//! Provides a filterable signal picker on the left (up to 12 simultaneous signals)
//! and a 2-column strip-chart grid on the right with configurable time window
//! (`10 s`, `20 s`, `60 s`, `120 s`), mouse-wheel time zoom (`1..120 s` anchored at
//! the newest sample on the right edge), double-click reset, and `Freeze` toggle.

use crate::app_state::AppController;
use crate::ui::axis_ticks::{grid_steps, tick_text, ticks};
use egui::{Align2, Color32, FontId, Pos2, RichText, ScrollArea, Sense, Stroke, Ui, Vec2};

/// Maximum number of simultaneous strip charts in the Time Plot grid.
pub const MAX_TIME_PLOT_SIGNALS: usize = 12;

const TRACE_PALETTE: &[[u8; 3]] = &[
    [56, 189, 248],
    [52, 211, 153],
    [251, 191, 36],
    [248, 113, 113],
    [167, 139, 250],
    [244, 114, 182],
    [96, 165, 250],
    [163, 230, 53],
];

/// Renders the `Time Plot` tab (`ActiveTab::TimePlot`).
pub fn render_time_plot_tab(ui: &mut Ui, controller: &mut AppController) {
    egui::SidePanel::left("time_plot_signal_picker_panel")
        .resizable(true)
        .default_width(280.0)
        .min_width(220.0)
        .show_inside(ui, |ui| {
            render_signal_picker(ui, controller);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        render_strip_charts(ui, controller);
    });
}

/// Resets the selected Time Plot signals to the first 4 measurements in the A2L model.
pub fn reset_time_plot_signals_to_defaults(controller: &mut AppController) {
    controller.settings.visualization.time_plot_signals.clear();
    if let Some(model) = &controller.a2l_model {
        for m in model.measurements.iter().take(4) {
            controller
                .settings
                .visualization
                .time_plot_signals
                .push(m.name.clone());
        }
    }
}

/// Toggles whether `sig_name` is included in `time_plot_signals` (up to [`MAX_TIME_PLOT_SIGNALS`]).
pub fn set_time_plot_signal_selected(
    controller: &mut AppController,
    sig_name: &str,
    checked: bool,
) {
    let list = &mut controller.settings.visualization.time_plot_signals;
    if checked {
        if !list.iter().any(|s| s == sig_name) && list.len() < MAX_TIME_PLOT_SIGNALS {
            list.push(sig_name.to_string());
        }
    } else {
        list.retain(|s| s != sig_name);
    }
}

fn render_signal_picker(ui: &mut Ui, controller: &mut AppController) {
    ui.heading(RichText::new("Time Plot Signals").strong());
    ui.horizontal(|ui| {
        if ui.button("Defaults").clicked() {
            reset_time_plot_signals_to_defaults(controller);
        }
        if ui.button("Clear").clicked() {
            controller.settings.visualization.time_plot_signals.clear();
        }
        ui.weak(format!(
            "{}/{}",
            controller.settings.visualization.time_plot_signals.len(),
            MAX_TIME_PLOT_SIGNALS
        ));
    });

    ui.label("🔍 Filter Signals:");
    ui.text_edit_singleline(&mut controller.time_plot_search_filter);
    ui.separator();

    let filter = controller.time_plot_search_filter.trim().to_lowercase();
    ScrollArea::vertical()
        .id_salt("time_plot_picker_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(model) = &controller.a2l_model else {
                ui.weak("Connect to target or load .sdf in Replay mode.");
                return;
            };

            let names: Vec<(String, String)> = model
                .measurements
                .iter()
                .map(|m| (m.name.clone(), m.description.clone()))
                .collect();

            for (name, desc) in names {
                if !filter.is_empty()
                    && !name.to_lowercase().contains(&filter)
                    && !desc.to_lowercase().contains(&filter)
                {
                    continue;
                }
                let mut checked = controller
                    .settings
                    .visualization
                    .time_plot_signals
                    .contains(&name);
                let can_check = checked
                    || controller.settings.visualization.time_plot_signals.len()
                        < MAX_TIME_PLOT_SIGNALS;
                if ui
                    .add_enabled(can_check, egui::Checkbox::new(&mut checked, &name))
                    .changed()
                {
                    set_time_plot_signal_selected(controller, &name, checked);
                }
            }
        });
}

fn render_strip_charts(ui: &mut Ui, controller: &mut AppController) {
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Rolling Time-Series Charts").strong());
        ui.separator();
        ui.label("Window:");
        for &win_s in &[10.0, 20.0, 60.0, 120.0] {
            let selected =
                (controller.settings.visualization.time_plot_window_s - win_s).abs() < 0.2;
            if ui
                .selectable_label(selected, format!("{:.0} s", win_s))
                .clicked()
            {
                controller.settings.visualization.time_plot_window_s = win_s;
            }
        }
        ui.weak(format!(
            "(Active: {:.1} s — Wheel to zoom, Double-click to reset)",
            controller.settings.visualization.time_plot_window_s
        ));
        ui.separator();
        ui.toggle_value(
            &mut controller.time_plot_frozen,
            RichText::new("❄ Freeze").strong(),
        );
    });
    ui.separator();

    let selected_signals = controller.settings.visualization.time_plot_signals.clone();
    if selected_signals.is_empty() {
        ui.label("Select up to 12 signals in the left panel to plot their time series.");
        return;
    }

    ScrollArea::vertical()
        .id_salt("time_plot_charts_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail_w = ui.available_width().max(320.0);
            let cols = if selected_signals.len() == 1 || avail_w < 560.0 {
                1
            } else {
                2
            };
            let cell_w = (avail_w - 12.0 * (cols as f32 - 1.0)) / (cols as f32);
            let cell_h = 185.0;

            for pair in selected_signals.chunks(cols) {
                ui.horizontal(|ui| {
                    for (sub_i, sig_name) in pair.iter().enumerate() {
                        let global_idx = selected_signals
                            .iter()
                            .position(|s| s == sig_name)
                            .unwrap_or(sub_i);
                        let rgb = TRACE_PALETTE[global_idx % TRACE_PALETTE.len()];
                        let trace_color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        render_single_strip_chart(
                            ui,
                            controller,
                            sig_name,
                            Vec2::new(cell_w, cell_h),
                            trace_color,
                        );
                    }
                });
                ui.add_space(6.0);
            }
        });
}

fn render_single_strip_chart(
    ui: &mut Ui,
    controller: &mut AppController,
    sig_name: &str,
    size: Vec2,
    trace_color: Color32,
) {
    let unit = controller
        .a2l_model
        .as_ref()
        .and_then(|m| m.measurements.iter().find(|x| x.name == sig_name))
        .map(|x| x.unit.clone())
        .unwrap_or_default();

    let latest_val_str = controller
        .latest_signals_by_name
        .get(sig_name)
        .map(|v| format!("{:.4}", v.primary_scalar()))
        .unwrap_or_else(|| "—".to_string());

    ui.vertical(|ui| {
        ui.set_width(size.x);
        ui.horizontal(|ui| {
            ui.colored_label(trace_color, RichText::new(sig_name).monospace().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !unit.is_empty() {
                    ui.weak(&unit);
                }
                ui.monospace(&latest_val_str);
            });
        });

        let plot_size = Vec2::new(size.x, (size.y - 24.0).max(90.0));
        let (response, painter) = ui.allocate_painter(plot_size, Sense::click());
        let rect = response.rect;
        painter.rect_filled(rect, 3.0, Color32::from_rgb(15, 20, 28));
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0_f32, Color32::from_rgb(38, 52, 72)),
        );

        if response.double_clicked() {
            controller.settings.visualization.time_plot_window_s = 20.0;
        }
        if response.hovered() {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.1 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.15 } else { 1.15 };
                controller.settings.visualization.time_plot_window_s =
                    (controller.settings.visualization.time_plot_window_s * factor)
                        .clamp(1.0, 120.0);
            }
        }

        let win_s = controller
            .settings
            .visualization
            .time_plot_window_s
            .clamp(1.0, 120.0);
        let series = controller.time_series_history.get(sig_name);
        let t_end = series
            .and_then(|s| s.back().map(|&(t, _)| t))
            .unwrap_or(controller.latest_sample_utc_s);
        let t_start = t_end - win_s;

        // Determine Y bounds over the visible window
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        if let Some(pts) = series {
            for &(t, y) in pts {
                if t >= t_start && y.is_finite() {
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }
        if !y_min.is_finite() || !y_max.is_finite() {
            y_min = -1.0;
            y_max = 1.0;
        } else if (y_max - y_min).abs() < 1e-6 {
            let pad = y_max.abs().max(1.0) * 0.1;
            y_min -= pad;
            y_max += pad;
        } else {
            let pad = (y_max - y_min) * 0.1;
            y_min -= pad;
            y_max += pad;
        }

        let w = rect.width() as f64;
        let h = rect.height() as f64;
        let to_screen = |t: f64, y: f64| -> Pos2 {
            let nx = ((t - t_start) / win_s.max(1e-6)).clamp(0.0, 1.0);
            let ny = ((y - y_min) / (y_max - y_min).max(1e-9)).clamp(0.0, 1.0);
            Pos2::new(
                rect.left() + (nx * w) as f32,
                rect.bottom() - (ny * h) as f32,
            )
        };

        // Draw Y grid lines + labels
        let (_, y_major) = grid_steps(y_max - y_min, h);
        for yv in ticks(y_min, y_max, y_major) {
            let p0 = to_screen(t_start, yv);
            let p1 = to_screen(t_end, yv);
            painter.line_segment(
                [p0, p1],
                Stroke::new(0.8_f32, Color32::from_rgb(30, 41, 59)),
            );
            painter.text(
                Pos2::new(rect.left() + 4.0, p0.y),
                Align2::LEFT_CENTER,
                tick_text(yv, y_major),
                FontId::monospace(9.5),
                Color32::from_rgb(148, 163, 184),
            );
        }

        // Draw relative time grid (-win_s .. 0 s)
        let (_, t_major) = grid_steps(win_s, w);
        for rel_t in ticks(-win_s, 0.0, t_major) {
            let abs_t = t_end + rel_t;
            let p0 = to_screen(abs_t, y_min);
            let p1 = to_screen(abs_t, y_max);
            painter.line_segment(
                [p0, p1],
                Stroke::new(0.8_f32, Color32::from_rgb(30, 41, 59)),
            );
            painter.text(
                Pos2::new(p0.x, rect.bottom() - 8.0),
                Align2::CENTER_CENTER,
                format!("{}s", tick_text(rel_t, t_major)),
                FontId::monospace(9.5),
                Color32::from_rgb(148, 163, 184),
            );
        }

        // Draw trace polyline
        if let Some(pts) = series {
            let poly: Vec<Pos2> = pts
                .iter()
                .filter(|&&(t, y)| t >= t_start && y.is_finite())
                .map(|&(t, y)| to_screen(t, y))
                .collect();
            if poly.len() >= 2 {
                painter.add(egui::Shape::line(poly, Stroke::new(1.8_f32, trace_color)));
            } else if let Some(&pt) = poly.first() {
                painter.circle_filled(pt, 3.0, trace_color);
            }
        }
    });
}
