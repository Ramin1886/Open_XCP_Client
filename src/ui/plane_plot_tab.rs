// SPDX-License-Identifier: GPL-2.0-only
//! Interactive `2D Live Plot` tab (`src/ui/plane_plot_tab.rs`).
//!
//! Renders the zoomable/pannable 2D ground plane in ISO 8855 vehicle coordinates
//! (`+x` forward/up, `+y` left, origin `(0, 0)` at rear axle center) or standard Cartesian axes,
//! draws the configurable Ego vehicle outline (`length_m`, `width_m`, `rear_axle_to_center_m`),
//! and plots arbitrary user-paired `(X, Y)` scalar or vector signals (`Scatter`, `Line`,
//! `TrajectoryTrail`, or `Polygon4Corner`).

use crate::app_state::AppController;
use crate::config_store::{XyDrawStyle, XyPlotLayerConfig};
use crate::recording::video_recorder::{
    extract_polygons_for_layer, extract_xy_points_for_layer, PlaneViewport,
};
use crate::ui::axis_ticks::{grid_steps, is_major, tick_text, ticks};
use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Ui, Vec2};

/// Renders the `2D Live Plot` tab (`ActiveTab::PlanePlot`).
pub fn render_plane_plot_tab(ui: &mut Ui, controller: &mut AppController) {
    egui::SidePanel::left("plane_plot_layer_builder_panel")
        .resizable(true)
        .default_width(310.0)
        .min_width(240.0)
        .show_inside(ui, |ui| {
            render_layer_sidebar(ui, controller);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        render_plane_canvas(ui, controller);
    });
}

/// Applies pan (`drag_delta`) and pointer-anchored wheel zoom (`scroll_y`) to `viewport`.
#[allow(clippy::too_many_arguments)]
pub fn apply_canvas_zoom_and_pan(
    viewport: &mut PlaneViewport,
    drag_delta: Vec2,
    scroll_y: f32,
    rel_sx: f64,
    rel_sy: f64,
    w: f64,
    h: f64,
    iso8855: bool,
) -> bool {
    let mut changed = false;
    if drag_delta.length_sq() > 0.0 {
        let ppm = viewport.pixels_per_meter.max(0.5);
        if iso8855 {
            viewport.center_x_m += (drag_delta.y as f64) / ppm;
            viewport.center_y_m += (drag_delta.x as f64) / ppm;
        } else {
            viewport.center_x_m -= (drag_delta.x as f64) / ppm;
            viewport.center_y_m += (drag_delta.y as f64) / ppm;
        }
        changed = true;
    }

    if scroll_y.abs() > 0.1 {
        let (wx_before, wy_before) = viewport.screen_to_world(rel_sx, rel_sy, w, h, iso8855);
        let factor = if scroll_y > 0.0 { 1.15 } else { 1.0 / 1.15 };
        viewport.pixels_per_meter = (viewport.pixels_per_meter * factor).clamp(1.0, 500.0);
        let (wx_after, wy_after) = viewport.screen_to_world(rel_sx, rel_sy, w, h, iso8855);
        viewport.center_x_m += wx_before - wx_after;
        viewport.center_y_m += wy_before - wy_after;
        changed = true;
    }
    changed
}

/// Formats the hover tooltip for the 2D canvas given the cursor position and plotted points.
pub fn format_canvas_hover_tooltip(
    hover_pos: Pos2,
    wx: f64,
    wy: f64,
    plotted_screen_points: &[(Pos2, String, f64, f64)],
) -> String {
    let nearest = plotted_screen_points
        .iter()
        .map(|(pt, name, x, y)| (pt.distance(hover_pos), name, *x, *y))
        .filter(|(d, _, _, _)| *d <= 16.0)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    if let Some((_, name, px, py)) = nearest {
        format!(
            "{}\nx = {:.3} m, y = {:.3} m\n(Cursor: {:.2} m, {:.2} m)",
            name, px, py, wx, wy
        )
    } else {
        format!("x = {:.2} m, y = {:.2} m", wx, wy)
    }
}

/// Adds a new [`XyPlotLayerConfig`] from the current builder fields on [`AppController`].
pub fn add_xy_layer_from_inputs(controller: &mut AppController) -> bool {
    if controller.new_layer_x_signal.trim().is_empty()
        || controller.new_layer_y_signal.trim().is_empty()
    {
        return false;
    }
    let name = if controller.new_layer_name.trim().is_empty() {
        format!(
            "{} vs {}",
            controller.new_layer_x_signal, controller.new_layer_y_signal
        )
    } else {
        controller.new_layer_name.trim().to_string()
    };
    controller
        .settings
        .visualization
        .xy_layers
        .push(XyPlotLayerConfig {
            name,
            enabled: true,
            x_signal: controller.new_layer_x_signal.trim().to_string(),
            y_signal: controller.new_layer_y_signal.trim().to_string(),
            valid_count_signal: controller.new_layer_valid_count_signal.trim().to_string(),
            corner_signals: controller.new_layer_corners.clone(),
            style: controller.new_layer_style,
            color_rgb: controller.new_layer_color_rgb,
            point_radius: 5.5,
        });
    controller.sync_video_scene_to_worker();
    true
}

fn render_layer_sidebar(ui: &mut Ui, controller: &mut AppController) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(RichText::new("2D View & Ego Vehicle").strong());
        let mut scene_changed = false;

        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut controller.settings.visualization.ego.show_ego,
                    "Show Ego",
                )
                .changed()
            {
                scene_changed = true;
            }
            if ui
                .checkbox(
                    &mut controller.settings.visualization.ego.iso8855_axes,
                    "ISO 8855 (+X Up, +Y Left)",
                )
                .changed()
            {
                scene_changed = true;
            }
        });

        ui.horizontal(|ui| {
            if ui.button("↺ Reset View").clicked() {
                controller.plane_viewport = PlaneViewport::default();
                scene_changed = true;
            }
            if ui.button("🧹 Clear Trails").clicked() {
                controller.trails_by_layer.clear();
            }
        });

        let ego = &controller.settings.visualization.ego;
        ui.weak(format!(
            "Ego: {:.2}m × {:.2}m (RearAxle→Center: {:+.2}m)",
            ego.length_m, ego.width_m, ego.rear_axle_to_center_m
        ));

        ui.separator();
        ui.heading(RichText::new("Active (X, Y) Signal Layers").strong());

        let mut remove_idx: Option<usize> = None;
        for (idx, layer) in controller
            .settings
            .visualization
            .xy_layers
            .iter_mut()
            .enumerate()
        {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut layer.enabled, "").changed() {
                        scene_changed = true;
                    }
                    if ui.color_edit_button_srgb(&mut layer.color_rgb).changed() {
                        scene_changed = true;
                    }
                    ui.label(RichText::new(&layer.name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✖").on_hover_text("Remove layer").clicked() {
                            remove_idx = Some(idx);
                        }
                        ui.weak(layer.style.label());
                    });
                });
                ui.monospace(format!("X: {}", layer.x_signal));
                ui.monospace(format!("Y: {}", layer.y_signal));
                if !layer.valid_count_signal.is_empty() {
                    ui.weak(format!("Valid N: {}", layer.valid_count_signal));
                }
            });
        }

        if let Some(idx) = remove_idx {
            controller.settings.visualization.xy_layers.remove(idx);
            scene_changed = true;
        }

        ui.separator();
        ui.heading(RichText::new("Pair (X, Y) Signals").strong());
        ui.label("Filter A2L Signals:");
        ui.text_edit_singleline(&mut controller.layer_signal_search_filter);

        let filter_lower = controller.layer_signal_search_filter.to_ascii_lowercase();
        let filtered_signals: Vec<(String, String)> = controller
            .a2l_model
            .as_ref()
            .map(|m| {
                m.measurements
                    .iter()
                    .filter(|meas| {
                        filter_lower.is_empty()
                            || meas.name.to_ascii_lowercase().contains(&filter_lower)
                            || meas
                                .description
                                .to_ascii_lowercase()
                                .contains(&filter_lower)
                    })
                    .map(|meas| {
                        let dim_tag = if meas.matrix_dim.is_empty() {
                            "scalar".to_string()
                        } else {
                            format!("{:?}", meas.matrix_dim)
                        };
                        (meas.name.clone(), format!("{} ({})", meas.name, dim_tag))
                    })
                    .collect()
            })
            .unwrap_or_default();

        egui::Grid::new("new_xy_layer_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Layer Name:");
                ui.text_edit_singleline(&mut controller.new_layer_name);
                ui.end_row();

                ui.label("Style:");
                ui.horizontal_wrapped(|ui| {
                    for st in [
                        XyDrawStyle::Scatter,
                        XyDrawStyle::Line,
                        XyDrawStyle::TrajectoryTrail,
                        XyDrawStyle::Polygon4Corner,
                    ] {
                        ui.selectable_value(&mut controller.new_layer_style, st, st.label());
                    }
                });
                ui.end_row();

                ui.label("Color:");
                ui.color_edit_button_srgb(&mut controller.new_layer_color_rgb);
                ui.end_row();

                ui.label("X Signal:");
                ui.text_edit_singleline(&mut controller.new_layer_x_signal);
                ui.end_row();

                ui.label("Y Signal:");
                ui.text_edit_singleline(&mut controller.new_layer_y_signal);
                ui.end_row();

                ui.label("Valid Count:");
                ui.text_edit_singleline(&mut controller.new_layer_valid_count_signal);
                ui.end_row();
            });

        // Quick-pick list from filtered A2L signals (works with both scalars and vectors)
        if !filtered_signals.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("xy_signal_quick_picker")
                .max_height(110.0)
                .show(ui, |ui| {
                    for (sig_name, sig_label) in filtered_signals.iter().take(25) {
                        ui.horizontal(|ui| {
                            if ui.small_button("Set X").clicked() {
                                controller.new_layer_x_signal = sig_name.clone();
                            }
                            if ui.small_button("Set Y").clicked() {
                                controller.new_layer_y_signal = sig_name.clone();
                            }
                            ui.monospace(sig_label);
                        });
                    }
                });
        }

        if controller.new_layer_style == XyDrawStyle::Polygon4Corner {
            egui::CollapsingHeader::new("Optional 4-Corner Signals (P0..P3)")
                .default_open(true)
                .show(ui, |ui| {
                    let labels = [
                        "P0.X", "P0.Y", "P1.X", "P1.Y", "P2.X", "P2.Y", "P3.X", "P3.Y",
                    ];
                    for (i, lbl) in labels.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(*lbl);
                            ui.text_edit_singleline(&mut controller.new_layer_corners[i]);
                        });
                    }
                });
        }

        if ui
            .button(RichText::new("➕ Add (X, Y) Layer to 2D Plot").strong())
            .clicked()
        {
            add_xy_layer_from_inputs(controller);
        }

        if scene_changed {
            controller.sync_video_scene_to_worker();
        }
    });
}

fn render_plane_canvas(ui: &mut Ui, controller: &mut AppController) {
    let avail = ui.available_size();
    let canvas_size = Vec2::new(avail.x.max(200.0), avail.y.max(200.0));
    let (response, painter) = ui.allocate_painter(canvas_size, Sense::click_and_drag());
    let rect = response.rect;

    painter.rect_filled(rect, 4.0, Color32::from_rgb(15, 20, 28));

    let iso8855 = controller.settings.visualization.ego.iso8855_axes;
    let w = rect.width() as f64;
    let h = rect.height() as f64;

    if response.double_clicked() {
        controller.plane_viewport = PlaneViewport::default();
        controller.sync_video_scene_to_worker();
    }

    let drag_delta = if response.dragged_by(egui::PointerButton::Primary) {
        response.drag_delta()
    } else {
        Vec2::ZERO
    };
    let scroll_y = if response.hovered() {
        ui.input(|i| i.smooth_scroll_delta.y)
    } else {
        0.0
    };
    let pointer_pos = response.hover_pos().unwrap_or_else(|| rect.center());
    let rel_sx = (pointer_pos.x - rect.left()) as f64;
    let rel_sy = (pointer_pos.y - rect.top()) as f64;
    if apply_canvas_zoom_and_pan(
        &mut controller.plane_viewport,
        drag_delta,
        scroll_y,
        rel_sx,
        rel_sy,
        w,
        h,
        iso8855,
    ) {
        controller.sync_video_scene_to_worker();
    }

    let vp = controller.plane_viewport;
    let to_pos2 = |x_m: f64, y_m: f64| -> Pos2 {
        let (sx, sy) = vp.world_to_screen(x_m, y_m, w, h, iso8855);
        Pos2::new(rect.left() + (sx as f32), rect.top() + (sy as f32))
    };

    let (x_min, x_max, y_min, y_max) = vp.world_bounds(w, h, iso8855);
    let (x_minor, x_major) = grid_steps(x_max - x_min, if iso8855 { h } else { w });
    let (y_minor, y_major) = grid_steps(y_max - y_min, if iso8855 { w } else { h });

    // Draw X grid lines + major tick labels
    for xv in ticks(x_min, x_max, x_minor) {
        let major = is_major(xv, x_major);
        let is_zero = xv.abs() < 1e-9;
        let stroke = if is_zero {
            Stroke::new(1.6_f32, Color32::from_rgb(75, 100, 132))
        } else if major {
            Stroke::new(1.0_f32, Color32::from_rgb(38, 52, 72))
        } else {
            Stroke::new(0.6_f32, Color32::from_rgb(23, 32, 45))
        };
        painter.line_segment([to_pos2(xv, y_min), to_pos2(xv, y_max)], stroke);
        if major {
            let lbl_pos = if iso8855 {
                let p = to_pos2(xv, y_max);
                Pos2::new(rect.left() + 8.0, p.y)
            } else {
                let p = to_pos2(xv, y_min);
                Pos2::new(p.x, rect.bottom() - 16.0)
            };
            painter.text(
                lbl_pos,
                Align2::LEFT_CENTER,
                format!("{} m", tick_text(xv, x_major)),
                FontId::monospace(10.5),
                Color32::from_rgb(148, 163, 184),
            );
        }
    }

    // Draw Y grid lines + major tick labels
    for yv in ticks(y_min, y_max, y_minor) {
        let major = is_major(yv, y_major);
        let is_zero = yv.abs() < 1e-9;
        let stroke = if is_zero {
            Stroke::new(1.6_f32, Color32::from_rgb(75, 100, 132))
        } else if major {
            Stroke::new(1.0_f32, Color32::from_rgb(38, 52, 72))
        } else {
            Stroke::new(0.6_f32, Color32::from_rgb(23, 32, 45))
        };
        painter.line_segment([to_pos2(x_min, yv), to_pos2(x_max, yv)], stroke);
        if major {
            let lbl_pos = if iso8855 {
                let p = to_pos2(x_min, yv);
                Pos2::new(p.x, rect.bottom() - 14.0)
            } else {
                let p = to_pos2(x_min, yv);
                Pos2::new(rect.left() + 8.0, p.y)
            };
            painter.text(
                lbl_pos,
                Align2::CENTER_CENTER,
                format!("{} m", tick_text(yv, y_major)),
                FontId::monospace(10.5),
                Color32::from_rgb(148, 163, 184),
            );
        }
    }

    // Draw Ego Vehicle footprint and rear-axle origin at (0, 0)
    let ego = &controller.settings.visualization.ego;
    if ego.show_ego {
        let x_front = ego.front_extent_m();
        let x_rear = ego.rear_extent_m();
        let hw = ego.half_width_m();
        let poly = vec![
            to_pos2(x_front, hw),
            to_pos2(x_front, -hw),
            to_pos2(x_rear, -hw),
            to_pos2(x_rear, hw),
        ];
        painter.add(egui::Shape::convex_polygon(
            poly,
            Color32::from_rgba_unmultiplied(148, 163, 184, 32),
            Stroke::new(1.8_f32, Color32::from_rgb(226, 232, 240)),
        ));
        let p_origin = to_pos2(0.0, 0.0);
        let p_nose = to_pos2(x_front, 0.0);
        painter.line_segment(
            [p_origin, p_nose],
            Stroke::new(1.5_f32, Color32::from_rgb(249, 115, 22)),
        );
        painter.circle_filled(p_origin, 4.0, Color32::from_rgb(249, 115, 22));
    }

    // Draw all enabled (X, Y) signal layers
    let mut plotted_screen_points: Vec<(Pos2, String, f64, f64)> = Vec::new();
    for layer in &controller.settings.visualization.xy_layers {
        if !layer.enabled {
            continue;
        }
        let color = Color32::from_rgb(layer.color_rgb[0], layer.color_rgb[1], layer.color_rgb[2]);
        let radius = layer.point_radius.clamp(2.0, 12.0);

        match layer.style {
            XyDrawStyle::Scatter => {
                for (i, (x, y)) in
                    extract_xy_points_for_layer(layer, &controller.latest_signals_by_name)
                        .into_iter()
                        .enumerate()
                {
                    let pt = to_pos2(x, y);
                    if rect.contains(pt) {
                        painter.circle_filled(pt, radius, color);
                        plotted_screen_points.push((pt, format!("{}[{}]", layer.name, i), x, y));
                    }
                }
            }
            XyDrawStyle::Line => {
                let pts_world =
                    extract_xy_points_for_layer(layer, &controller.latest_signals_by_name);
                let pts_screen: Vec<Pos2> = pts_world.iter().map(|&(x, y)| to_pos2(x, y)).collect();
                if pts_screen.len() >= 2 {
                    painter.add(egui::Shape::line(
                        pts_screen.clone(),
                        Stroke::new(2.0_f32, color),
                    ));
                }
                for (i, (&pt, &(x, y))) in pts_screen.iter().zip(pts_world.iter()).enumerate() {
                    if rect.contains(pt) {
                        painter.circle_filled(pt, (radius - 1.0).max(2.5), color);
                        plotted_screen_points.push((pt, format!("{}[{}]", layer.name, i), x, y));
                    }
                }
            }
            XyDrawStyle::TrajectoryTrail => {
                if let Some(trail) = controller.trails_by_layer.get(&layer.name) {
                    let trail_screen: Vec<Pos2> =
                        trail.iter().map(|&(x, y)| to_pos2(x, y)).collect();
                    if trail_screen.len() >= 2 {
                        painter.add(egui::Shape::line(
                            trail_screen,
                            Stroke::new(1.8_f32, color.gamma_multiply(0.65)),
                        ));
                    }
                }
                for (i, (x, y)) in
                    extract_xy_points_for_layer(layer, &controller.latest_signals_by_name)
                        .into_iter()
                        .enumerate()
                {
                    let pt = to_pos2(x, y);
                    if rect.contains(pt) {
                        painter.circle_filled(pt, radius, color);
                        plotted_screen_points.push((pt, format!("{}[{}]", layer.name, i), x, y));
                    }
                }
            }
            XyDrawStyle::Polygon4Corner => {
                for (i, poly) in
                    extract_polygons_for_layer(layer, &controller.latest_signals_by_name)
                        .into_iter()
                        .enumerate()
                {
                    let pts_screen: Vec<Pos2> = poly.iter().map(|&(x, y)| to_pos2(x, y)).collect();
                    painter.add(egui::Shape::convex_polygon(
                        pts_screen.clone(),
                        color.gamma_multiply(0.18),
                        Stroke::new(2.0_f32, color),
                    ));
                    for (&pt, &(x, y)) in pts_screen.iter().zip(poly.iter()) {
                        plotted_screen_points.push((pt, format!("{}#{}", layer.name, i), x, y));
                    }
                }
            }
        }
    }

    let axis_badge = if iso8855 {
        "ISO 8855 (+X Up, +Y Left)"
    } else {
        "Cartesian (+X Right, +Y Up)"
    };
    painter.text(
        Pos2::new(rect.right() - 12.0, rect.top() + 14.0),
        Align2::RIGHT_TOP,
        format!(
            "{} | Minor Grid: {:.2} m | Zoom: {:.1} px/m",
            axis_badge,
            x_minor.min(y_minor),
            vp.pixels_per_meter
        ),
        FontId::monospace(11.0),
        Color32::from_rgb(148, 163, 184),
    );

    if let Some(hover_pos) = response.hover_pos() {
        let (wx, wy) = vp.screen_to_world(
            (hover_pos.x - rect.left()) as f64,
            (hover_pos.y - rect.top()) as f64,
            w,
            h,
            iso8855,
        );
        let tooltip_text = format_canvas_hover_tooltip(hover_pos, wx, wy, &plotted_screen_points);
        response.on_hover_text(tooltip_text);
    }
}
