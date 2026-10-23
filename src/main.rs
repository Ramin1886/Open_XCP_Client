// SPDX-License-Identifier: GPL-2.0-only
//! Binary entry point for `open_xcp_client`.

use open_xcp_client::app_state::AppController;
use open_xcp_client::config_store::ConfigStore;
use open_xcp_client::ui::OpenXcpClientApp;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("open_xcp_client {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--headless-self-check") {
        let store = ConfigStore::new(ConfigStore::default_path());
        let controller = AppController::new(store);
        let mut app = OpenXcpClientApp::new(controller);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render_frame(ctx);
        });
        println!("Headless UI self-check completed successfully.");
        return Ok(());
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Open_XCP_Client — Lightweight XCP Logging (.sdf) & Calibration")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([860.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Open_XCP_Client",
        native_options,
        Box::new(|_cc| {
            let store = ConfigStore::new(ConfigStore::default_path());
            let controller = AppController::new(store);
            Ok(Box::new(OpenXcpClientApp::new(controller)))
        }),
    )?;

    Ok(())
}
