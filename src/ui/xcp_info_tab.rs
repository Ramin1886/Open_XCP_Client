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
use egui::{Color32, RichText, ScrollArea, Ui};

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
            render_link_integrity(ui, controller);
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
                        .num_columns(11)
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
                                "Discarded",
                                "No ODT 0",
                                "ODT Seq Err",
                            ] {
                                ui.label(RichText::new(hdr).strong());
                            }
                            ui.end_row();

                            for ev in controller.event_telemetry.values() {
                                ui.monospace(format!("0x{:04X} ({})", ev.event_channel_id, ev.event_channel_id));
                                ui.label(RichText::new(&ev.event_name).monospace().strong());
                                ui.monospace(format!("{:.2}", ev.configured_cycle_ms));
                                // Flag a measured cycle that has drifted away from the A2L value,
                                // instead of leaving the comparison to the reader's eye.
                                match ev.mean_cycle_ms() {
                                    Some(mean) => {
                                        let drifted = ev.configured_cycle_ms > 0.0
                                            && (mean - ev.configured_cycle_ms).abs()
                                                > ev.configured_cycle_ms
                                                    * CYCLE_DEVIATION_WARN_FRACTION;
                                        let text = RichText::new(format!("{:.2}", mean)).monospace();
                                        ui.label(if drifted {
                                            text.color(LOSS_WARN_COLOR).strong()
                                        } else {
                                            text
                                        })
                                        .on_hover_text(if drifted {
                                            "Measured cycle deviates from the A2L cycle time by more than 25%"
                                        } else {
                                            "Measured cycle matches the A2L cycle time"
                                        });
                                    }
                                    None => {
                                        ui.monospace("—");
                                    }
                                }
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
                                let loss = controller
                                    .event_loss
                                    .get(&ev.event_channel_id)
                                    .copied()
                                    .unwrap_or_default();
                                loss_cell(ui, loss.cycles_discarded);
                                loss_cell(ui, loss.cycles_lost_no_header);
                                loss_cell(ui, loss.odt_sequence_errors);
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

/// Amber used for any non-zero loss figure.
const LOSS_WARN_COLOR: Color32 = Color32::from_rgb(251, 191, 36);

/// Relative deviation between measured and A2L cycle time that earns a warning.
const CYCLE_DEVIATION_WARN_FRACTION: f64 = 0.25;

/// Renders one integrity count, amber when non-zero so loss cannot be skimmed past.
fn loss_cell(ui: &mut Ui, value: u64) {
    let text = RichText::new(format!("{}", value)).monospace();
    ui.label(if value > 0 {
        text.color(LOSS_WARN_COLOR).strong()
    } else {
        text
    });
}

/// Renders the link-integrity section: CTR continuity, queue drops and recent discarded cycles.
///
/// Without these figures a dropped UDP datagram is invisible, and a recording taken over a lossy
/// link is indistinguishable from a complete one.
pub fn render_link_integrity(ui: &mut Ui, controller: &AppController) {
    ui.group(|ui| {
        let clean = controller.daq_integrity_is_clean();
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Link & DAQ Integrity").strong());
            ui.label(if clean {
                RichText::new("✔ no loss observed").strong()
            } else {
                RichText::new("⚠ loss observed")
                    .color(LOSS_WARN_COLOR)
                    .strong()
            });
        });
        ui.weak(
            "Derived from the 16-bit XCP-on-Ethernet CTR of every received frame, plus per-DAQ-list              cycle completeness. Incomplete cycles are discarded, never recorded.",
        );
        ui.separator();

        let link = &controller.link_stats;
        egui::Grid::new("xcp_link_integrity_grid")
            .num_columns(2)
            .spacing([18.0, 4.0])
            .show(ui, |ui| {
                ui.label("Frames received:");
                ui.monospace(format!("{}", link.frames_received));
                ui.end_row();

                ui.label("CTR gaps / frames lost:");
                ui.horizontal(|ui| {
                    loss_cell(ui, link.gaps_detected);
                    ui.weak("/");
                    loss_cell(ui, link.frames_lost_estimate);
                });
                ui.end_row();

                ui.label("Reordered / duplicated:");
                ui.horizontal(|ui| {
                    loss_cell(ui, link.reordered);
                    ui.weak("/");
                    loss_cell(ui, link.duplicated);
                });
                ui.end_row();

                ui.label("DAQ queue overflow drops:");
                loss_cell(ui, link.channel_overflow_drops);
                ui.end_row();

                ui.label("Unplanned DTOs:");
                loss_cell(ui, link.unplanned_dto);
                ui.end_row();
            });

        if !controller.recent_cycle_discards.is_empty() {
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "Recent discarded cycles ({})",
                    controller.recent_cycle_discards.len()
                ))
                .strong(),
            );
            egui::ScrollArea::vertical()
                .id_salt("xcp_recent_discards")
                .max_height(110.0)
                .show(ui, |ui| {
                    for (host_s, discard) in controller.recent_cycle_discards.iter().rev() {
                        ui.monospace(format!(
                            "{:.3}  event 0x{:04X}  DAQ {}  — {}",
                            host_s,
                            discard.event_channel_id,
                            discard.daq_list_num,
                            discard.reason.label()
                        ));
                    }
                });
        }
    });
}
