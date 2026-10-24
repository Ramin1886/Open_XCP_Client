// SPDX-License-Identifier: GPL-2.0-only
//! `XCP Info` Tab (`src/ui/xcp_info_tab.rs`).
//!
//! Displays standard XCP connection and A2L metadata (Target Endpoint, Transport,
//! Project/Module name, UTC clock synchronization status, signal counts), a live
//! per-`EVENT` DAQ telemetry table (Configured Cycle `ms`, Measured Mean Cycle `ms`,
//! Measured Max Cycle `ms`, Sample Count, Latest UTC Timestamp, Age `ms`), and
//! Recording / Replay file status.

use crate::app_state::{AppController, AppMode};
use crate::ui::theme::{COLOR_CONNECTED, COLOR_DISCONNECTED, COLOR_RECORDING};
use egui::{RichText, ScrollArea, Ui};

/// Renders the `XCP Info` tab (`ActiveTab::XcpInfo`).
pub fn render_xcp_info_tab(ui: &mut Ui, controller: &mut AppController) {
    ScrollArea::vertical()
        .id_salt("xcp_info_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Section 1: XCP Target & A2L Metadata
            ui.group(|ui| {
                ui.heading(RichText::new("XCP Target & A2L Metadata").strong());
                ui.separator();

                egui::Grid::new("xcp_metadata_grid")
                    .num_columns(4)
                    .spacing([24.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Operating Mode:").strong());
                        ui.label(match controller.mode {
                            AppMode::Live => "Live XCP Target Streaming",
                            AppMode::Replay => "Offline .sdf Recording Replay",
                        });
                        ui.label(RichText::new("Connection State:").strong());
                        if controller.connected {
                            ui.colored_label(
                                COLOR_CONNECTED,
                                format!("Connected ({})", controller.target_endpoint),
                            );
                        } else {
                            ui.colored_label(
                                COLOR_DISCONNECTED,
                                format!(
                                    "Disconnected ({}:{} {})",
                                    controller.settings.connection.target_ip,
                                    controller.settings.connection.target_port,
                                    controller.settings.connection.transport.as_str()
                                ),
                            );
                        }
                        ui.end_row();

                        let (proj, module, meas_n, cal_n, ev_n) =
                            if let Some(m) = &controller.a2l_model {
                                (
                                    m.project_name.as_str(),
                                    m.module_name.as_str(),
                                    m.measurements.len(),
                                    m.characteristics.len(),
                                    m.events.len(),
                                )
                            } else {
                                ("—", "—", 0, 0, 0)
                            };

                        ui.label(RichText::new("A2L Project / Module:").strong());
                        ui.monospace(format!("{} / {}", proj, module));
                        ui.label(RichText::new("Clock Correlation:").strong());
                        ui.label(if controller.target_native_utc {
                            "Target Native UTC Clock (GET_DAQ_CLOCK)"
                        } else {
                            "Host-Anchored UTC Clock Correlation"
                        });
                        ui.end_row();

                        ui.label(RichText::new("Resolved Measurements:").strong());
                        ui.monospace(format!("{} signals", meas_n));
                        ui.label(RichText::new("Calibration Parameters:").strong());
                        ui.monospace(format!("{} characteristics ({} events)", cal_n, ev_n));
                        ui.end_row();
                    });
            });

            ui.add_space(8.0);

            // Section 2: Per-Event DAQ Telemetry Table
            ui.group(|ui| {
                ui.heading(RichText::new("XCP DAQ Event Channel Telemetry").strong());
                ui.weak("Live arrival statistics computed over a rolling 5-second window.");
                ui.separator();

                if controller.event_telemetry.is_empty() {
                    ui.label("No XCP Event channels active yet. Connect to target or load an .sdf recording.");
                } else {
                    egui::Grid::new("xcp_event_telemetry_grid")
                        .num_columns(8)
                        .spacing([18.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for hdr in [
                                "Event ID",
                                "Event Name",
                                "A2L Cycle (ms)",
                                "Mean Cycle (ms)",
                                "Max Cycle (ms)",
                                "Samples",
                                "Latest UTC (s)",
                                "Age (ms)",
                            ] {
                                ui.label(RichText::new(hdr).strong());
                            }
                            ui.end_row();

                            for ev in controller.event_telemetry.values() {
                                ui.monospace(format!("0x{:04X} ({})", ev.event_channel_id, ev.event_channel_id));
                                ui.label(RichText::new(&ev.event_name).monospace().strong());
                                ui.monospace(format!("{:.2}", ev.configured_cycle_ms));
                                ui.monospace(
                                    ev.mean_cycle_ms()
                                        .map(|v| format!("{:.2}", v))
                                        .unwrap_or_else(|| "—".to_string()),
                                );
                                ui.monospace(
                                    ev.max_cycle_ms()
                                        .map(|v| format!("{:.2}", v))
                                        .unwrap_or_else(|| "—".to_string()),
                                );
                                ui.monospace(format!("{}", ev.received_count));
                                ui.monospace(if ev.last_utc_s > 0.0 {
                                    format!("{:.3}", ev.last_utc_s)
                                } else {
                                    "—".to_string()
                                });
                                ui.monospace(
                                    ev.age_ms()
                                        .map(|v| format!("{:.0}", v))
                                        .unwrap_or_else(|| "—".to_string()),
                                );
                                ui.end_row();
                            }
                        });
                }
            });

            ui.add_space(8.0);

            // Section 3: Recording (.sdf + .mp4) & Replay Status
            ui.group(|ui| {
                ui.heading(RichText::new("Recording (.sdf + .mp4) & Replay Session").strong());
                ui.separator();

                if controller.is_recording {
                    ui.colored_label(
                        COLOR_RECORDING,
                        format!(
                            "● Active Recording — PART{:03} ({} total event samples)",
                            controller.active_part_number, controller.total_samples_recorded
                        ),
                    );
                    if let Some(sdf) = &controller.active_recording_file {
                        ui.monospace(format!("Active SDF: {}", sdf.display()));
                    }
                    if let Some(mp4) = &controller.active_video_file {
                        ui.monospace(format!("Active MP4: {}", mp4.display()));
                    }
                } else {
                    ui.label("Recording Status: Idle");
                }

                if !controller.completed_files.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "Finalized .sdf Part Files ({}):",
                            controller.completed_files.len()
                        ))
                        .strong(),
                    );
                    for f in &controller.completed_files {
                        ui.monospace(format!("  • {}", f.display()));
                    }
                }
                if !controller.completed_video_files.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "Finalized .mp4 Video Part Files ({}):",
                            controller.completed_video_files.len()
                        ))
                        .strong(),
                    );
                    for f in &controller.completed_video_files {
                        ui.monospace(format!("  • {}", f.display()));
                    }
                }
            });
        });
}
