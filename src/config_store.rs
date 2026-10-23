// SPDX-License-Identifier: GPL-2.0-only
//! Persistent JSON configuration store (`./config/settings.json`).
//!
//! Stores connection settings, recording preferences (including the configurable
//! logfile split size in MB, defaulting to 500 MB), calibration watch-list, and UI options.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Default maximum logfile size before splitting, in megabytes (500 MB).
pub const DEFAULT_MAX_LOGFILE_SIZE_MB: u64 = 500;

/// Default XCP server port used by Vector `xcplite` 2.2.2.
pub const DEFAULT_XCP_PORT: u16 = 5555;

/// Default subfolder for recorded `.sdf` logfiles.
pub const DEFAULT_LOG_FOLDER: &str = "./logfile";

/// Default logfile name prefix.
pub const DEFAULT_FILE_PREFIX: &str = "Measurement";

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

/// Recording and `.sdf` file-splitting settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSettings {
    pub default_log_folder: String,
    pub default_file_prefix: String,
    /// Maximum size of a single `.sdf` split part in megabytes (default: 500 MB).
    pub max_logfile_size_mb: u64,
    pub prefer_target_utc_clock: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            default_log_folder: DEFAULT_LOG_FOLDER.to_string(),
            default_file_prefix: DEFAULT_FILE_PREFIX.to_string(),
            max_logfile_size_mb: DEFAULT_MAX_LOGFILE_SIZE_MB,
            prefer_target_utc_clock: true,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub connection: ConnectionSettings,
    pub recording: RecordingSettings,
    pub calibration: CalibrationSettings,
    pub ui: UiSettings,
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
        Ok(())
    }

    /// Returns the maximum logfile split size in bytes (`max_logfile_size_mb * 1_000_000`).
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
