// SPDX-License-Identifier: GPL-2.0-only
//! Persistent JSON configuration store (`./config/settings.json`).
//!
//! Stores connection settings, recording preferences (including the configurable
//! logfile and MP4 video split size in MB, defaulting to 500 MB), Ego vehicle geometry,
//! 2D XY plot signal-pair layers, time-series plot selection, calibration watch-list,
//! and UI options.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Default maximum logfile/video size before splitting, in megabytes (500 MB).
pub const DEFAULT_MAX_LOGFILE_SIZE_MB: u64 = 500;

/// Default XCP server port used by Vector `xcplite` 2.2.2.
pub const DEFAULT_XCP_PORT: u16 = 5555;

/// Default subfolder for recorded `.sdf` and `.mp4` logfiles.
pub const DEFAULT_LOG_FOLDER: &str = "./logfile";

/// Default logfile name prefix.
pub const DEFAULT_FILE_PREFIX: &str = "Measurement";

/// Default video recording frame rate (FPS).
pub const DEFAULT_VIDEO_FPS: u32 = 10;

/// Default ego vehicle length in meters.
pub const DEFAULT_EGO_LENGTH_M: f64 = 4.85;

/// Default ego vehicle width in meters.
pub const DEFAULT_EGO_WIDTH_M: f64 = 1.90;

/// Default longitudinal offset from rear axle (origin `(0, 0)`) to geometric vehicle center (`m`).
pub const DEFAULT_EGO_REAR_AXLE_TO_CENTER_M: f64 = 1.425;

/// Errors occurring during configuration load or save operations.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error accessing settings file '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON serialization error in '{path}': {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Invalid configuration parameter: {0}")]
    Validation(String),
}

/// Network transport protocol for XCP on Ethernet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransportProtocol {
    #[default]
    Udp,
    Tcp,
}

impl TransportProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Tcp => "TCP",
        }
    }
}

/// Target XCP connection settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSettings {
    pub target_ip: String,
    pub target_port: u16,
    pub transport: TransportProtocol,
    pub connect_timeout_ms: u64,
    pub local_a2l_fallback_path: String,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            target_ip: "127.0.0.1".to_string(),
            target_port: DEFAULT_XCP_PORT,
            transport: TransportProtocol::Udp,
            connect_timeout_ms: 2000,
            local_a2l_fallback_path: String::new(),
        }
    }
}

fn default_record_video_mp4() -> bool {
    true
}

fn default_video_fps() -> u32 {
    DEFAULT_VIDEO_FPS
}

fn default_video_encoder() -> String {
    "libx264".to_string()
}

fn default_video_width() -> u32 {
    1280
}

fn default_video_height() -> u32 {
    720
}

/// Recording, `.sdf` file-splitting, and `.mp4` 2D plot video recording settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSettings {
    pub default_log_folder: String,
    pub default_file_prefix: String,
    /// Maximum size of a single `.sdf` or `.mp4` split part in megabytes (default: 500 MB).
    pub max_logfile_size_mb: u64,
    pub prefer_target_utc_clock: bool,
    /// Whether to record the 2D XY Plot as an `.mp4` (H.264) video alongside the `.sdf` file.
    #[serde(default = "default_record_video_mp4")]
    pub record_video_mp4: bool,
    /// Video recording refresh rate in frames per second (`1..=30`, default `10`).
    #[serde(default = "default_video_fps")]
    pub video_fps: u32,
    /// Preferred FFmpeg H.264 encoder (`"libx264"`, `"h264_nvenc"`, `"h264_qsv"`, `"h264_amf"`).
    #[serde(default = "default_video_encoder")]
    pub video_encoder: String,
    /// Offscreen video frame width in pixels (must be even).
    #[serde(default = "default_video_width")]
    pub video_width: u32,
    /// Offscreen video frame height in pixels (must be even).
    #[serde(default = "default_video_height")]
    pub video_height: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            default_log_folder: DEFAULT_LOG_FOLDER.to_string(),
            default_file_prefix: DEFAULT_FILE_PREFIX.to_string(),
            max_logfile_size_mb: DEFAULT_MAX_LOGFILE_SIZE_MB,
            prefer_target_utc_clock: true,
            record_video_mp4: true,
            video_fps: DEFAULT_VIDEO_FPS,
            video_encoder: default_video_encoder(),
            video_width: default_video_width(),
            video_height: default_video_height(),
        }
    }
}

/// Ego vehicle footprint dimensions and coordinate convention for the 2D Live Plot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgoVehicleSettings {
    /// Total ego vehicle length (`m`), default `4.85`.
    pub length_m: f64,
    /// Total ego vehicle width (`m`), default `1.90`.
    pub width_m: f64,
    /// Longitudinal distance (`+x` forward) from the rear-axle origin `(0, 0)` to the geometric
    /// center of the vehicle (`m`), default `1.425`.
    pub rear_axle_to_center_m: f64,
    /// Whether to render the ego vehicle outline and rear-axle crosshair at `(0, 0)`.
    pub show_ego: bool,
    /// When `true` (default), uses ISO 8855 vehicle body axes (`+x` forward/up, `+y` left).
    /// When `false`, uses standard Cartesian axes (`+x` right, `+y` up).
    pub iso8855_axes: bool,
}

impl Default for EgoVehicleSettings {
    fn default() -> Self {
        Self {
            length_m: DEFAULT_EGO_LENGTH_M,
            width_m: DEFAULT_EGO_WIDTH_M,
            rear_axle_to_center_m: DEFAULT_EGO_REAR_AXLE_TO_CENTER_M,
            show_ego: true,
            iso8855_axes: true,
        }
    }
}

impl EgoVehicleSettings {
    /// Longitudinal coordinate (`x`, meters from rear axle) of the front bumper.
    pub fn front_extent_m(&self) -> f64 {
        self.rear_axle_to_center_m + self.length_m * 0.5
    }

    /// Longitudinal coordinate (`x`, meters from rear axle, typically negative) of the rear bumper.
    pub fn rear_extent_m(&self) -> f64 {
        self.rear_axle_to_center_m - self.length_m * 0.5
    }

    /// Half-width (`y`, meters from centerline) of the vehicle body.
    pub fn half_width_m(&self) -> f64 {
        self.width_m * 0.5
    }

    /// Loads ego dimensions from a `VehicleConfig.json` file (containing `Length`, `Width`,
    /// and `RearAxleToRearBumper` or `RearAxleToCenter`).
    pub fn load_from_vehicle_config_json(&mut self, path: &Path) -> Result<(), ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let val: serde_json::Value =
            serde_json::from_str(&raw).map_err(|source| ConfigError::Json {
                path: path.display().to_string(),
                source,
            })?;
        let length = val
            .get("Length")
            .or_else(|| val.get("length_m"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let width = val
            .get("Width")
            .or_else(|| val.get("width_m"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if length <= 0.1 || width <= 0.1 {
            return Err(ConfigError::Validation(
                "Vehicle Length and Width must be > 0.1 m".to_string(),
            ));
        }
        self.length_m = length;
        self.width_m = width;
        if let Some(center) = val
            .get("RearAxleToCenter")
            .or_else(|| val.get("rear_axle_to_center_m"))
            .and_then(|v| v.as_f64())
        {
            self.rear_axle_to_center_m = center;
        } else if let Some(rear_bumper) = val.get("RearAxleToRearBumper").and_then(|v| v.as_f64()) {
            // rear_extent = -rear_bumper => rear_axle_to_center = length/2 - rear_bumper
            self.rear_axle_to_center_m = length * 0.5 - rear_bumper.abs();
        }
        Ok(())
    }
}

/// Visual rendering style for a 2D `(X, Y)` signal pair layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum XyDrawStyle {
    #[default]
    Scatter,
    Line,
    TrajectoryTrail,
    Polygon4Corner,
}

impl XyDrawStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scatter => "Points / Scatter",
            Self::Line => "Connected Line",
            Self::TrajectoryTrail => "History Trail",
            Self::Polygon4Corner => "4-Corner Polygon (P0..P3)",
        }
    }
}

fn default_point_radius() -> f32 {
    5.0
}

/// User-defined `(X, Y)` signal pair layer on the 2D Live Plot.
/// Supports scalar-scalar, vector-vector (`MATRIX_DIM`), scalar-vector broadcast,
/// and 4-corner polygon sets (`P0..P3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XyPlotLayerConfig {
    pub name: String,
    pub enabled: bool,
    pub x_signal: String,
    pub y_signal: String,
    #[serde(default)]
    pub valid_count_signal: String,
    /// Optional 4-corner signal names `[p0_x, p0_y, p1_x, p1_y, p2_x, p2_y, p3_x, p3_y]`
    /// used when `style == XyDrawStyle::Polygon4Corner`.
    #[serde(default)]
    pub corner_signals: [String; 8],
    pub style: XyDrawStyle,
    pub color_rgb: [u8; 3],
    #[serde(default = "default_point_radius")]
    pub point_radius: f32,
}

impl Default for XyPlotLayerConfig {
    fn default() -> Self {
        Self {
            name: "XY Layer".to_string(),
            enabled: true,
            x_signal: String::new(),
            y_signal: String::new(),
            valid_count_signal: String::new(),
            corner_signals: Default::default(),
            style: XyDrawStyle::Scatter,
            color_rgb: [56, 189, 248],
            point_radius: 5.0,
        }
    }
}

fn default_time_plot_window_s() -> f64 {
    20.0
}

/// Visualization settings for the 2D Live Plot and Time Plot tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualizationSettings {
    #[serde(default)]
    pub ego: EgoVehicleSettings,
    #[serde(default)]
    pub xy_layers: Vec<XyPlotLayerConfig>,
    #[serde(default)]
    pub time_plot_signals: Vec<String>,
    #[serde(default = "default_time_plot_window_s")]
    pub time_plot_window_s: f64,
}

impl Default for VisualizationSettings {
    fn default() -> Self {
        Self {
            ego: EgoVehicleSettings::default(),
            xy_layers: Vec::new(),
            time_plot_signals: Vec::new(),
            time_plot_window_s: default_time_plot_window_s(),
        }
    }
}

/// Calibration tab persistence and verification settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationSettings {
    pub watch_list: Vec<String>,
    pub verify_readback: bool,
    pub active_cal_page: u8,
}

impl Default for CalibrationSettings {
    fn default() -> Self {
        Self {
            watch_list: Vec::new(),
            verify_readback: true,
            active_cal_page: 0,
        }
    }
}

/// User interface preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSettings {
    pub dark_mode: bool,
    pub enable_mock_server: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            enable_mock_server: false,
        }
    }
}

/// Complete application settings persisted in JSON (`./config/settings.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub connection: ConnectionSettings,
    pub recording: RecordingSettings,
    pub calibration: CalibrationSettings,
    pub ui: UiSettings,
    #[serde(default)]
    pub visualization: VisualizationSettings,
}

impl AppSettings {
    /// Validates and sanitizes settings values, returning an error if invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.connection.target_ip.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Target IP address cannot be empty".to_string(),
            ));
        }
        if self.connection.target_port == 0 {
            return Err(ConfigError::Validation(
                "Target port must be between 1 and 65535".to_string(),
            ));
        }
        if self.recording.max_logfile_size_mb == 0 {
            return Err(ConfigError::Validation(
                "Maximum logfile size (MB) must be at least 1 MB".to_string(),
            ));
        }
        if self.recording.default_log_folder.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Default log folder cannot be empty".to_string(),
            ));
        }
        if self.recording.default_file_prefix.trim().is_empty() {
            return Err(ConfigError::Validation(
                "Default logfile name prefix cannot be empty".to_string(),
            ));
        }
        if self.recording.video_fps == 0 || self.recording.video_fps > 30 {
            return Err(ConfigError::Validation(
                "Video recording FPS must be between 1 and 30".to_string(),
            ));
        }
        if self.visualization.ego.length_m <= 0.1 || self.visualization.ego.width_m <= 0.1 {
            return Err(ConfigError::Validation(
                "Ego vehicle length and width must be greater than 0.1 m".to_string(),
            ));
        }
        Ok(())
    }

    /// Returns the maximum logfile/video split size in bytes (`max_logfile_size_mb * 1_000_000`).
    /// Uses decimal megabytes (1 MB = 1,000,000 bytes) so a 500 MB setting never exceeds
    /// either 500,000,000 bytes or 500 MiB (524,288,000 bytes).
    pub fn max_logfile_size_bytes(&self) -> u64 {
        self.recording.max_logfile_size_mb.saturating_mul(1_000_000)
    }
}

/// Manages loading and saving [`AppSettings`] to a JSON file on disk.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    /// Creates a new `ConfigStore` pointing to the given JSON file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the default configuration file path (`./config/settings.json`).
    pub fn default_path() -> PathBuf {
        PathBuf::from("./config/settings.json")
    }

    /// Returns the underlying path managed by this store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads settings from the configured path, falling back to `./config/settings.default.json`
    /// or `AppSettings::default()` if the file does not exist yet.
    pub fn load_or_default(&self) -> Result<AppSettings, ConfigError> {
        if self.path.exists() {
            return self.load_from_path(&self.path);
        }
        let fallback = PathBuf::from("./config/settings.default.json");
        if fallback.exists() {
            return self.load_from_path(&fallback);
        }
        let defaults = AppSettings::default();
        Ok(defaults)
    }

    /// Reads and parses [`AppSettings`] from an explicit file path.
    pub fn load_from_path(&self, path: &Path) -> Result<AppSettings, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let settings: AppSettings =
            serde_json::from_str(&raw).map_err(|source| ConfigError::Json {
                path: path.display().to_string(),
                source,
            })?;
        settings.validate()?;
        Ok(settings)
    }

    /// Validates and writes [`AppSettings`] as pretty-printed JSON to disk.
    pub fn save(&self, settings: &AppSettings) -> Result<(), ConfigError> {
        settings.validate()?;
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }
        let serialized =
            serde_json::to_string_pretty(settings).map_err(|source| ConfigError::Json {
                path: self.path.display().to_string(),
                source,
            })?;
        fs::write(&self.path, serialized).map_err(|source| ConfigError::Io {
            path: self.path.display().to_string(),
            source,
        })
    }
}
