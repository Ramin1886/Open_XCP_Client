// SPDX-License-Identifier: GPL-2.0-only
//! Application controller state, background worker command/event channels,
//! and coordination between the UI, XCP network engine, and `.sdf` recorder.

use crate::a2l::model::{A2lModel, Characteristic};
use crate::a2l::parser::parse_a2l;
use crate::calibration::manager::CalibrationManager;
use crate::config_store::{AppSettings, ConfigStore};
use crate::mock_server::MockXcpServer;
use crate::recording::file_namer::format_utc_for_filename;
use crate::recording::sdf_writer::SdfSessionRecorder;
use crate::xcp::client::XcpClient;
use crate::xcp::daq_planner::{DaqEventSample, DaqPlanner, DecodedValue};
use chrono::Utc;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Active primary tab in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Logging,
    Calibration,
}

/// Commands dispatched from the UI thread to the background XCP/Recording worker.
#[derive(Debug, Clone)]
pub enum WorkerCommand {
    Connect {
        settings: AppSettings,
    },
    Disconnect,
    StartRecording {
        folder: String,
        prefix: String,
        max_size_bytes: u64,
    },
    StopRecording,
    ReadCharacteristic {
        characteristic: Characteristic,
    },
    WriteCharacteristic {
        characteristic: Characteristic,
        raw_bytes: Vec<u8>,
        verify_readback: bool,
    },
    SetCalPage {
        page: u8,
    },
    Shutdown,
}

/// Events emitted from the background worker back to the UI thread.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Connected {
        target_endpoint: String,
        a2l_model: Box<A2lModel>,
        target_native_utc: bool,
    },
    Disconnected {
        reason: String,
    },
    RecordingStarted {
        active_file: PathBuf,
        start_utc_human: String,
    },
    RecordingProgress {
        active_file: PathBuf,
        part_number: u32,
        current_part_bytes: u64,
        max_part_bytes: u64,
        total_samples: u64,
        latest_sample: DaqEventSample,
    },
    RecordingPartSplit {
        completed_file: PathBuf,
        new_active_file: PathBuf,
        new_part_number: u32,
    },
    RecordingStopped {
        completed_files: Vec<PathBuf>,
    },
    CharacteristicValueRead {
        name: String,
        raw_bytes: Vec<u8>,
    },
    CharacteristicWriteFailed {
        name: String,
        error: String,
    },
    StatusMessage(String),
}

/// Shared application controller state manipulated by the UI and tests.
pub struct AppController {
    pub config_store: ConfigStore,
    pub settings: AppSettings,
    pub active_tab: ActiveTab,
    pub settings_menu_open: bool,
    pub connected: bool,
    pub target_endpoint: String,
    pub target_native_utc: bool,
    pub a2l_model: Option<A2lModel>,

    // Logging tab fields
    pub log_folder_input: String,
    pub log_prefix_input: String,
    pub is_recording: bool,
    pub active_recording_file: Option<PathBuf>,
    pub active_part_number: u32,
    pub current_part_bytes: u64,
    pub max_part_bytes: u64,
    pub total_samples_recorded: u64,
    pub completed_files: Vec<PathBuf>,
    pub latest_measurement_values: BTreeMap<usize, DecodedValue>,
    pub measurement_search_filter: String,

    // Calibration tab fields
    pub calibration_search_filter: String,
    pub calibration_manager: CalibrationManager,

    // Status bar
    pub status_message: String,

    cmd_tx: Sender<WorkerCommand>,
    evt_rx: Receiver<WorkerEvent>,
}

impl AppController {
    /// Creates a new `AppController` backed by `config_store` and spawns the background worker thread.
    pub fn new(config_store: ConfigStore) -> Self {
        let settings = config_store.load_or_default().unwrap_or_default();
        let (cmd_tx, cmd_rx) = unbounded::<WorkerCommand>();
        let (evt_tx, evt_rx) = unbounded::<WorkerEvent>();

        std::thread::Builder::new()
            .name("open-xcp-worker".to_string())
            .spawn(move || run_background_worker(cmd_rx, evt_tx))
            .expect("Failed to spawn background XCP worker thread");

        let log_folder_input = settings.recording.default_log_folder.clone();
        let log_prefix_input = settings.recording.default_file_prefix.clone();
        let max_part_bytes = settings.max_logfile_size_bytes();

        Self {
            config_store,
            settings,
            active_tab: ActiveTab::Logging,
            settings_menu_open: false,
            connected: false,
            target_endpoint: String::new(),
            target_native_utc: false,
            a2l_model: None,
            log_folder_input,
            log_prefix_input,
            is_recording: false,
            active_recording_file: None,
            active_part_number: 1,
            current_part_bytes: 0,
            max_part_bytes,
            total_samples_recorded: 0,
            completed_files: Vec::new(),
            latest_measurement_values: BTreeMap::new(),
            measurement_search_filter: String::new(),
            calibration_search_filter: String::new(),
            calibration_manager: CalibrationManager::new(),
            status_message: "Ready — Configure target in Settings or click Connect.".to_string(),
            cmd_tx,
            evt_rx,
        }
    }

    /// Returns a live preview of the next logfile name (`<Prefix>_<UTC>_PART001.sdf`).
    pub fn preview_next_logfile_name(&self) -> String {
        let now_str = format_utc_for_filename(Utc::now());
        let prefix = crate::recording::file_namer::sanitize_prefix(&self.log_prefix_input);
        format!("{}_{}_PART001.sdf", prefix, now_str)
    }

    /// Saves the current [`AppSettings`] to `./config/settings.json` and synchronizes
    /// defaults into the active state.
    pub fn save_settings(&mut self) -> Result<(), String> {
        self.settings.calibration.watch_list = self.calibration_manager.watch_list_names();
        self.config_store
            .save(&self.settings)
            .map_err(|e| e.to_string())?;
        self.max_part_bytes = self.settings.max_logfile_size_bytes();
        self.status_message = format!(
            "Settings saved to {} (Max Split Size: {} MB)",
            self.config_store.path().display(),
            self.settings.recording.max_logfile_size_mb
        );
        Ok(())
    }

    /// Requests connection to the configured XCP target (or built-in mock server).
    pub fn request_connect(&mut self) {
        let _ = self.cmd_tx.send(WorkerCommand::Connect {
            settings: self.settings.clone(),
        });
        self.status_message = format!(
            "Connecting to {}:{} ({})...",
            self.settings.connection.target_ip,
            self.settings.connection.target_port,
            self.settings.connection.transport.as_str()
        );
    }

    /// Requests disconnection from the target.
    pub fn request_disconnect(&mut self) {
        let _ = self.cmd_tx.send(WorkerCommand::Disconnect);
    }

    /// Toggles Start / Stop Recording.
    pub fn toggle_recording(&mut self) {
        if self.is_recording {
            let _ = self.cmd_tx.send(WorkerCommand::StopRecording);
        } else {
            let max_bytes = self.settings.max_logfile_size_bytes();
            self.max_part_bytes = max_bytes;
            let _ = self.cmd_tx.send(WorkerCommand::StartRecording {
                folder: self.log_folder_input.clone(),
                prefix: self.log_prefix_input.clone(),
                max_size_bytes: max_bytes,
            });
        }
    }

    /// Adds a calibration parameter by name to the active Calibration Content list and
    /// immediately triggers a target read (`SHORT_UPLOAD`/`UPLOAD`) if connected.
    pub fn add_calibration_variable(&mut self, name: &str) {
        if let Some(model) = &self.a2l_model {
            if self.calibration_manager.add_by_name(name, model) {
                self.settings.calibration.watch_list = self.calibration_manager.watch_list_names();
                let _ = self.config_store.save(&self.settings);
                if self.connected {
                    if let Some(entry) = self.calibration_manager.entries().get(name) {
                        let _ = self.cmd_tx.send(WorkerCommand::ReadCharacteristic {
                            characteristic: entry.characteristic.clone(),
                        });
                    }
                }
            }
        }
    }

    /// Refreshes all calibration variables in the active Calibration Content list from the target.
    pub fn refresh_all_calibrations(&self) {
        if !self.connected {
            return;
        }
        for entry in self.calibration_manager.entries().values() {
            let _ = self.cmd_tx.send(WorkerCommand::ReadCharacteristic {
                characteristic: entry.characteristic.clone(),
            });
        }
    }

    /// Validates and sends a modified calibration variable to the target when the user presses Enter.
    pub fn submit_calibration_on_enter(&mut self, name: &str) -> Result<(), String> {
        let Some(model) = &self.a2l_model else {
            return Err("No A2L model loaded".to_string());
        };
        match self.calibration_manager.prepare_write_on_enter(name, model) {
            Ok((characteristic, raw_bytes)) => {
                if self.connected {
                    let _ = self.cmd_tx.send(WorkerCommand::WriteCharacteristic {
                        characteristic,
                        raw_bytes,
                        verify_readback: self.settings.calibration.verify_readback,
                    });
                } else {
                    // Offline preview update when not connected to target
                    self.calibration_manager
                        .mark_synced_with_raw(name, model, &raw_bytes);
                }
                self.status_message = format!("Sent calibration update for '{}'", name);
                Ok(())
            }
            Err(err) => {
                let msg = err.to_string();
                self.status_message = format!("Calibration validation error: {}", msg);
                Err(msg)
            }
        }
    }

    /// Sends a command directly to the background worker thread.
    pub fn send_worker_command(&self, cmd: WorkerCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Polls all pending events from the background worker without blocking.
    pub fn poll_worker_events(&mut self) {
        while let Ok(evt) = self.evt_rx.try_recv() {
            self.handle_worker_event(evt);
        }
    }

    /// Processes a single [`WorkerEvent`] and updates controller state.
    pub fn handle_worker_event(&mut self, evt: WorkerEvent) {
        match evt {
            WorkerEvent::Connected {
                target_endpoint,
                a2l_model,
                target_native_utc,
            } => {
                self.connected = true;
                self.target_endpoint = target_endpoint.clone();
                self.target_native_utc = target_native_utc;
                let meas_count = a2l_model.measurements.len();
                let cal_count = a2l_model.characteristics.len();
                let ev_count = a2l_model.events.len();

                // Restore any saved watch_list items that exist in this A2L
                let saved_watch = self.settings.calibration.watch_list.clone();
                self.a2l_model = Some(*a2l_model);
                for name in saved_watch {
                    self.add_calibration_variable(&name);
                }
                self.refresh_all_calibrations();

                self.status_message = format!(
                    "Connected to {} — Loaded A2L ({} measurements, {} events, {} calibrations)",
                    target_endpoint, meas_count, ev_count, cal_count
                );
            }
            WorkerEvent::Disconnected { reason } => {
                self.connected = false;
                self.is_recording = false;
                self.status_message = format!("Disconnected: {}", reason);
            }
            WorkerEvent::RecordingStarted {
                active_file,
                start_utc_human,
            } => {
                self.is_recording = true;
                self.active_recording_file = Some(active_file.clone());
                self.active_part_number = 1;
                self.current_part_bytes = 0;
                self.total_samples_recorded = 0;
                self.status_message = format!(
                    "Recording started at {} -> {}",
                    start_utc_human,
                    active_file.display()
                );
            }
            WorkerEvent::RecordingProgress {
                active_file,
                part_number,
                current_part_bytes,
                max_part_bytes,
                total_samples,
                latest_sample,
            } => {
                self.active_recording_file = Some(active_file);
                self.active_part_number = part_number;
                self.current_part_bytes = current_part_bytes;
                self.max_part_bytes = max_part_bytes;
                self.total_samples_recorded = total_samples;
                for (m_idx, val) in latest_sample.values {
                    self.latest_measurement_values.insert(m_idx, val);
                }
            }
            WorkerEvent::RecordingPartSplit {
                completed_file,
                new_active_file,
                new_part_number,
            } => {
                self.completed_files.push(completed_file.clone());
                self.active_recording_file = Some(new_active_file.clone());
                self.active_part_number = new_part_number;
                self.status_message = format!(
                    "Split logfile at size limit -> rotated to PART{:03} ({})",
                    new_part_number,
                    new_active_file.display()
                );
            }
            WorkerEvent::RecordingStopped { completed_files } => {
                self.is_recording = false;
                self.completed_files = completed_files;
                self.status_message = format!(
                    "Recording stopped — {} .sdf file(s) finalized.",
                    self.completed_files.len()
                );
            }
            WorkerEvent::CharacteristicValueRead { name, raw_bytes } => {
                if let Some(model) = &self.a2l_model {
                    self.calibration_manager
                        .mark_synced_with_raw(&name, model, &raw_bytes);
                }
            }
            WorkerEvent::CharacteristicWriteFailed { name, error } => {
                self.calibration_manager.mark_error(&name, error.clone());
                self.status_message = format!("Failed to update '{}': {}", name, error);
            }
            WorkerEvent::StatusMessage(msg) => {
                self.status_message = msg;
            }
        }
    }
}

impl Drop for AppController {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WorkerCommand::Shutdown);
    }
}

fn run_background_worker(cmd_rx: Receiver<WorkerCommand>, evt_tx: Sender<WorkerEvent>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime for XCP worker");

    rt.block_on(async move {
        let mut mock_server: Option<MockXcpServer> = None;
        let mut active_client: Option<XcpClient> = None;
        let mut active_model: Option<A2lModel> = None;
        let mut active_planner: Option<DaqPlanner> = None;
        let mut active_clock_sync = crate::xcp::clock_sync::TargetClockSync::new(true);
        let mut daq_rx_opt: Option<
            tokio::sync::mpsc::Receiver<crate::xcp::transport::IncomingDaqDto>,
        > = None;
        let mut recorder_opt: Option<SdfSessionRecorder> = None;

        loop {
            // Process any pending UI commands without blocking DAQ
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    WorkerCommand::Connect { settings } => {
                        if let Some(mut rec) = recorder_opt.take() {
                            let _ = rec.finish();
                        }
                        if let Some(client) = active_client.take() {
                            let _ = client.disconnect().await;
                        }
                        mock_server = None;

                        let target_addr: SocketAddr = if settings.ui.enable_mock_server {
                            match MockXcpServer::start(0).await {
                                Ok(srv) => {
                                    let addr = match settings.connection.transport {
                                        crate::config_store::TransportProtocol::Udp => srv.udp_addr,
                                        crate::config_store::TransportProtocol::Tcp => srv.tcp_addr,
                                    };
                                    mock_server = Some(srv);
                                    addr
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::Disconnected {
                                        reason: format!("Failed to start mock server: {e}"),
                                    });
                                    continue;
                                }
                            }
                        } else {
                            match format!(
                                "{}:{}",
                                settings.connection.target_ip.trim(),
                                settings.connection.target_port
                            )
                            .parse()
                            {
                                Ok(a) => a,
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::Disconnected {
                                        reason: format!("Invalid target IP/port: {e}"),
                                    });
                                    continue;
                                }
                            }
                        };

                        let timeout =
                            Duration::from_millis(settings.connection.connect_timeout_ms.max(200));
                        match XcpClient::connect(
                            target_addr,
                            settings.connection.transport,
                            timeout,
                            settings.recording.prefer_target_utc_clock,
                        )
                        .await
                        {
                            Ok((client, clock_sync, daq_rx)) => {
                                let fallback = if settings
                                    .connection
                                    .local_a2l_fallback_path
                                    .trim()
                                    .is_empty()
                                {
                                    None
                                } else {
                                    Some(settings.connection.local_a2l_fallback_path.as_str())
                                };
                                match client.fetch_a2l(fallback).await {
                                    Ok(a2l_text) => match parse_a2l(&a2l_text) {
                                        Ok(model) => {
                                            let planner = DaqPlanner::from_a2l(
                                                &model,
                                                client.max_dto().saturating_sub(6),
                                            );
                                            let native_utc = clock_sync.is_target_native_utc();
                                            active_clock_sync = clock_sync;
                                            active_planner = Some(planner);
                                            active_model = Some(model.clone());
                                            active_client = Some(client);
                                            daq_rx_opt = Some(daq_rx);

                                            let _ = evt_tx.send(WorkerEvent::Connected {
                                                target_endpoint: format!(
                                                    "{} ({})",
                                                    target_addr,
                                                    settings.connection.transport.as_str()
                                                ),
                                                a2l_model: Box::new(model),
                                                target_native_utc: native_utc,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = evt_tx.send(WorkerEvent::Disconnected {
                                                reason: format!("A2L parse error: {e}"),
                                            });
                                        }
                                    },
                                    Err(e) => {
                                        let _ = evt_tx.send(WorkerEvent::Disconnected {
                                            reason: format!("A2L upload error: {e}"),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = evt_tx.send(WorkerEvent::Disconnected {
                                    reason: format!("XCP connection failed: {e}"),
                                });
                            }
                        }
                    }
                    WorkerCommand::Disconnect => {
                        if let Some(mut rec) = recorder_opt.take() {
                            if let Ok(files) = rec.finish() {
                                let _ = evt_tx.send(WorkerEvent::RecordingStopped {
                                    completed_files: files,
                                });
                            }
                        }
                        if let Some(client) = active_client.take() {
                            let _ = client.disconnect().await;
                        }
                        daq_rx_opt = None;
                        mock_server = None;
                        let _ = evt_tx.send(WorkerEvent::Disconnected {
                            reason: "User disconnected".to_string(),
                        });
                    }
                    WorkerCommand::StartRecording {
                        folder,
                        prefix,
                        max_size_bytes,
                    } => {
                        if let (Some(client), Some(model), Some(planner)) =
                            (&active_client, &active_model, &active_planner)
                        {
                            let start_utc = Utc::now();
                            match SdfSessionRecorder::new(
                                model,
                                &folder,
                                &prefix,
                                start_utc,
                                max_size_bytes,
                            ) {
                                Ok(rec) => {
                                    if let Err(e) = client.configure_and_start_daq(planner).await {
                                        let _ = evt_tx.send(WorkerEvent::StatusMessage(format!(
                                            "Failed to start DAQ on target: {e}"
                                        )));
                                    } else {
                                        let active_file = rec.current_file_path();
                                        let start_utc_human = format_utc_for_filename(start_utc);
                                        recorder_opt = Some(rec);
                                        let _ = evt_tx.send(WorkerEvent::RecordingStarted {
                                            active_file,
                                            start_utc_human,
                                        });
                                    }
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::StatusMessage(format!(
                                        "Failed to initialize SDF recorder: {e}"
                                    )));
                                }
                            }
                        } else {
                            let _ = evt_tx.send(WorkerEvent::StatusMessage(
                                "Connect to an XCP target before starting recording.".to_string(),
                            ));
                        }
                    }
                    WorkerCommand::StopRecording => {
                        if let Some(client) = &active_client {
                            let _ = client.stop_daq().await;
                        }
                        if let Some(mut rec) = recorder_opt.take() {
                            match rec.finish() {
                                Ok(files) => {
                                    let _ = evt_tx.send(WorkerEvent::RecordingStopped {
                                        completed_files: files,
                                    });
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::StatusMessage(format!(
                                        "Error finalizing SDF file: {e}"
                                    )));
                                }
                            }
                        }
                    }
                    WorkerCommand::ReadCharacteristic { characteristic } => {
                        if let Some(client) = &active_client {
                            match client.read_characteristic(&characteristic).await {
                                Ok(raw_bytes) => {
                                    let _ = evt_tx.send(WorkerEvent::CharacteristicValueRead {
                                        name: characteristic.name,
                                        raw_bytes,
                                    });
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::CharacteristicWriteFailed {
                                        name: characteristic.name,
                                        error: e.to_string(),
                                    });
                                }
                            }
                        }
                    }
                    WorkerCommand::WriteCharacteristic {
                        characteristic,
                        raw_bytes,
                        verify_readback,
                    } => {
                        if let Some(client) = &active_client {
                            match client
                                .write_memory(
                                    characteristic.address,
                                    characteristic.address_extension,
                                    &raw_bytes,
                                    verify_readback,
                                )
                                .await
                            {
                                Ok(()) => {
                                    let _ = evt_tx.send(WorkerEvent::CharacteristicValueRead {
                                        name: characteristic.name,
                                        raw_bytes,
                                    });
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::CharacteristicWriteFailed {
                                        name: characteristic.name,
                                        error: e.to_string(),
                                    });
                                }
                            }
                        }
                    }
                    WorkerCommand::SetCalPage { page } => {
                        if let Some(client) = &active_client {
                            let _ = client.set_calibration_page(0, page).await;
                        }
                    }
                    WorkerCommand::Shutdown => {
                        if let Some(mut rec) = recorder_opt.take() {
                            let _ = rec.finish();
                        }
                        if let Some(client) = active_client.take() {
                            let _ = client.disconnect().await;
                        }
                        drop(mock_server);
                        return;
                    }
                }
            }

            // Drain incoming DAQ DTO packets if recording is active
            let mut processed_any = false;
            if let (Some(rx), Some(planner), Some(model), Some(rec)) = (
                &mut daq_rx_opt,
                &mut active_planner,
                &active_model,
                &mut recorder_opt,
            ) {
                while let Ok(dto) = rx.try_recv() {
                    processed_any = true;
                    if let Some(sample) = planner.process_dto_packet(
                        dto.odt_num,
                        dto.daq_list_num,
                        &dto.payload,
                        model,
                        &mut active_clock_sync,
                    ) {
                        if let Ok(Some(completed_part)) = rec.record_event_sample(&sample) {
                            let _ = evt_tx.send(WorkerEvent::RecordingPartSplit {
                                completed_file: completed_part,
                                new_active_file: rec.current_file_path(),
                                new_part_number: rec.current_part(),
                            });
                        }
                        let _ = evt_tx.send(WorkerEvent::RecordingProgress {
                            active_file: rec.current_file_path(),
                            part_number: rec.current_part(),
                            current_part_bytes: rec.current_part_bytes(),
                            max_part_bytes: rec.max_part_bytes(),
                            total_samples: rec.total_samples_recorded(),
                            latest_sample: sample,
                        });
                    }
                }
            }

            if !processed_any {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }
    });
}
