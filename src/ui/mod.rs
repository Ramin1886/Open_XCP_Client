// SPDX-License-Identifier: GPL-2.0-only
//! Main `eframe::App` GUI implementation integrating the Top Bar, Settings Menu modal,
//! Logging Tab, Calibration Tab, and Status Footer.

pub mod calibration_tab;
pub mod logging_tab;
pub mod settings_menu;
pub mod theme;
pub mod top_bar;

use crate::app_state::{ActiveTab, AppController};
use calibration_tab::render_calibration_tab;
use egui::Context;
use logging_tab::render_logging_tab;
use settings_menu::render_settings_menu;
use std::time::Duration;
use theme::apply_theme;
use top_bar::render_top_bar;

/// Top-level `eframe::App` wrapper around [`AppController`].
pub struct OpenXcpClientApp {
    pub controller: AppController,
}

impl OpenXcpClientApp {
    /// Creates a new `OpenXcpClientApp` with the given [`AppController`].
    pub fn new(controller: AppController) -> Self {
        Self { controller }
    }

    /// Executes a single UI update pass on any `egui::Context` (used by both `eframe`
    /// and headless UI unit tests).
    pub fn render_frame(&mut self, ctx: &Context) {
        self.controller.poll_worker_events();
        apply_theme(ctx, self.controller.settings.ui.dark_mode);

        egui::TopBottomPanel::top("top_navigation_bar").show(ctx, |ui| {
            render_top_bar(ui, &mut self.controller);
        });

        egui::TopBottomPanel::bottom("bottom_status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.controller.status_message);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.controller.active_tab {
            ActiveTab::Logging => render_logging_tab(ui, &mut self.controller),
            ActiveTab::Calibration => render_calibration_tab(ui, &mut self.controller),
        });

        render_settings_menu(ctx, &mut self.controller);

        if self.controller.is_recording || self.controller.connected {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

impl eframe::App for OpenXcpClientApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_frame(ctx);
    }
}
