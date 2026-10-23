// SPDX-License-Identifier: GPL-2.0-only
//! Calibration parameter management, strict data-type and range validation,
//! physical/raw conversion, and concurrent target read/write/verify state tracking.

pub mod manager;
pub mod validator;

pub use manager::{CalibrationEntryState, CalibrationManager, CalibrationSyncStatus};
pub use validator::{
    decode_characteristic_to_display, validate_and_encode_characteristic,
    CalibrationValidationError,
};
