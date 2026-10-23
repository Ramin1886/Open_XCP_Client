// SPDX-License-Identifier: GPL-2.0-only
//! Visual theme and styling for `Open_XCP_Client`.

use egui::{Color32, Context, Visuals};

/// Applies a high-contrast engineering theme (dark or light) to the `egui::Context`.
pub fn apply_theme(ctx: &Context, dark_mode: bool) {
    let mut visuals = if dark_mode {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.override_text_color = None;
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(4.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(4.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(4.0);
    visuals.widgets.active.rounding = egui::Rounding::same(4.0);
    ctx.set_visuals(visuals);
}

pub const COLOR_CONNECTED: Color32 = Color32::from_rgb(46, 204, 113);
pub const COLOR_DISCONNECTED: Color32 = Color32::from_rgb(189, 195, 199);
pub const COLOR_RECORDING: Color32 = Color32::from_rgb(231, 76, 60);
pub const COLOR_READY_START: Color32 = Color32::from_rgb(39, 174, 96);
pub const COLOR_WARNING: Color32 = Color32::from_rgb(243, 156, 18);
