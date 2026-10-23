// SPDX-License-Identifier: GPL-2.0-only
//! Manages the active Calibration Content list, tracks current target values vs.
//! user-edited input buffers, and handles validation and synchronization status.

use crate::a2l::model::{A2lModel, Characteristic};
use crate::calibration::validator::{
    decode_characteristic_to_display, validate_and_encode_characteristic,
    CalibrationValidationError,
};
use std::collections::BTreeMap;

/// Synchronization state of a calibration parameter in the active Calibration Content table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalibrationSyncStatus {
    /// Added to content list; waiting for initial read from target.
    PendingRead,
    /// Current input matches the latest value read or verified on the target.
    Synced,
    /// User has modified the input field (not yet sent with Enter).
    Modified,
    /// Write command has been dispatched to the XCP worker.
    Sending,
    /// Validation or XCP error occurred.
    Error(String),
}

/// State of a single calibration variable added to the Calibration Content tab.
#[derive(Debug, Clone)]
pub struct CalibrationEntryState {
    pub characteristic: Characteristic,
    /// Last confirmed value decoded from the target.
    pub target_value_display: String,
    /// Current editable string in the UI input widget.
    pub input_buffer: String,
    /// Per-element editable strings when `characteristic.element_count() > 1`.
    pub element_buffers: Vec<String>,
    /// Current synchronization status.
    pub status: CalibrationSyncStatus,
}

impl CalibrationEntryState {
    /// Creates a new entry initialized with default `0` display until target read completes.
    pub fn new(ch: Characteristic, model: &A2lModel) -> Self {
        let zero_bytes = vec![0u8; ch.total_byte_size()];
        let initial_display = decode_characteristic_to_display(&ch, model, &zero_bytes);
        let element_buffers = split_display_to_elements(&initial_display, ch.element_count());
        Self {
            characteristic: ch,
            target_value_display: initial_display.clone(),
            input_buffer: initial_display,
            element_buffers,
            status: CalibrationSyncStatus::PendingRead,
        }
    }

    /// Updates the entry with fresh raw bytes uploaded from the XCP target.
    pub fn apply_target_raw_bytes(&mut self, model: &A2lModel, raw_bytes: &[u8]) {
        let display = decode_characteristic_to_display(&self.characteristic, model, raw_bytes);
        self.target_value_display = display.clone();
        self.input_buffer = display.clone();
        self.element_buffers =
            split_display_to_elements(&display, self.characteristic.element_count());
        self.status = CalibrationSyncStatus::Synced;
    }

    /// Synchronizes `input_buffer` from `element_buffers` when an individual array element cell is edited.
    pub fn sync_input_from_elements(&mut self) {
        if self.characteristic.element_count() > 1 {
            self.input_buffer = format!("[{}]", self.element_buffers.join(", "));
            self.status = CalibrationSyncStatus::Modified;
        }
    }
}

fn split_display_to_elements(display: &str, count: usize) -> Vec<String> {
    if count <= 1 {
        return vec![display.to_string()];
    }
    let parts: Vec<String> = display
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    if parts.len() == count {
        parts
    } else {
        vec!["0".to_string(); count]
    }
}

/// Manages all calibration entries added to the Calibration Content panel.
#[derive(Debug, Clone, Default)]
pub struct CalibrationManager {
    entries: BTreeMap<String, CalibrationEntryState>,
}

impl CalibrationManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a calibration parameter by name from `model` into the active content list.
    /// Returns `true` if newly added.
    pub fn add_by_name(&mut self, name: &str, model: &A2lModel) -> bool {
        if self.entries.contains_key(name) {
            return false;
        }
        if let Some(ch) = model.characteristics.iter().find(|c| c.name == name) {
            self.entries.insert(
                name.to_string(),
                CalibrationEntryState::new(ch.clone(), model),
            );
            true
        } else {
            false
        }
    }

    /// Removes a calibration parameter from the active content list.
    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    /// Clears all entries from the active content list.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns a list of all active calibration variable names (for saving to `settings.json`).
    pub fn watch_list_names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Returns an immutable map of active entries.
    pub fn entries(&self) -> &BTreeMap<String, CalibrationEntryState> {
        &self.entries
    }

    /// Returns a mutable map of active entries.
    pub fn entries_mut(&mut self) -> &mut BTreeMap<String, CalibrationEntryState> {
        &mut self.entries
    }

    /// Updates the user input string for `name` (before Enter is pressed).
    pub fn update_input_buffer(&mut self, name: &str, new_input: String) {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.input_buffer = new_input;
            entry.status = if entry.input_buffer.trim() == entry.target_value_display.trim() {
                CalibrationSyncStatus::Synced
            } else {
                CalibrationSyncStatus::Modified
            };
        }
    }

    /// Called when the user presses Enter on a calibration input field:
    /// validates the current `input_buffer` against the parameter's data type and A2L limits,
    /// and if valid, marks status as `Sending` and returns `(Characteristic, encoded_raw_bytes)`.
    pub fn prepare_write_on_enter(
        &mut self,
        name: &str,
        model: &A2lModel,
    ) -> Result<(Characteristic, Vec<u8>), CalibrationValidationError> {
        let entry = self
            .entries
            .get_mut(name)
            .ok_or(CalibrationValidationError::EmptyInput)?;

        match validate_and_encode_characteristic(&entry.characteristic, model, &entry.input_buffer)
        {
            Ok(raw_bytes) => {
                entry.status = CalibrationSyncStatus::Sending;
                Ok((entry.characteristic.clone(), raw_bytes))
            }
            Err(err) => {
                entry.status = CalibrationSyncStatus::Error(err.to_string());
                Err(err)
            }
        }
    }

    /// Updates an entry when target read or write verification succeeds.
    pub fn mark_synced_with_raw(&mut self, name: &str, model: &A2lModel, raw_bytes: &[u8]) {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.apply_target_raw_bytes(model, raw_bytes);
        }
    }

    /// Marks an entry with an error message if an XCP read/write operation failed.
    pub fn mark_error(&mut self, name: &str, error_msg: String) {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.status = CalibrationSyncStatus::Error(error_msg);
        }
    }
}
