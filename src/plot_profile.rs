// SPDX-License-Identifier: GPL-2.0-only
//! Named, reusable plot profiles (`./config/profiles/<Name>.json`).
//!
//! A *plot profile* captures one complete measurement view — the 2D Live Plot `(X, Y)` signal
//! pair layers, the Time Plot signal selection and time window, the Ego vehicle geometry and
//! coordinate convention, the 2D viewport framing, and the Calibration watch-list — as a single
//! standalone, pretty-printed JSON file. Profiles can be saved, loaded, updated, deleted,
//! imported and exported independently of `./config/settings.json`, so a measurement setup is
//! defined once and reused across test sessions or handed to a colleague.
//!
//! Profiles reference signals by their A2L name only, which keeps them A2L-agnostic: loading a
//! profile against a target whose A2L lacks some of those signals applies it anyway and reports
//! the unresolved names via [`PlotProfile::unresolved_signals`] instead of discarding them.

use crate::a2l::model::A2lModel;
use crate::config_store::{write_atomic, EgoVehicleSettings, XyDrawStyle, XyPlotLayerConfig};
use crate::recording::file_namer::sanitize_prefix;
use crate::recording::video_recorder::PlaneViewport;
use crate::ui::time_plot_tab::MAX_TIME_PLOT_SIGNALS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Default directory holding one JSON file per plot profile.
pub const PROFILE_DIR: &str = "./config/profiles";

/// Schema version written into every profile file; a file declaring a higher version is rejected.
pub const PROFILE_FORMAT_VERSION: u32 = 1;

/// Minimum accepted Time Plot window in seconds.
pub const MIN_TIME_PLOT_WINDOW_S: f64 = 1.0;

/// Maximum accepted Time Plot window in seconds (matches the rolling history retention).
pub const MAX_TIME_PLOT_WINDOW_S: f64 = 120.0;

/// Errors occurring while listing, loading, saving or deleting plot profiles.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("I/O error accessing profile '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON error in profile '{path}': {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Profile '{name}' was written by a newer version (format_version {found}, supported up to {supported})")]
    UnsupportedVersion {
        name: String,
        found: u32,
        supported: u32,
    },
    #[error("Invalid profile: {0}")]
    Validation(String),
    #[error("Profile '{0}' not found in {1}")]
    NotFound(String, String),
}

fn default_format_version() -> u32 {
    PROFILE_FORMAT_VERSION
}

fn default_time_plot_window_s() -> f64 {
    20.0
}

/// A named, reusable measurement view stored as one JSON file in [`PROFILE_DIR`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotProfile {
    /// Schema version of this file (see [`PROFILE_FORMAT_VERSION`]).
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    /// Display name; also determines the file name via [`ProfileStore::file_name_for`].
    pub name: String,
    /// Free-text note describing what this view is for.
    #[serde(default)]
    pub description: String,
    /// RFC-3339 UTC timestamp recorded when the profile was first saved.
    #[serde(default)]
    pub created_utc: String,
    /// `PROJECT` name of the A2L this profile was captured against (provenance only).
    #[serde(default)]
    pub a2l_project: String,
    /// `MODULE` name of the A2L this profile was captured against (provenance only).
    #[serde(default)]
    pub a2l_module: String,
    /// Ego vehicle footprint and coordinate convention.
    #[serde(default)]
    pub ego: EgoVehicleSettings,
    /// 2D Live Plot `(X, Y)` signal pair layers.
    #[serde(default)]
    pub xy_layers: Vec<XyPlotLayerConfig>,
    /// Time Plot signal selection (at most [`MAX_TIME_PLOT_SIGNALS`] entries).
    #[serde(default)]
    pub time_plot_signals: Vec<String>,
    /// Time Plot rolling window in seconds.
    #[serde(default = "default_time_plot_window_s")]
    pub time_plot_window_s: f64,
    /// Calibration parameters to re-add to the watch-list when this profile is applied.
    #[serde(default)]
    pub calibration_watch_list: Vec<String>,
    /// Saved 2D zoom/pan, so loading a profile restores the exact framing.
    #[serde(default)]
    pub viewport: Option<PlaneViewport>,
}

impl Default for PlotProfile {
    fn default() -> Self {
        Self {
            format_version: PROFILE_FORMAT_VERSION,
            name: String::new(),
            description: String::new(),
            created_utc: String::new(),
            a2l_project: String::new(),
            a2l_module: String::new(),
            ego: EgoVehicleSettings::default(),
            xy_layers: Vec::new(),
            time_plot_signals: Vec::new(),
            time_plot_window_s: default_time_plot_window_s(),
            calibration_watch_list: Vec::new(),
            viewport: None,
        }
    }
}

impl PlotProfile {
    /// Validates the profile, returning a [`ProfileError::Validation`] describing the first problem.
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.name.trim().is_empty() {
            return Err(ProfileError::Validation(
                "Profile name cannot be empty".to_string(),
            ));
        }
        if !(MIN_TIME_PLOT_WINDOW_S..=MAX_TIME_PLOT_WINDOW_S).contains(&self.time_plot_window_s) {
            return Err(ProfileError::Validation(format!(
                "Time Plot window must be between {} s and {} s",
                MIN_TIME_PLOT_WINDOW_S, MAX_TIME_PLOT_WINDOW_S
            )));
        }
        if self.time_plot_signals.len() > MAX_TIME_PLOT_SIGNALS {
            return Err(ProfileError::Validation(format!(
                "Time Plot selection holds {} signals; at most {} are supported",
                self.time_plot_signals.len(),
                MAX_TIME_PLOT_SIGNALS
            )));
        }
        for layer in &self.xy_layers {
            if layer.style == XyDrawStyle::Polygon4Corner {
                continue;
            }
            if layer.x_signal.trim().is_empty() || layer.y_signal.trim().is_empty() {
                return Err(ProfileError::Validation(format!(
                    "2D layer '{}' is missing its X or Y signal",
                    layer.name
                )));
            }
        }
        Ok(())
    }

    /// Returns every signal name this profile references, in a stable order.
    pub fn referenced_signals(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for layer in &self.xy_layers {
            for candidate in [&layer.x_signal, &layer.y_signal, &layer.valid_count_signal] {
                if !candidate.trim().is_empty() {
                    names.insert(candidate.clone());
                }
            }
            for corner in &layer.corner_signals {
                if !corner.trim().is_empty() {
                    names.insert(corner.clone());
                }
            }
        }
        for sig in &self.time_plot_signals {
            if !sig.trim().is_empty() {
                names.insert(sig.clone());
            }
        }
        names
    }

    /// Returns the signal names this profile references that do not exist in `model`.
    ///
    /// Measurement references are matched against the expanded leaf `MEASUREMENT` names produced
    /// by the A2L structure resolver; watch-list references are matched against `CHARACTERISTIC`
    /// names. Unresolved names are reported, never removed — the profile stays usable against the
    /// A2L build it was captured from.
    pub fn unresolved_signals(&self, model: &A2lModel) -> BTreeSet<String> {
        let measurements: BTreeSet<&str> =
            model.measurements.iter().map(|m| m.name.as_str()).collect();
        let characteristics: BTreeSet<&str> = model
            .characteristics
            .iter()
            .map(|c| c.name.as_str())
            .collect();

        let mut unresolved = BTreeSet::new();
        for name in self.referenced_signals() {
            if !measurements.contains(name.as_str()) {
                unresolved.insert(name);
            }
        }
        for name in &self.calibration_watch_list {
            if !name.trim().is_empty() && !characteristics.contains(name.as_str()) {
                unresolved.insert(name.clone());
            }
        }
        unresolved
    }
}

/// Summary of one profile file as shown in the profile selector.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSummary {
    /// Profile name (from the file contents, or the file stem when unreadable).
    pub name: String,
    /// Full path of the profile file.
    pub path: PathBuf,
    /// Free-text description, empty when unreadable.
    pub description: String,
    /// Number of 2D `(X, Y)` layers.
    pub layer_count: usize,
    /// Number of selected Time Plot signals.
    pub time_plot_signal_count: usize,
    /// Reason the file could not be parsed, or `None` when it loaded cleanly.
    pub invalid_reason: Option<String>,
}

impl ProfileSummary {
    /// Returns `true` when this file could not be parsed as a profile.
    pub fn is_invalid(&self) -> bool {
        self.invalid_reason.is_some()
    }

    /// Returns the one-line label shown in the profile selector.
    pub fn label(&self) -> String {
        match &self.invalid_reason {
            Some(reason) => format!("⚠ {} (unreadable: {})", self.name, reason),
            None => format!(
                "{} — {} layer(s) / {} signal(s)",
                self.name, self.layer_count, self.time_plot_signal_count
            ),
        }
    }
}

/// Manages the directory of plot profile JSON files.
#[derive(Debug, Clone)]
pub struct ProfileStore {
    dir: PathBuf,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new(Self::default_dir())
    }
}

impl ProfileStore {
    /// Creates a store over an explicit profile directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Returns the default profile directory ([`PROFILE_DIR`], relative to the process CWD).
    pub fn default_dir() -> PathBuf {
        PathBuf::from(PROFILE_DIR)
    }

    /// Returns the directory managed by this store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the sanitized file name for `name` (`"Park In / Test #2"` -> `"Park_In_Test_2.json"`).
    ///
    /// Reuses [`sanitize_prefix`], then collapses the runs of `_` it leaves behind so a name full
    /// of punctuation still yields a readable file name.
    pub fn file_name_for(name: &str) -> Result<String, ProfileError> {
        let trimmed = name.trim();
        if !trimmed
            .chars()
            .any(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ProfileError::Validation(format!(
                "Profile name '{}' contains no usable filename characters",
                name
            )));
        }
        let sanitized = sanitize_prefix(trimmed);
        let mut collapsed = String::with_capacity(sanitized.len());
        for ch in sanitized.chars() {
            if ch == '_' && collapsed.ends_with('_') {
                continue;
            }
            collapsed.push(ch);
        }
        Ok(format!("{}.json", collapsed))
    }

    /// Returns the full path a profile called `name` is stored at.
    pub fn path_for(&self, name: &str) -> Result<PathBuf, ProfileError> {
        Ok(self.dir.join(Self::file_name_for(name)?))
    }

    /// Lists every `*.json` file in the profile directory, sorted by name.
    ///
    /// A file that cannot be parsed yields a [`ProfileSummary`] with `invalid_reason` set rather
    /// than failing the whole listing. A missing profile directory yields an empty list.
    pub fn list(&self) -> Result<Vec<ProfileSummary>, ProfileError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.dir).map_err(|source| ProfileError::Io {
            path: self.dir.display().to_string(),
            source,
        })?;

        let mut summaries = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ProfileError::Io {
                path: self.dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("profile")
                .to_string();
            match self.load_from_path(&path) {
                Ok(profile) => summaries.push(ProfileSummary {
                    name: profile.name,
                    path,
                    description: profile.description,
                    layer_count: profile.xy_layers.len(),
                    time_plot_signal_count: profile.time_plot_signals.len(),
                    invalid_reason: None,
                }),
                Err(err) => summaries.push(ProfileSummary {
                    name: stem,
                    path,
                    description: String::new(),
                    layer_count: 0,
                    time_plot_signal_count: 0,
                    invalid_reason: Some(err.to_string()),
                }),
            }
        }
        summaries.sort_by_key(|s| s.name.to_lowercase());
        Ok(summaries)
    }

    /// Loads the profile called `name` from the profile directory.
    pub fn load(&self, name: &str) -> Result<PlotProfile, ProfileError> {
        let path = self.path_for(name)?;
        if !path.exists() {
            return Err(ProfileError::NotFound(
                name.to_string(),
                self.dir.display().to_string(),
            ));
        }
        self.load_from_path(&path)
    }

    /// Reads and validates a profile from an explicit path (the Import action).
    pub fn load_from_path(&self, path: &Path) -> Result<PlotProfile, ProfileError> {
        let raw = fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let profile: PlotProfile =
            serde_json::from_str(&raw).map_err(|source| ProfileError::Json {
                path: path.display().to_string(),
                source,
            })?;
        if profile.format_version > PROFILE_FORMAT_VERSION {
            return Err(ProfileError::UnsupportedVersion {
                name: profile.name,
                found: profile.format_version,
                supported: PROFILE_FORMAT_VERSION,
            });
        }
        profile.validate()?;
        Ok(profile)
    }

    /// Validates and writes `profile` into the profile directory under its sanitized name.
    pub fn save(&self, profile: &PlotProfile) -> Result<PathBuf, ProfileError> {
        let path = self.path_for(&profile.name)?;
        self.save_to_path(profile, &path)?;
        Ok(path)
    }

    /// Validates and writes `profile` to an explicit path (the Export action).
    ///
    /// The write is atomic: the JSON goes to a sibling `<file>.tmp` which is then renamed over the
    /// destination, so an interrupted save never truncates an existing profile.
    pub fn save_to_path(&self, profile: &PlotProfile, path: &Path) -> Result<(), ProfileError> {
        profile.validate()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| ProfileError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }
        let serialized =
            serde_json::to_string_pretty(profile).map_err(|source| ProfileError::Json {
                path: path.display().to_string(),
                source,
            })?;

        write_atomic(path, serialized.as_bytes()).map_err(|source| ProfileError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    /// Deletes the profile called `name`.
    pub fn delete(&self, name: &str) -> Result<(), ProfileError> {
        let path = self.path_for(name)?;
        if !path.exists() {
            return Err(ProfileError::NotFound(
                name.to_string(),
                self.dir.display().to_string(),
            ));
        }
        fs::remove_file(&path).map_err(|source| ProfileError::Io {
            path: path.display().to_string(),
            source,
        })
    }
}
