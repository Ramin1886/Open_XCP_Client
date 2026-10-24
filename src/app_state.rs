// SPDX-License-Identifier: GPL-2.0-only
//! Application controller state, background worker command/event channels,
//! and coordination between the 5 UI tabs, XCP network engine, `.sdf` + `.mp4`
//! synchronized recorder, and `.sdf` replay player.

use crate::a2l::model::{A2lModel, Characteristic};
use crate::a2l::parser::parse_a2l;
use crate::calibration::manager::CalibrationManager;
use crate::config_store::{
    AppSettings, ConfigStore, EgoVehicleSettings, XyDrawStyle, XyPlotLayerConfig,
};
use crate::mock_server::MockXcpServer;
use crate::plot_profile::{PlotProfile, ProfileStore, ProfileSummary};
use crate::recording::file_namer::format_utc_for_filename;
use crate::recording::sdf_writer::{SdfIntegritySummary, SdfSessionRecorder};
use crate::recording::video_recorder::{
    extract_xy_points_for_layer, render_plane_plot_rgb24, Mp4VideoRecorder, PlaneViewport,
};
use crate::replay::SdfReplayPlayer;
use crate::xcp::client::XcpClient;
use crate::xcp::daq_planner::{
    CycleDiscard, CycleDiscardReason, DaqEventSample, DaqListLossStats, DaqPlanner, DecodedValue,
    DTO_HEADER_RESERVE_BYTES,
};
use crate::xcp::link_stats::LinkStats;
use chrono::Utc;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Idle back-off of the worker loop when neither commands nor DTOs are pending; short enough that
/// UI commands feel immediate, long enough not to spin a core (replacing it with `select!` is TODO T-15).
const WORKER_IDLE_POLL: Duration = Duration::from_millis(2);
/// Maximum points kept per history-trail layer in the 2D plot.
const TRAIL_MAX_POINTS: usize = 400;

/// Rolling time-series history retention window in seconds (`120.0 s`).
pub const TIME_SERIES_HISTORY_SECONDS: f64 = 120.0;

/// Maximum number of recent discarded-cycle records retained for display.
pub const MAX_RECENT_CYCLE_DISCARDS: usize = 64;

/// Duration in seconds for which a changed value stays highlighted in the Numerical tab.
pub const VALUE_CHANGE_HIGHLIGHT_SECONDS: f64 = 1.0;

/// Top-level operating mode of the application (`Live` XCP target vs `.sdf` `Replay`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Live,
    Replay,
}

/// Active primary tab in the UI (`Ctrl+1` through `Ctrl+5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    PlanePlot,
    Numerical,
    TimePlot,
    Calibration,
    XcpInfo,
}

/// Per-event runtime DAQ telemetry displayed in the `XCP Info` tab.
#[derive(Debug, Clone)]
pub struct EventRuntimeStats {
    pub event_channel_id: u16,
    pub event_name: String,
    pub configured_cycle_ms: f64,
    pub received_count: u64,
    pub last_utc_s: f64,
    pub last_arrival: Option<Instant>,
    pub recent_intervals_ms: VecDeque<(Instant, f64)>,
}

impl EventRuntimeStats {
    pub fn new(event_channel_id: u16, event_name: String, configured_cycle_ms: f64) -> Self {
        Self {
            event_channel_id,
            event_name,
            configured_cycle_ms,
            received_count: 0,
            last_utc_s: 0.0,
            last_arrival: None,
            recent_intervals_ms: VecDeque::new(),
        }
    }

    /// Mean measured cycle time (`ms`) over the last 5 seconds.
    pub fn mean_cycle_ms(&self) -> Option<f64> {
        if self.recent_intervals_ms.is_empty() {
            return None;
        }
        let sum: f64 = self.recent_intervals_ms.iter().map(|(_, dt)| *dt).sum();
        Some(sum / (self.recent_intervals_ms.len() as f64))
    }

    /// Maximum measured cycle time (`ms`) over the last 5 seconds.
    pub fn max_cycle_ms(&self) -> Option<f64> {
        self.recent_intervals_ms
            .iter()
            .map(|(_, dt)| *dt)
            .fold(None, |acc, v| Some(acc.map_or(v, |m: f64| m.max(v))))
    }

    /// Milliseconds elapsed since the last sample arrived.
    pub fn age_ms(&self) -> Option<f64> {
        self.last_arrival
            .map(|t| t.elapsed().as_secs_f64() * 1000.0)
    }
}

/// Commands dispatched from the UI thread to the background XCP/Recording worker.
#[derive(Debug, Clone)]
pub enum WorkerCommand {
    Connect {
        settings: AppSettings,
    },
    Disconnect,
    ConfigureVideoRecording {
        record_video_mp4: bool,
        video_fps: u32,
        video_encoder: String,
        video_width: u32,
        video_height: u32,
        viewport: PlaneViewport,
        ego: EgoVehicleSettings,
        xy_layers: Vec<XyPlotLayerConfig>,
    },
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
    LiveSample(DaqEventSample),
    /// Link- and DAQ-integrity counters, published whenever they change.
    LinkIntegrity {
        link: LinkStats,
        per_event: Vec<(u16, DaqListLossStats)>,
        recent_discards: Vec<CycleDiscard>,
        unplanned_dto: u64,
    },
    RecordingStarted {
        active_file: PathBuf,
        active_video_file: Option<PathBuf>,
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
    VideoPartSplit {
        completed_video_file: PathBuf,
        new_active_video_file: PathBuf,
        new_part_number: u32,
    },
    RecordingStopped {
        completed_files: Vec<PathBuf>,
        completed_video_files: Vec<PathBuf>,
    },
    /// A recorder I/O step failed; the worker has closed both recorders. Parts that reached
    /// disk before the failure are listed so the UI can still show them.
    RecordingFailed {
        stage: &'static str,
        error: String,
        completed_files: Vec<PathBuf>,
        completed_video_files: Vec<PathBuf>,
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

/// Updates 2D trajectory trail buffers for all enabled `XyDrawStyle::TrajectoryTrail` layers.
pub fn update_trails_for_layers(
    xy_layers: &[XyPlotLayerConfig],
    signals_by_name: &BTreeMap<String, DecodedValue>,
    trails_by_layer: &mut BTreeMap<String, Vec<(f64, f64)>>,
) {
    for layer in xy_layers {
        if layer.enabled && layer.style == XyDrawStyle::TrajectoryTrail {
            let pts = extract_xy_points_for_layer(layer, signals_by_name);
            if !pts.is_empty() {
                let trail = trails_by_layer.entry(layer.name.clone()).or_default();
                trail.extend(pts);
                if trail.len() > TRAIL_MAX_POINTS {
                    let excess = trail.len() - TRAIL_MAX_POINTS;
                    trail.drain(0..excess);
                }
            }
        }
    }
}

/// Why a recorder step failed; the worker turns it into [`WorkerEvent::RecordingFailed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingFailure {
    /// Which step failed (`"sdf part rotation"`, `"video frame"`, ...).
    pub stage: &'static str,
    /// The underlying error text.
    pub error: String,
}

impl RecordingFailure {
    fn new(stage: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            stage,
            error: error.to_string(),
        }
    }
}

/// Records one DAQ sample into `rec` and, when `emit_video_frame` is set, renders a video frame
/// into `video_recorder_opt`, rotating both `.sdf` and `.mp4` part files in lockstep whenever
/// either reaches `rec.max_part_bytes()`.
///
/// Any I/O failure is returned instead of being dropped, so the caller can close the recorders
/// and tell the UI; the DAQ stream itself is unaffected.
#[allow(clippy::too_many_arguments)]
pub fn record_synchronized_daq_and_video_sample(
    rec: &mut SdfSessionRecorder,
    video_recorder_opt: &mut Option<Mp4VideoRecorder>,
    emit_video_frame: bool,
    viewport: &PlaneViewport,
    ego: &EgoVehicleSettings,
    xy_layers: &[XyPlotLayerConfig],
    signals_by_name: &BTreeMap<String, DecodedValue>,
    trails_by_layer: &BTreeMap<String, Vec<(f64, f64)>>,
    sample: DaqEventSample,
    evt_tx: &Sender<WorkerEvent>,
) -> Result<(), RecordingFailure> {
    let max_part_bytes = rec.max_part_bytes();
    let mut did_split = false;

    match rec.record_event_sample(&sample) {
        Ok(Some(completed_part)) => {
            did_split = true;
            let _ = evt_tx.send(WorkerEvent::RecordingPartSplit {
                completed_file: completed_part,
                new_active_file: rec.current_file_path(),
                new_part_number: rec.current_part(),
            });
            if let Some(vid) = video_recorder_opt.as_mut() {
                let completed_vid = vid
                    .rotate_part()
                    .map_err(|e| RecordingFailure::new("video part rotation", e))?;
                let _ = evt_tx.send(WorkerEvent::VideoPartSplit {
                    completed_video_file: completed_vid,
                    new_active_video_file: vid.current_file_path(),
                    new_part_number: vid.current_part(),
                });
            }
        }
        Ok(None) => {}
        Err(e) => return Err(RecordingFailure::new("sdf part rotation", e)),
    }

    if let Some(vid) = video_recorder_opt.as_mut() {
        if emit_video_frame {
            if !did_split && vid.should_split(max_part_bytes) {
                let completed_sdf = rec
                    .force_rotate_part()
                    .map_err(|e| RecordingFailure::new("sdf part rotation", e))?;
                let _ = evt_tx.send(WorkerEvent::RecordingPartSplit {
                    completed_file: completed_sdf,
                    new_active_file: rec.current_file_path(),
                    new_part_number: rec.current_part(),
                });
                let completed_vid = vid
                    .rotate_part()
                    .map_err(|e| RecordingFailure::new("video part rotation", e))?;
                let _ = evt_tx.send(WorkerEvent::VideoPartSplit {
                    completed_video_file: completed_vid,
                    new_active_video_file: vid.current_file_path(),
                    new_part_number: vid.current_part(),
                });
            }
            let rgb = render_plane_plot_rgb24(
                vid.width(),
                vid.height(),
                viewport,
                ego,
                xy_layers,
                signals_by_name,
                trails_by_layer,
            );
            vid.write_rgb24_frame(&rgb)
                .map_err(|e| RecordingFailure::new("video frame", e))?;
        }
    }

    let vid_bytes = video_recorder_opt
        .as_ref()
        .map(|v| v.current_part_bytes())
        .unwrap_or(0);
    let _ = evt_tx.send(WorkerEvent::RecordingProgress {
        active_file: rec.current_file_path(),
        part_number: rec.current_part(),
        current_part_bytes: rec.current_part_bytes().max(vid_bytes),
        max_part_bytes: rec.max_part_bytes(),
        total_samples: rec.total_samples_recorded(),
        latest_sample: sample,
    });
    Ok(())
}

/// Shared application controller state manipulated by the UI and tests.
pub struct AppController {
    pub config_store: ConfigStore,
    pub settings: AppSettings,
    pub mode: AppMode,
    pub active_tab: ActiveTab,
    pub settings_menu_open: bool,
    pub connected: bool,
    pub target_endpoint: String,
    pub target_native_utc: bool,
    pub a2l_model: Option<A2lModel>,

    // Recording & Live bar fields
    pub log_folder_input: String,
    pub log_prefix_input: String,
    pub is_recording: bool,
    /// Why the last recording ended abnormally, if it did; cleared when a new one starts.
    pub last_recording_error: Option<String>,
    pub active_recording_file: Option<PathBuf>,
    pub active_video_file: Option<PathBuf>,
    pub active_part_number: u32,
    pub current_part_bytes: u64,
    pub max_part_bytes: u64,
    pub total_samples_recorded: u64,
    pub completed_files: Vec<PathBuf>,
    pub completed_video_files: Vec<PathBuf>,

    // Live / Replay signal state & Numerical tab
    pub latest_measurement_values: BTreeMap<usize, DecodedValue>,
    pub latest_signals_by_name: BTreeMap<String, DecodedValue>,
    pub signal_changed_at: BTreeMap<usize, Instant>,
    pub measurement_search_filter: String,
    pub numerical_expand_all: Option<bool>,

    // 2D Live Plot tab state
    pub plane_viewport: PlaneViewport,
    pub layer_signal_search_filter: String,
    pub new_layer_name: String,
    pub new_layer_x_signal: String,
    pub new_layer_y_signal: String,
    pub new_layer_valid_count_signal: String,
    pub new_layer_style: XyDrawStyle,
    pub new_layer_color_rgb: [u8; 3],
    pub new_layer_corners: [String; 8],
    pub trails_by_layer: BTreeMap<String, Vec<(f64, f64)>>,

    // Time Plot tab state
    pub time_plot_search_filter: String,
    pub time_plot_frozen: bool,
    pub time_series_history: BTreeMap<String, VecDeque<(f64, f64)>>,
    pub latest_sample_utc_s: f64,

    // Calibration tab fields
    pub calibration_search_filter: String,
    pub calibration_manager: CalibrationManager,

    // XCP Info tab telemetry
    pub event_telemetry: BTreeMap<u16, EventRuntimeStats>,
    /// Link-level integrity counters (CTR gaps, queue drops) for the XCP Info tab.
    pub link_stats: LinkStats,
    /// Per-event DAQ cycle integrity counters, keyed by XCP event channel id.
    pub event_loss: BTreeMap<u16, DaqListLossStats>,
    /// Most recent discarded cycles, newest last, capped for display.
    pub recent_cycle_discards: VecDeque<(f64, CycleDiscard)>,
    /// Whether integrity was clean at the previous report, so the warning fires on transition.
    daq_integrity_was_clean: bool,

    // Replay mode state
    pub replay_player: Option<SdfReplayPlayer>,
    pub replay_file_input: String,
    pub last_replay_tick: Instant,

    // Plot profiles (./config/profiles/<Name>.json)
    pub profile_store: ProfileStore,
    pub profile_list: Vec<ProfileSummary>,
    pub selected_profile: String,
    pub new_profile_name: String,
    pub new_profile_description: String,
    pub profile_io_path: String,
    pub unresolved_profile_signals: BTreeSet<String>,
    profile_applied: bool,

    // Status bar
    pub status_message: String,

    cmd_tx: Sender<WorkerCommand>,
    evt_rx: Receiver<WorkerEvent>,
    /// Handle of the background worker thread, joined (with a deadline) on drop so open
    /// recordings are finalized before the process exits.
    worker: Option<std::thread::JoinHandle<()>>,
}

impl AppController {
    /// Creates a new `AppController` backed by `config_store` and spawns the background worker thread.
    ///
    /// Uses the default plot profile directory; see [`AppController::new_with_profile_store`] to
    /// point the profile store elsewhere.
    pub fn new(config_store: ConfigStore) -> Self {
        Self::new_with_profile_store(config_store, ProfileStore::default())
    }

    /// Creates a new `AppController` with an explicit [`ProfileStore`] (used by tests).
    ///
    /// After construction, the profile named by `settings.active_profile` is loaded and applied,
    /// so the last-used measurement view is restored across sessions.
    pub fn new_with_profile_store(config_store: ConfigStore, profile_store: ProfileStore) -> Self {
        // A malformed settings file must never be silently replaced: keep a backup, start from
        // defaults, and tell the user (the first save would otherwise overwrite their file).
        let (settings, settings_load_error) = match config_store.load_or_default() {
            Ok(settings) => (settings, None),
            Err(err) => {
                let path = config_store.path().to_path_buf();
                let backup = path.with_extension("json.bak");
                let backup_note = match std::fs::copy(&path, &backup) {
                    Ok(_) => format!("backed up to '{}'", backup.display()),
                    Err(copy_err) => format!("backup failed: {copy_err}"),
                };
                (
                    AppSettings::default(),
                    Some(format!(
                        "Settings file could not be read ({err}); {backup_note}; defaults loaded"
                    )),
                )
            }
        };
        let (cmd_tx, cmd_rx) = unbounded::<WorkerCommand>();
        let (evt_tx, evt_rx) = unbounded::<WorkerEvent>();

        let worker = std::thread::Builder::new()
            .name("open-xcp-worker".to_string())
            .spawn(move || run_background_worker(cmd_rx, evt_tx))
            .expect("Failed to spawn background XCP worker thread");

        let log_folder_input = settings.recording.default_log_folder.clone();
        let log_prefix_input = settings.recording.default_file_prefix.clone();
        let max_part_bytes = settings.max_logfile_size_bytes();

        let mut controller = Self {
            config_store,
            settings,
            mode: AppMode::Live,
            active_tab: ActiveTab::PlanePlot,
            settings_menu_open: false,
            connected: false,
            target_endpoint: String::new(),
            target_native_utc: false,
            a2l_model: None,
            log_folder_input,
            log_prefix_input,
            is_recording: false,
            last_recording_error: None,
            active_recording_file: None,
            active_video_file: None,
            active_part_number: 1,
            current_part_bytes: 0,
            max_part_bytes,
            total_samples_recorded: 0,
            completed_files: Vec::new(),
            completed_video_files: Vec::new(),
            latest_measurement_values: BTreeMap::new(),
            latest_signals_by_name: BTreeMap::new(),
            signal_changed_at: BTreeMap::new(),
            measurement_search_filter: String::new(),
            numerical_expand_all: None,
            plane_viewport: PlaneViewport::default(),
            layer_signal_search_filter: String::new(),
            new_layer_name: "Targets XY".to_string(),
            new_layer_x_signal: String::new(),
            new_layer_y_signal: String::new(),
            new_layer_valid_count_signal: String::new(),
            new_layer_style: XyDrawStyle::Scatter,
            new_layer_color_rgb: [56, 189, 248],
            new_layer_corners: Default::default(),
            trails_by_layer: BTreeMap::new(),
            time_plot_search_filter: String::new(),
            time_plot_frozen: false,
            time_series_history: BTreeMap::new(),
            latest_sample_utc_s: 0.0,
            calibration_search_filter: String::new(),
            calibration_manager: CalibrationManager::new(),
            event_telemetry: BTreeMap::new(),
            link_stats: LinkStats::new(),
            event_loss: BTreeMap::new(),
            recent_cycle_discards: VecDeque::new(),
            daq_integrity_was_clean: true,
            replay_player: None,
            replay_file_input: String::new(),
            last_replay_tick: Instant::now(),
            profile_store,
            profile_list: Vec::new(),
            selected_profile: String::new(),
            new_profile_name: String::new(),
            new_profile_description: String::new(),
            profile_io_path: String::new(),
            unresolved_profile_signals: BTreeSet::new(),
            profile_applied: false,
            status_message: "Ready — Configure target in Settings or click Connect.".to_string(),
            cmd_tx,
            evt_rx,
            worker: Some(worker),
        };

        controller.refresh_profile_list();
        let active = controller.settings.active_profile.trim().to_string();
        if !active.is_empty() {
            controller.selected_profile = active.clone();
            if let Err(err) = controller.load_profile_by_name(&active) {
                controller.status_message =
                    format!("Could not restore plot profile '{}': {}", active, err);
            }
        }
        if let Some(msg) = settings_load_error {
            controller.status_message = msg;
        }
        controller
    }

    /// Switches between `AppMode::Live` and `AppMode::Replay`.
    pub fn set_mode(&mut self, mode: AppMode) {
        if self.mode == mode {
            return;
        }
        if mode == AppMode::Replay && self.is_recording {
            let _ = self.cmd_tx.send(WorkerCommand::StopRecording);
        }
        self.mode = mode;
        self.last_replay_tick = Instant::now();
        self.status_message = match mode {
            AppMode::Live => "Switched to Live XCP mode.".to_string(),
            AppMode::Replay => {
                "Switched to SDF Replay mode — Select one or more .sdf part files.".to_string()
            }
        };
    }

    /// Loads one or more `.sdf` part files into the Replay engine and initializes the timeline.
    pub fn load_replay_files(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        let mut player = SdfReplayPlayer::load_files(paths).map_err(|e| e.to_string())?;
        let model = player.a2l_model().clone();
        self.a2l_model = Some(model.clone());
        self.init_default_layers_and_plots_for_model(&model);
        self.clear_runtime_buffers();
        let initial_samples = player.seek_to_seconds(0.0);
        for sample in initial_samples {
            self.ingest_daq_sample(sample);
        }
        let file_count = player.loaded_files().len();
        let dur = player.duration_s();
        let total = player.total_samples();
        self.replay_player = Some(player);
        self.mode = AppMode::Replay;
        self.last_replay_tick = Instant::now();
        self.status_message = format!(
            "Loaded {} .sdf part file(s) for replay ({:.2} s, {} samples).",
            file_count, dur, total
        );
        Ok(())
    }

    /// Seeks the active `.sdf` replay session to `target_pos_s` (`0.0..=duration_s`).
    pub fn replay_seek(&mut self, target_pos_s: f64) {
        let Some(mut player) = self.replay_player.take() else {
            return;
        };
        self.clear_runtime_buffers();
        let samples = player.seek_to_seconds(target_pos_s);
        for s in samples {
            self.ingest_daq_sample(s);
        }
        self.replay_player = Some(player);
    }

    /// Advances the active `.sdf` replay session by `wall_dt_s` seconds if playing.
    pub fn tick_replay(&mut self, wall_dt_s: f64) {
        if self.mode != AppMode::Replay {
            return;
        }
        let Some(mut player) = self.replay_player.take() else {
            return;
        };
        let samples = player.advance_by_wall_dt(wall_dt_s);
        for s in samples {
            self.ingest_daq_sample(s);
        }
        self.replay_player = Some(player);
    }

    fn clear_runtime_buffers(&mut self) {
        self.latest_measurement_values.clear();
        self.latest_signals_by_name.clear();
        self.signal_changed_at.clear();
        self.time_series_history.clear();
        self.trails_by_layer.clear();
        self.event_telemetry.clear();
        if let Some(model) = &self.a2l_model {
            for (&ev_id, ev) in &model.events {
                self.event_telemetry.insert(
                    ev_id,
                    EventRuntimeStats::new(ev_id, ev.name.clone(), ev.refresh_rate_ms()),
                );
            }
        }
    }

    /// Populates sensible default 2D XY Plot layers and Time Plot signals when an A2L model
    /// is loaded for the first time and the user's configuration lists are empty.
    ///
    /// Seeding is skipped once a plot profile has been applied, so an intentionally empty
    /// profile is never overwritten by the heuristic.
    pub fn init_default_layers_and_plots_for_model(&mut self, model: &A2lModel) {
        for (&ev_id, ev) in &model.events {
            self.event_telemetry.entry(ev_id).or_insert_with(|| {
                EventRuntimeStats::new(ev_id, ev.name.clone(), ev.refresh_rate_ms())
            });
        }

        if self.new_layer_x_signal.is_empty() {
            if let Some(first) = model.measurements.first() {
                self.new_layer_x_signal = first.name.clone();
            }
        }
        if self.new_layer_y_signal.is_empty() {
            if let Some(second) = model
                .measurements
                .get(1)
                .or_else(|| model.measurements.first())
            {
                self.new_layer_y_signal = second.name.clone();
            }
        }

        if !self.profile_applied
            && self.settings.visualization.xy_layers.is_empty()
            && model.measurements.len() >= 2
        {
            // Try to pair x/dist with y/lat if present, otherwise pair first two signals
            let x_cand = model
                .measurements
                .iter()
                .find(|m| {
                    let lower = m.name.to_ascii_lowercase();
                    lower.contains("dist") || lower.ends_with(".x") || lower.contains("pos_x")
                })
                .unwrap_or(&model.measurements[0]);
            let y_cand = model
                .measurements
                .iter()
                .find(|m| {
                    let lower = m.name.to_ascii_lowercase();
                    lower.contains("lat") || lower.ends_with(".y") || lower.contains("pos_y")
                })
                .unwrap_or(&model.measurements[1]);

            self.settings
                .visualization
                .xy_layers
                .push(XyPlotLayerConfig {
                    name: "Primary XY Pair".to_string(),
                    enabled: true,
                    x_signal: x_cand.name.clone(),
                    y_signal: y_cand.name.clone(),
                    valid_count_signal: String::new(),
                    corner_signals: Default::default(),
                    style: XyDrawStyle::Scatter,
                    color_rgb: [56, 189, 248],
                    point_radius: 5.5,
                });
        }

        if !self.profile_applied && self.settings.visualization.time_plot_signals.is_empty() {
            for m in model.measurements.iter().take(4) {
                self.settings
                    .visualization
                    .time_plot_signals
                    .push(m.name.clone());
            }
        }

        // Now that an A2L model is known, re-check which profile signals it actually resolves.
        self.recompute_unresolved_profile_signals(model);
    }

    /// Ingests a decoded [`DaqEventSample`] into all live/replay visualization buffers:
    /// - `latest_measurement_values` & `latest_signals_by_name`
    /// - `signal_changed_at` (for 1.0 s change highlight in the Numerical tab)
    /// - `time_series_history` (rolling 120 s window for the Time Plot tab)
    /// - `trails_by_layer` (for 2D Plot layers with `XyDrawStyle::TrajectoryTrail`)
    /// - `event_telemetry` (for the XCP Info tab)
    pub fn ingest_daq_sample(&mut self, sample: DaqEventSample) {
        let now = Instant::now();
        self.latest_sample_utc_s = sample.time_utc_s;

        // Update per-event runtime telemetry
        let configured_ms = self
            .a2l_model
            .as_ref()
            .and_then(|m| m.events.get(&sample.event_channel_id))
            .map(|e| e.refresh_rate_ms())
            .unwrap_or(10.0);
        let ev_stat = self
            .event_telemetry
            .entry(sample.event_channel_id)
            .or_insert_with(|| {
                EventRuntimeStats::new(
                    sample.event_channel_id,
                    sample.event_name.clone(),
                    configured_ms,
                )
            });
        if ev_stat.last_utc_s > 0.0 && sample.time_utc_s > ev_stat.last_utc_s {
            let dt_ms = (sample.time_utc_s - ev_stat.last_utc_s) * 1_000.0;
            ev_stat.recent_intervals_ms.push_back((now, dt_ms));
            while let Some((ts, _)) = ev_stat.recent_intervals_ms.front() {
                if now.duration_since(*ts).as_secs_f64() > 5.0 {
                    ev_stat.recent_intervals_ms.pop_front();
                } else {
                    break;
                }
            }
        }
        ev_stat.received_count = ev_stat.received_count.saturating_add(1);
        ev_stat.last_utc_s = sample.time_utc_s;
        ev_stat.last_arrival = Some(now);

        let cutoff_utc = sample.time_utc_s - TIME_SERIES_HISTORY_SECONDS;

        for (m_idx, val) in sample.values {
            let changed = match self.latest_measurement_values.get(&m_idx) {
                Some(prev) => prev != &val,
                None => true,
            };
            if changed {
                self.signal_changed_at.insert(m_idx, now);
            }
            if let Some(model) = &self.a2l_model {
                if let Some(m) = model.measurements.get(m_idx) {
                    let scalar = val.primary_scalar();
                    if !self.time_plot_frozen {
                        let series = self.time_series_history.entry(m.name.clone()).or_default();
                        series.push_back((sample.time_utc_s, scalar));
                        while let Some(&(t_front, _)) = series.front() {
                            if t_front < cutoff_utc {
                                series.pop_front();
                            } else {
                                break;
                            }
                        }
                    }
                    self.latest_signals_by_name
                        .insert(m.name.clone(), val.clone());
                }
            }
            self.latest_measurement_values.insert(m_idx, val);
        }

        // Update 2D history trails for any layer using TrajectoryTrail
        update_trails_for_layers(
            &self.settings.visualization.xy_layers,
            &self.latest_signals_by_name,
            &mut self.trails_by_layer,
        );
    }

    /// Synchronizes the current 2D plot scene configuration to the background worker
    /// so offscreen `.mp4` recording reflects any layer, ego, or viewport changes.
    pub fn sync_video_scene_to_worker(&self) {
        let _ = self.cmd_tx.send(WorkerCommand::ConfigureVideoRecording {
            record_video_mp4: self.settings.recording.record_video_mp4,
            video_fps: self.settings.recording.video_fps,
            video_encoder: self.settings.recording.video_encoder.clone(),
            video_width: self.settings.recording.video_width,
            video_height: self.settings.recording.video_height,
            viewport: self.plane_viewport,
            ego: self.settings.visualization.ego.clone(),
            xy_layers: self.settings.visualization.xy_layers.clone(),
        });
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
        self.sync_video_scene_to_worker();
        self.status_message = format!(
            "Settings saved to {} (Max Split Size: {} MB)",
            self.config_store.path().display(),
            self.settings.recording.max_logfile_size_mb
        );
        Ok(())
    }

    /// Persists the current visualization state (2D layers, Time Plot selection, Ego/axes)
    /// to `./config/settings.json` on a best-effort basis.
    ///
    /// Called from the plot sidebars so a view survives application exit without the user
    /// having to open `⚙ Settings` and click Save.
    pub fn persist_visualization(&mut self) {
        if let Err(err) = self.save_settings() {
            self.status_message = format!("Could not persist plot settings: {}", err);
        }
    }

    /// Re-reads the profile directory into [`AppController::profile_list`].
    pub fn refresh_profile_list(&mut self) {
        match self.profile_store.list() {
            Ok(list) => self.profile_list = list,
            Err(err) => {
                self.profile_list.clear();
                self.status_message = format!("Could not list plot profiles: {}", err);
            }
        }
    }

    /// Recomputes which signals referenced by the active profile are absent from `model`.
    pub fn recompute_unresolved_profile_signals(&mut self, model: &A2lModel) {
        let profile =
            self.capture_current_profile(self.settings.active_profile.clone(), String::new());
        self.unresolved_profile_signals = profile.unresolved_signals(model);
    }

    /// Snapshots the current measurement view into a [`PlotProfile`].
    pub fn capture_current_profile(&self, name: String, description: String) -> PlotProfile {
        let (a2l_project, a2l_module) = self
            .a2l_model
            .as_ref()
            .map(|m| (m.project_name.clone(), m.module_name.clone()))
            .unwrap_or_default();

        PlotProfile {
            format_version: crate::plot_profile::PROFILE_FORMAT_VERSION,
            name,
            description,
            created_utc: Utc::now().to_rfc3339(),
            a2l_project,
            a2l_module,
            ego: self.settings.visualization.ego.clone(),
            xy_layers: self.settings.visualization.xy_layers.clone(),
            time_plot_signals: self.settings.visualization.time_plot_signals.clone(),
            time_plot_window_s: self.settings.visualization.time_plot_window_s,
            calibration_watch_list: self.calibration_manager.watch_list_names(),
            viewport: Some(self.plane_viewport),
        }
    }

    /// Captures the current view and saves it as the profile `name`, making it the active profile.
    pub fn save_current_as_profile(
        &mut self,
        name: &str,
        description: &str,
    ) -> Result<PathBuf, String> {
        let profile =
            self.capture_current_profile(name.trim().to_string(), description.trim().to_string());
        let path = self
            .profile_store
            .save(&profile)
            .map_err(|e| e.to_string())?;
        self.settings.active_profile = profile.name.clone();
        self.selected_profile = profile.name.clone();
        self.profile_applied = true;
        self.refresh_profile_list();
        self.save_settings()?;
        self.status_message = format!(
            "Plot profile '{}' saved to {} ({} layer(s), {} Time Plot signal(s))",
            profile.name,
            path.display(),
            profile.xy_layers.len(),
            profile.time_plot_signals.len()
        );
        Ok(path)
    }

    /// Applies `profile` to the live view: 2D layers, Time Plot selection, Ego geometry,
    /// viewport framing and the Calibration watch-list.
    ///
    /// Signals absent from the current A2L are kept in the profile and reported through
    /// [`AppController::unresolved_profile_signals`] rather than being discarded.
    pub fn apply_profile(&mut self, profile: &PlotProfile) {
        self.settings.visualization.ego = profile.ego.clone();
        self.settings.visualization.xy_layers = profile.xy_layers.clone();
        self.settings.visualization.time_plot_signals = profile.time_plot_signals.clone();
        self.settings.visualization.time_plot_window_s = profile.time_plot_window_s;
        self.plane_viewport = profile.viewport.unwrap_or_default();
        self.trails_by_layer.clear();

        // Loading a profile replaces the watch-list rather than accumulating onto it.
        self.calibration_manager.clear();
        for name in &profile.calibration_watch_list {
            self.add_calibration_variable(name);
        }
        self.settings.calibration.watch_list = profile.calibration_watch_list.clone();

        self.unresolved_profile_signals = match &self.a2l_model {
            Some(model) => profile.unresolved_signals(model),
            None => BTreeSet::new(),
        };

        self.settings.active_profile = profile.name.clone();
        self.selected_profile = profile.name.clone();
        self.profile_applied = true;
        self.sync_video_scene_to_worker();
        if let Err(err) = self.save_settings() {
            self.status_message = format!("Could not persist active profile: {}", err);
            return;
        }

        let unresolved = self.unresolved_profile_signals.len();
        self.status_message = if unresolved == 0 {
            format!(
                "Plot profile '{}' applied ({} layer(s), {} Time Plot signal(s))",
                profile.name,
                profile.xy_layers.len(),
                profile.time_plot_signals.len()
            )
        } else {
            format!(
                "Plot profile '{}' applied ({} layer(s), {} Time Plot signal(s)) — {} signal(s) not in the current A2L (kept)",
                profile.name,
                profile.xy_layers.len(),
                profile.time_plot_signals.len(),
                unresolved
            )
        };
    }

    /// Loads the profile called `name` from the profile directory and applies it.
    pub fn load_profile_by_name(&mut self, name: &str) -> Result<(), String> {
        let profile = self.profile_store.load(name).map_err(|e| e.to_string())?;
        self.apply_profile(&profile);
        Ok(())
    }

    /// Imports a profile from an explicit file path, applies it, and copies it into the
    /// profile directory so it appears in the selector.
    pub fn import_profile_from_path(&mut self, path: &str) -> Result<(), String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("Enter a path to a profile .json file".to_string());
        }
        let profile = self
            .profile_store
            .load_from_path(Path::new(trimmed))
            .map_err(|e| e.to_string())?;
        self.profile_store
            .save(&profile)
            .map_err(|e| e.to_string())?;
        self.apply_profile(&profile);
        self.refresh_profile_list();
        Ok(())
    }

    /// Exports the current view to an explicit file path, without changing the active profile.
    pub fn export_current_profile_to_path(&mut self, path: &str) -> Result<(), String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("Enter a destination path for the exported profile".to_string());
        }
        let name = if self.settings.active_profile.trim().is_empty() {
            Path::new(trimmed)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Profile")
                .to_string()
        } else {
            self.settings.active_profile.clone()
        };
        let profile =
            self.capture_current_profile(name, self.new_profile_description.trim().to_string());
        self.profile_store
            .save_to_path(&profile, Path::new(trimmed))
            .map_err(|e| e.to_string())?;
        self.status_message = format!("Plot profile exported to {}", trimmed);
        Ok(())
    }

    /// Deletes the profile called `name`, clearing the active-profile pointer when it matches.
    pub fn delete_profile(&mut self, name: &str) -> Result<(), String> {
        self.profile_store.delete(name).map_err(|e| e.to_string())?;
        if self.settings.active_profile == name {
            self.settings.active_profile.clear();
            self.profile_applied = false;
            self.unresolved_profile_signals.clear();
            let _ = self.save_settings();
        }
        if self.selected_profile == name {
            self.selected_profile.clear();
        }
        self.refresh_profile_list();
        self.status_message = format!("Plot profile '{}' deleted", name);
        Ok(())
    }

    /// Returns a one-line summary of link and DAQ-cycle integrity for the status bar.
    pub fn integrity_summary(&self) -> String {
        let discarded: u64 = self.event_loss.values().map(|s| s.cycles_discarded).sum();
        let no_header: u64 = self
            .event_loss
            .values()
            .map(|s| s.cycles_lost_no_header)
            .sum();
        let seq: u64 = self
            .event_loss
            .values()
            .map(|s| s.odt_sequence_errors)
            .sum();
        format!(
            "{} | cycles discarded {}, missing-header {}, ODT sequence errors {}",
            self.link_stats.summary(),
            discarded,
            no_header,
            seq
        )
    }

    /// Returns `true` when no loss has been observed on the link or in any DAQ list.
    pub fn daq_integrity_is_clean(&self) -> bool {
        self.link_stats.is_clean() && self.event_loss.values().all(DaqListLossStats::is_clean)
    }

    /// Returns `true` when `signal` is referenced by the active profile but absent from the A2L.
    pub fn is_signal_unresolved(&self, signal: &str) -> bool {
        self.unresolved_profile_signals.contains(signal)
    }

    /// Requests connection to the configured XCP target (or built-in mock server).
    pub fn request_connect(&mut self) {
        self.mode = AppMode::Live;
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

    /// Toggles Start / Stop Recording (synchronized `.sdf` + `.mp4`).
    pub fn toggle_recording(&mut self) {
        if self.is_recording {
            let _ = self.cmd_tx.send(WorkerCommand::StopRecording);
        } else {
            let max_bytes = self.settings.max_logfile_size_bytes();
            self.max_part_bytes = max_bytes;
            self.sync_video_scene_to_worker();
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
                if !self.connected {
                    // Nothing was written: say so instead of showing the entry as Synced.
                    self.calibration_manager.mark_error(
                        name,
                        "Not connected: value not written to the target".to_string(),
                    );
                    self.status_message =
                        format!("Not connected: '{}' was not written to the target", name);
                    return Err(format!("not connected: '{name}' was not written"));
                }
                let _ = self.cmd_tx.send(WorkerCommand::WriteCharacteristic {
                    characteristic,
                    raw_bytes,
                    verify_readback: self.settings.calibration.verify_readback,
                });
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

                self.init_default_layers_and_plots_for_model(&a2l_model);

                let saved_watch = self.settings.calibration.watch_list.clone();
                self.a2l_model = Some(*a2l_model);
                for name in saved_watch {
                    self.add_calibration_variable(&name);
                }
                self.refresh_all_calibrations();
                self.sync_video_scene_to_worker();

                self.status_message = format!(
                    "Connected to {} — Live DAQ streaming ({} measurements, {} events, {} calibrations)",
                    target_endpoint, meas_count, ev_count, cal_count
                );
            }
            WorkerEvent::Disconnected { reason } => {
                self.connected = false;
                self.is_recording = false;
                self.active_video_file = None;
                self.status_message = format!("Disconnected: {}", reason);
            }
            WorkerEvent::LiveSample(sample) => {
                if self.mode == AppMode::Live {
                    self.ingest_daq_sample(sample);
                }
            }
            WorkerEvent::LinkIntegrity {
                link,
                per_event,
                recent_discards,
                unplanned_dto,
            } => {
                self.link_stats = link;
                self.link_stats.unplanned_dto = unplanned_dto;
                self.event_loss = per_event.into_iter().collect();
                let now = Utc::now().timestamp_millis() as f64 / 1000.0;
                for discard in recent_discards {
                    self.recent_cycle_discards.push_back((now, discard));
                }
                while self.recent_cycle_discards.len() > MAX_RECENT_CYCLE_DISCARDS {
                    self.recent_cycle_discards.pop_front();
                }
                // Warn on the transition into a lossy state only, so an integrity report never
                // overwrites an unrelated message on every poll.
                let now_clean = self.daq_integrity_is_clean();
                if !now_clean && self.daq_integrity_was_clean {
                    self.status_message = format!("⚠ DAQ integrity: {}", self.integrity_summary());
                }
                self.daq_integrity_was_clean = now_clean;
            }
            WorkerEvent::RecordingStarted {
                active_file,
                active_video_file,
                start_utc_human,
            } => {
                self.is_recording = true;
                self.last_recording_error = None;
                self.active_recording_file = Some(active_file.clone());
                self.active_video_file = active_video_file.clone();
                self.active_part_number = 1;
                self.current_part_bytes = 0;
                self.total_samples_recorded = 0;
                if let Some(vid) = active_video_file {
                    self.status_message = format!(
                        "Recording started at {} -> {} + {}",
                        start_utc_human,
                        active_file.display(),
                        vid.display()
                    );
                } else {
                    self.status_message = format!(
                        "Recording started at {} -> {}",
                        start_utc_human,
                        active_file.display()
                    );
                }
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
                if self.mode == AppMode::Live {
                    self.ingest_daq_sample(latest_sample);
                }
            }
            WorkerEvent::RecordingPartSplit {
                completed_file,
                new_active_file,
                new_part_number,
            } => {
                self.completed_files.push(completed_file);
                self.active_recording_file = Some(new_active_file.clone());
                self.active_part_number = new_part_number;
                self.status_message = format!(
                    "Split logfile at size limit -> rotated to PART{:03} ({})",
                    new_part_number,
                    new_active_file.display()
                );
            }
            WorkerEvent::VideoPartSplit {
                completed_video_file,
                new_active_video_file,
                new_part_number,
            } => {
                self.completed_video_files.push(completed_video_file);
                self.active_video_file = Some(new_active_video_file);
                self.active_part_number = new_part_number;
            }
            WorkerEvent::RecordingStopped {
                completed_files,
                completed_video_files,
            } => {
                self.is_recording = false;
                self.active_video_file = None;
                self.completed_files = completed_files;
                self.completed_video_files = completed_video_files;
                self.status_message = format!(
                    "Recording stopped — {} .sdf file(s) and {} .mp4 file(s) finalized.",
                    self.completed_files.len(),
                    self.completed_video_files.len()
                );
            }
            WorkerEvent::RecordingFailed {
                stage,
                error,
                completed_files,
                completed_video_files,
            } => {
                self.is_recording = false;
                self.active_recording_file = None;
                self.active_video_file = None;
                self.completed_files.extend(completed_files);
                self.completed_video_files.extend(completed_video_files);
                let msg = format!("Recording stopped: {stage} failed: {error}");
                self.last_recording_error = Some(msg.clone());
                self.status_message = msg;
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

/// How long `Drop` waits for the worker to finalize recordings and disconnect before giving up
/// and leaving the thread detached (it covers the 2 s command timeout plus a part flush).
const WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

impl Drop for AppController {
    fn drop(&mut self) {
        // Ask the worker to finish open recorders and disconnect, then wait for it so the
        // process cannot exit (and lose the current part) while finish() is still writing.
        let _ = self.cmd_tx.send(WorkerCommand::Shutdown);
        if let Some(handle) = self.worker.take() {
            let deadline = std::time::Instant::now() + WORKER_JOIN_TIMEOUT;
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

#[derive(Clone)]
struct WorkerVideoConfig {
    record_video_mp4: bool,
    video_fps: u32,
    video_encoder: String,
    video_width: u32,
    video_height: u32,
    viewport: PlaneViewport,
    ego: EgoVehicleSettings,
    xy_layers: Vec<XyPlotLayerConfig>,
}

impl Default for WorkerVideoConfig {
    fn default() -> Self {
        Self {
            record_video_mp4: true,
            video_fps: 10,
            video_encoder: "libx264".to_string(),
            video_width: 1280,
            video_height: 720,
            viewport: PlaneViewport::default(),
            ego: EgoVehicleSettings::default(),
            xy_layers: Vec::new(),
        }
    }
}

/// Closes both recorders (best effort) and reports the outcome: `RecordingFailed` when `failure`
/// is set or a `finish()` fails, otherwise `RecordingStopped` whenever a recorder existed. Every
/// path that ends a recording goes through here so the UI can never be left showing "REC".
fn finish_recorders_and_report(
    recorder_opt: &mut Option<SdfSessionRecorder>,
    video_recorder_opt: &mut Option<Mp4VideoRecorder>,
    failure: Option<RecordingFailure>,
    finish_stage: &'static str,
    evt_tx: &Sender<WorkerEvent>,
) {
    let had_recorder = recorder_opt.is_some() || video_recorder_opt.is_some();
    let mut failure = failure;
    let mut completed_files = Vec::new();
    let mut completed_video_files = Vec::new();
    if let Some(mut rec) = recorder_opt.take() {
        match rec.finish() {
            Ok(files) => completed_files = files,
            Err(e) => {
                completed_files = rec.completed_files().to_vec();
                failure.get_or_insert_with(|| {
                    RecordingFailure::new(finish_stage, format!("SDF finalize: {e}"))
                });
            }
        }
    }
    if let Some(mut vid) = video_recorder_opt.take() {
        match vid.finish() {
            Ok(files) => completed_video_files = files,
            Err(e) => {
                completed_video_files = vid.completed_files().to_vec();
                failure.get_or_insert_with(|| {
                    RecordingFailure::new(finish_stage, format!("video finalize: {e}"))
                });
            }
        }
    }
    match failure {
        Some(f) => {
            let _ = evt_tx.send(WorkerEvent::RecordingFailed {
                stage: f.stage,
                error: f.error,
                completed_files,
                completed_video_files,
            });
        }
        None if had_recorder => {
            let _ = evt_tx.send(WorkerEvent::RecordingStopped {
                completed_files,
                completed_video_files,
            });
        }
        None => {}
    }
}

fn run_background_worker(cmd_rx: Receiver<WorkerCommand>, evt_tx: Sender<WorkerEvent>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime for XCP worker");

    rt.block_on(async move {
        let mut last_reported_link_stats = LinkStats::new();
        let mut last_link_generation = 0u64;
        let mut mock_server: Option<MockXcpServer> = None;
        let mut active_client: Option<XcpClient> = None;
        let mut active_model: Option<A2lModel> = None;
        let mut active_planner: Option<DaqPlanner> = None;
        let mut active_clock_sync = crate::xcp::clock_sync::TargetClockSync::new(true);
        let mut daq_rx_opt: Option<
            tokio::sync::mpsc::Receiver<crate::xcp::transport::IncomingDaqDto>,
        > = None;
        let mut recorder_opt: Option<SdfSessionRecorder> = None;
        let mut video_recorder_opt: Option<Mp4VideoRecorder> = None;
        let mut video_cfg = WorkerVideoConfig::default();
        let mut worker_signals_by_name: BTreeMap<String, DecodedValue> = BTreeMap::new();
        let mut worker_trails_by_layer: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        let mut last_video_frame_instant = Instant::now();

        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    WorkerCommand::Connect { settings } => {
                        finish_recorders_and_report(
                            &mut recorder_opt,
                            &mut video_recorder_opt,
                            None,
                            "finalize on reconnect",
                            &evt_tx,
                        );
                        if let Some(client) = active_client.take() {
                            let _ = client.disconnect().await;
                        }
                        mock_server = None;
                        worker_signals_by_name.clear();
                        worker_trails_by_layer.clear();

                        video_cfg.record_video_mp4 = settings.recording.record_video_mp4;
                        video_cfg.video_fps = settings.recording.video_fps.clamp(1, 30);
                        video_cfg.video_encoder = settings.recording.video_encoder.clone();
                        video_cfg.video_width = settings.recording.video_width;
                        video_cfg.video_height = settings.recording.video_height;
                        video_cfg.ego = settings.visualization.ego.clone();
                        video_cfg.xy_layers = settings.visualization.xy_layers.clone();

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
                                    Ok((a2l_text, a2l_source)) => match parse_a2l(&a2l_text) {
                                        Ok(model) => {
                                            let planner = DaqPlanner::from_a2l(
                                                &model,
                                                client
                                                    .max_dto()
                                                    .saturating_sub(DTO_HEADER_RESERVE_BYTES),
                                            );
                                            // Start live DAQ streaming immediately on connect.
                                            // A failure here must be reported: otherwise the UI
                                            // claims "Live DAQ streaming" while no DTO ever
                                            // arrives, with nothing to distinguish a rejected DAQ
                                            // setup from a target that never triggers its events.
                                            let daq_error = client
                                                .configure_and_start_daq(&planner)
                                                .await
                                                .err()
                                                .map(|e| e.to_string());
                                            let dropped =
                                                planner.dropped_measurement_indices().len();

                                            let native_utc = clock_sync.is_target_native_utc();
                                            active_clock_sync = clock_sync;
                                            active_planner = Some(planner);
                                            active_model = Some(model.clone());
                                            active_client = Some(client);
                                            daq_rx_opt = Some(daq_rx);

                                            let _ = evt_tx.send(WorkerEvent::Connected {
                                                target_endpoint: format!(
                                                    "{} ({}, A2L from {})",
                                                    target_addr,
                                                    settings.connection.transport.as_str(),
                                                    a2l_source
                                                ),
                                                a2l_model: Box::new(model),
                                                target_native_utc: native_utc,
                                            });

                                            // Sent after Connected so the connect status
                                            // message cannot overwrite these diagnostics.
                                            if let Some(err) = daq_error {
                                                let _ = evt_tx.send(WorkerEvent::StatusMessage(
                                                    format!(
                                                        "Failed to start DAQ on target: {err} — \
                                                         connected, but no live data will arrive"
                                                    ),
                                                ));
                                            } else if dropped > 0 {
                                                let _ = evt_tx.send(WorkerEvent::StatusMessage(
                                                    format!(
                                                        "Live DAQ started, but {dropped} \
                                                         measurement(s) exceed the per-DAQ-list \
                                                         ODT limit of this target and are not \
                                                         acquired"
                                                    ),
                                                ));
                                            }
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
                        finish_recorders_and_report(
                            &mut recorder_opt,
                            &mut video_recorder_opt,
                            None,
                            "finalize on disconnect",
                            &evt_tx,
                        );
                        if let Some(client) = active_client.take() {
                            let _ = client.stop_daq().await;
                            let _ = client.disconnect().await;
                        }
                        daq_rx_opt = None;
                        mock_server = None;
                        let _ = evt_tx.send(WorkerEvent::Disconnected {
                            reason: "User disconnected".to_string(),
                        });
                    }
                    WorkerCommand::ConfigureVideoRecording {
                        record_video_mp4,
                        video_fps,
                        video_encoder,
                        video_width,
                        video_height,
                        viewport,
                        ego,
                        xy_layers,
                    } => {
                        video_cfg = WorkerVideoConfig {
                            record_video_mp4,
                            video_fps: video_fps.clamp(1, 30),
                            video_encoder,
                            video_width,
                            video_height,
                            viewport,
                            ego,
                            xy_layers,
                        };
                    }
                    WorkerCommand::StartRecording {
                        folder,
                        prefix,
                        max_size_bytes,
                    } => {
                        if let Some(model) = active_model
                            .as_ref()
                            .filter(|_| active_client.is_some() && active_planner.is_some())
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
                                    // DAQ is configured and streaming from the moment we
                                    // connect, and StopRecording never stops it, so recording
                                    // is purely host-side. Re-running the DAQ setup here would
                                    // tear down a working configuration (and some slaves reject
                                    // the re-allocation outright with ERR_MEMORY_OVERFLOW).
                                    let active_file = rec.current_file_path();
                                    let start_utc_human = format_utc_for_filename(start_utc);
                                    let mut active_video_file = None;
                                    let mut video_failure: Option<RecordingFailure> = None;
                                    if video_cfg.record_video_mp4 {
                                        match Mp4VideoRecorder::new(
                                            &folder,
                                            &prefix,
                                            start_utc,
                                            video_cfg.video_width,
                                            video_cfg.video_height,
                                            video_cfg.video_fps,
                                            &video_cfg.video_encoder,
                                        ) {
                                            Ok(mut vid) => {
                                                // Write initial frame immediately so PART001.mp4 has content
                                                let rgb = render_plane_plot_rgb24(
                                                    vid.width(),
                                                    vid.height(),
                                                    &video_cfg.viewport,
                                                    &video_cfg.ego,
                                                    &video_cfg.xy_layers,
                                                    &worker_signals_by_name,
                                                    &worker_trails_by_layer,
                                                );
                                                match vid.write_rgb24_frame(&rgb) {
                                                    Ok(()) => {
                                                        active_video_file =
                                                            Some(vid.current_file_path());
                                                        video_recorder_opt = Some(vid);
                                                        last_video_frame_instant = Instant::now();
                                                    }
                                                    Err(e) => {
                                                        video_failure = Some(
                                                            RecordingFailure::new("video start", e),
                                                        );
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                video_failure =
                                                    Some(RecordingFailure::new("video start", e));
                                            }
                                        }
                                    }
                                    if let Some(f) = video_failure {
                                        // The operator asked for .sdf + .mp4 in lockstep; a silent
                                        // .sdf-only recording would violate that, so refuse to start.
                                        drop(rec);
                                        let _ = evt_tx.send(WorkerEvent::RecordingFailed {
                                            stage: f.stage,
                                            error: f.error,
                                            completed_files: Vec::new(),
                                            completed_video_files: Vec::new(),
                                        });
                                    } else {
                                        recorder_opt = Some(rec);
                                        let _ = evt_tx.send(WorkerEvent::RecordingStarted {
                                            active_file,
                                            active_video_file,
                                            start_utc_human,
                                        });
                                    }
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(WorkerEvent::RecordingFailed {
                                        stage: "start",
                                        error: format!("Failed to initialize SDF recorder: {e}"),
                                        completed_files: Vec::new(),
                                        completed_video_files: Vec::new(),
                                    });
                                }
                            }
                        } else {
                            let _ = evt_tx.send(WorkerEvent::StatusMessage(
                                "Connect to an XCP target before starting recording.".to_string(),
                            ));
                        }
                    }
                    WorkerCommand::StopRecording => {
                        finish_recorders_and_report(
                            &mut recorder_opt,
                            &mut video_recorder_opt,
                            None,
                            "finalize",
                            &evt_tx,
                        );
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
                        finish_recorders_and_report(
                            &mut recorder_opt,
                            &mut video_recorder_opt,
                            None,
                            "finalize on shutdown",
                            &evt_tx,
                        );
                        if let Some(client) = active_client.take() {
                            let _ = client.disconnect().await;
                        }
                        drop(mock_server);
                        return;
                    }
                }
            }

            // Drain incoming DAQ DTO packets (both during Live monitoring and during Recording)
            let mut processed_any = false;
            if let (Some(rx), Some(planner), Some(model)) =
                (&mut daq_rx_opt, &mut active_planner, &active_model)
            {
                while let Ok(dto) = rx.try_recv() {
                    processed_any = true;

                    // The transport observes the CTR of every frame (CTO responses included,
                    // since the counter is shared) and bumps its generation on any discontinuity.
                    // A change here proves datagrams went missing but not which DAQ list they
                    // belonged to, so every half-assembled cycle becomes untrustworthy.
                    if dto.link_generation != last_link_generation {
                        last_link_generation = dto.link_generation;
                        planner.invalidate_open_cycles(CycleDiscardReason::LinkDiscontinuity);
                    }

                    if let Some(sample) = planner.process_dto_packet(
                        dto.odt_num,
                        dto.daq_list_num,
                        &dto.payload,
                        dto.link_generation,
                        model,
                        &mut active_clock_sync,
                    ) {
                        for (&m_idx, val) in &sample.values {
                            if let Some(m) = model.measurements.get(m_idx) {
                                worker_signals_by_name.insert(m.name.clone(), val.clone());
                            }
                        }
                        update_trails_for_layers(
                            &video_cfg.xy_layers,
                            &worker_signals_by_name,
                            &mut worker_trails_by_layer,
                        );

                        let record_result = if let Some(rec) = &mut recorder_opt {
                            let emit_video_frame = video_recorder_opt
                                .as_ref()
                                .map(|vid| {
                                    let interval =
                                        Duration::from_secs_f64(1.0 / (vid.fps().max(1) as f64));
                                    last_video_frame_instant.elapsed() >= interval
                                })
                                .unwrap_or(false);
                            if emit_video_frame {
                                last_video_frame_instant = Instant::now();
                            }
                            record_synchronized_daq_and_video_sample(
                                rec,
                                &mut video_recorder_opt,
                                emit_video_frame,
                                &video_cfg.viewport,
                                &video_cfg.ego,
                                &video_cfg.xy_layers,
                                &worker_signals_by_name,
                                &worker_trails_by_layer,
                                sample,
                                &evt_tx,
                            )
                        } else {
                            let _ = evt_tx.send(WorkerEvent::LiveSample(sample));
                            Ok(())
                        };
                        if let Err(failure) = record_result {
                            // Disk failed, the link did not: close the recorders, tell the UI,
                            // and keep draining DTOs so live plots stay alive.
                            finish_recorders_and_report(
                                &mut recorder_opt,
                                &mut video_recorder_opt,
                                Some(failure),
                                "finalize after error",
                                &evt_tx,
                            );
                        }
                    }
                }
            }

            // Publish link and DAQ-cycle integrity counters whenever they move.
            if let (Some(client), Some(planner)) = (&active_client, &mut active_planner) {
                let link_stats = client.link_stats();
                let discards = planner.take_cycle_discards();

                // An active recording carries its own provenance: every discarded cycle becomes an
                // auditable gap record, so a hole in the data is visible rather than merely absent.
                if let Some(rec) = &mut recorder_opt {
                    let host_now = Utc::now().timestamp_millis() as f64 / 1000.0;
                    for discard in &discards {
                        let event_name = planner
                            .lists
                            .iter()
                            .find(|l| l.daq_list_num == discard.daq_list_num)
                            .map(|l| l.event_name.as_str())
                            .unwrap_or("Unknown");
                        rec.note_discarded_cycle(
                            discard.event_channel_id,
                            event_name,
                            host_now,
                            discard.reason.label(),
                        );
                    }
                    let totals = planner.loss_stats().iter().fold(
                        SdfIntegritySummary {
                            link_gaps_detected: link_stats.gaps_detected,
                            link_frames_lost_estimate: link_stats.frames_lost_estimate,
                            link_reordered: link_stats.reordered,
                            link_duplicated: link_stats.duplicated,
                            channel_overflow_drops: link_stats.channel_overflow_drops,
                            unplanned_dto: planner.unplanned_dto_count(),
                            measurements_not_acquired: planner.dropped_measurement_indices().len()
                                as u64,
                            ..SdfIntegritySummary::default()
                        },
                        |mut acc, st| {
                            acc.cycles_discarded =
                                acc.cycles_discarded.saturating_add(st.cycles_discarded);
                            acc.cycles_lost_no_header = acc
                                .cycles_lost_no_header
                                .saturating_add(st.cycles_lost_no_header);
                            acc.odt_sequence_errors = acc
                                .odt_sequence_errors
                                .saturating_add(st.odt_sequence_errors);
                            acc
                        },
                    );
                    rec.set_integrity_counters(totals);
                }

                if !discards.is_empty() || link_stats != last_reported_link_stats {
                    let per_event = planner
                        .lists
                        .iter()
                        .zip(planner.loss_stats().iter())
                        .map(|(l, st)| (l.event_channel_id, *st))
                        .collect();
                    let _ = evt_tx.send(WorkerEvent::LinkIntegrity {
                        link: link_stats.clone(),
                        per_event,
                        recent_discards: discards,
                        unplanned_dto: planner.unplanned_dto_count(),
                    });
                    last_reported_link_stats = link_stats;
                }
            }

            if !processed_any {
                tokio::time::sleep(WORKER_IDLE_POLL).await;
            }
        }
    });
}
