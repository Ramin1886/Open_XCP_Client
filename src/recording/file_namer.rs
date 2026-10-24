// SPDX-License-Identifier: GPL-2.0-only
//! Generates `.sdf` logfile names following the pattern:
//! `<Prefix>_<UTCTimeInHumanReadableFormat>_PART<Number>.sdf`
//! where `<UTCTimeInHumanReadableFormat>` is fixed at the moment the user clicks Start Recording.

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Formats a UTC [`DateTime`] into a filesystem-safe human-readable UTC timestamp:
/// `YYYY-MM-DD_HH-MM-SSZ` (e.g., `2026-10-23_01-30-00Z`).
pub fn format_utc_for_filename(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d_%H-%M-%SZ").to_string()
}

/// Sanitize a user-provided filename prefix so it contains no path separators or invalid chars.
pub fn sanitize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_').to_string();
    if cleaned.is_empty() {
        "Measurement".to_string()
    } else {
        cleaned
    }
}

/// Stateful logfile namer for a single recording session.
/// Keeps the start-recording UTC timestamp constant across all split parts (`PART001`, `PART002`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingFileNamer {
    folder: PathBuf,
    prefix: String,
    start_utc_human: String,
    start_utc_iso8601: String,
    current_part: u32,
}

impl RecordingFileNamer {
    /// Creates a new `RecordingFileNamer` anchored at `start_time_utc`.
    pub fn new(folder: impl Into<PathBuf>, prefix: &str, start_time_utc: DateTime<Utc>) -> Self {
        Self {
            folder: folder.into(),
            prefix: sanitize_prefix(prefix),
            start_utc_human: format_utc_for_filename(start_time_utc),
            start_utc_iso8601: start_time_utc.to_rfc3339(),
            current_part: 1,
        }
    }

    /// Returns the human-readable UTC string captured at recording start.
    pub fn start_utc_human(&self) -> &str {
        &self.start_utc_human
    }

    /// Returns the RFC-3339 ISO-8601 UTC timestamp string captured at recording start.
    pub fn start_utc_iso8601(&self) -> &str {
        &self.start_utc_iso8601
    }

    /// Returns the current 1-based part number (`1` -> `PART001`).
    pub fn current_part(&self) -> u32 {
        self.current_part
    }

    /// Returns the output directory path.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Returns the `.sdf` filename (without directory) for a specific 1-based `part_number`.
    pub fn filename_for_part(&self, part_number: u32) -> String {
        format!(
            "{}_{}_PART{:03}.sdf",
            self.prefix,
            self.start_utc_human,
            part_number.max(1)
        )
    }

    /// Returns the `.mp4` filename (without directory) for a specific 1-based `part_number`.
    pub fn mp4_filename_for_part(&self, part_number: u32) -> String {
        format!(
            "{}_{}_PART{:03}.mp4",
            self.prefix,
            self.start_utc_human,
            part_number.max(1)
        )
    }

    /// Returns the full [`PathBuf`] for the current `.sdf` split part.
    pub fn current_file_path(&self) -> PathBuf {
        self.folder.join(self.filename_for_part(self.current_part))
    }

    /// Returns the full [`PathBuf`] for the current `.mp4` split part.
    pub fn current_mp4_file_path(&self) -> PathBuf {
        self.folder
            .join(self.mp4_filename_for_part(self.current_part))
    }

    /// Advances to the next part number (`PART001` -> `PART002`) and returns the new `.sdf` file path.
    pub fn advance_part(&mut self) -> PathBuf {
        self.current_part = self.current_part.saturating_add(1);
        self.current_file_path()
    }
}
