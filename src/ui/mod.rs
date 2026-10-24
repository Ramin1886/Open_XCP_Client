// SPDX-License-Identifier: GPL-2.0-only
//! Main `eframe::App` GUI implementation integrating the Top Bar (`Live`/`Replay` switch,
//! Live Recording Bar, `.sdf` Replay Bar, 5-tab selector), Settings Menu modal,
//! `2D Live Plot` Tab, `Numerical` Tab, `Time Plot` Tab, `Calibration` Tab, `XCP Info` Tab,
//! reusable Plot Profile selector,
//! and Status Footer.

pub mod axis_ticks;
pub mod calibration_tab;
pub mod numerical_tab;
pub mod plane_plot_tab;
pub mod profile_panel;
pub mod settings_menu;
pub mod theme;
pub mod time_plot_tab;
pub mod top_bar;
pub mod xcp_info_tab;

use crate::app_state::{ActiveTab, AppController, AppMode};
use calibration_tab::render_calibration_tab;
use egui::Context;
use numerical_tab::render_numerical_tab;
use plane_plot_tab::render_plane_plot_tab;
use settings_menu::render_settings_menu;
use std::time::{Duration, Instant};
use theme::apply_theme;
use time_plot_tab::render_time_plot_tab;
use top_bar::render_top_bar;
use xcp_info_tab::render_xcp_info_tab;

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

        if self.controller.mode == AppMode::Replay {
            let now = Instant::now();
            let dt = now
                .duration_since(self.controller.last_replay_tick)
                .as_secs_f64();
            self.controller.last_replay_tick = now;
            self.controller.tick_replay(dt);
        }

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
            ActiveTab::PlanePlot => render_plane_plot_tab(ui, &mut self.controller),
            ActiveTab::Numerical => render_numerical_tab(ui, &mut self.controller),
            ActiveTab::TimePlot => render_time_plot_tab(ui, &mut self.controller),
            ActiveTab::Calibration => render_calibration_tab(ui, &mut self.controller),
            ActiveTab::XcpInfo => render_xcp_info_tab(ui, &mut self.controller),
        });

        render_settings_menu(ctx, &mut self.controller);

        let replay_playing = self
            .controller
            .replay_player
            .as_ref()
            .map(|p| p.is_playing())
            .unwrap_or(false);
        if self.controller.is_recording || self.controller.connected || replay_playing {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

impl eframe::App for OpenXcpClientApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.render_frame(ctx);
    }
}
