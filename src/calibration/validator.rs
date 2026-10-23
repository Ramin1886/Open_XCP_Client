// SPDX-License-Identifier: GPL-2.0-only
//! Strict data-type and A2L `[LowerLimit, UpperLimit]` validator and encoder for
//! calibration parameters (`CHARACTERISTIC`).

use crate::a2l::model::{A2lDataType, A2lModel, Characteristic};
use thiserror::Error;

/// Validation errors when user edits a calibration parameter and presses Enter.
#[derive(Debug, Error, PartialEq)]
pub enum CalibrationValidationError {
    #[error("Input value cannot be empty")]
    EmptyInput,
    #[error("Expected {expected} elements for array parameter '{name}', but got {actual}")]
    ArrayLengthMismatch {
        name: String,
        expected: usize,
        actual: usize,
    },
    #[error("Invalid {data_type} syntax '{token}': expected a valid {expected_kind}")]
    InvalidTypeSyntax {
        data_type: String,
        token: String,
        expected_kind: &'static str,
    },
    #[error("Unknown enum variant '{token}' for '{name}' (valid options: {valid_options})")]
    InvalidEnumVariant {
        name: String,
        token: String,
        valid_options: String,
    },
    #[error("Value {value} is outside allowed A2L range [{lower}, {upper}] for '{name}'")]
    OutOfPhysicalRange {
        name: String,
        value: f64,
        lower: f64,
        upper: f64,
    },
    #[error("Converted raw value {raw} overflows raw {data_type} bounds [{min}, {max}]")]
    OutOfRawTypeBounds {
        data_type: String,
        raw: f64,
        min: f64,
        max: f64,
    },
}

/// Validates user text input for a [`Characteristic`] against its data type, `COMPU_METHOD`
/// (including `TAB_VERB` verbal enums), array dimensions, and `[lower_limit, upper_limit]`,
/// returning the encoded little-endian raw target bytes ready for XCP download.
pub fn validate_and_encode_characteristic(
    ch: &Characteristic,
    model: &A2lModel,
    user_input: &str,
) -> Result<Vec<u8>, CalibrationValidationError> {
    let trimmed = user_input.trim();
    if trimmed.is_empty() {
        return Err(CalibrationValidationError::EmptyInput);
    }

    let expected_elems = ch.element_count();
    let tokens: Vec<&str> = if expected_elems > 1 {
        trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split([',', ';', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![trimmed]
    };

    if tokens.len() != expected_elems {
        return Err(CalibrationValidationError::ArrayLengthMismatch {
            name: ch.name.clone(),
            expected: expected_elems,
            actual: tokens.len(),
        });
    }

    let enum_table = model.enum_table_for(&ch.compu_method);
    let mut encoded_bytes = Vec::with_capacity(ch.total_byte_size());

    for token in tokens {
        let phys_val = if let Some(vtab) = enum_table {
            // Accept either exact enum label (case-insensitive) or integer key
            if let Some((&k, _)) = vtab
                .iter()
                .find(|(_, label)| label.eq_ignore_ascii_case(token))
            {
                k as f64
            } else if let Ok(int_val) = token.parse::<i64>() {
                if vtab.contains_key(&int_val) {
                    int_val as f64
                } else {
                    let valid_options = vtab
                        .iter()
                        .map(|(k, v)| format!("{k}:{v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(CalibrationValidationError::InvalidEnumVariant {
                        name: ch.name.clone(),
                        token: token.to_string(),
                        valid_options,
                    });
                }
            } else {
                let valid_options = vtab
                    .iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CalibrationValidationError::InvalidEnumVariant {
                    name: ch.name.clone(),
                    token: token.to_string(),
                    valid_options,
                });
            }
        } else {
            parse_typed_scalar_token(ch, model, token)?
        };

        // Check physical limits from A2L (with small epsilon for float rounding)
        let eps = 1e-7;
        if phys_val < ch.lower_limit - eps || phys_val > ch.upper_limit + eps {
            return Err(CalibrationValidationError::OutOfPhysicalRange {
                name: ch.name.clone(),
                value: phys_val,
                lower: ch.lower_limit,
                upper: ch.upper_limit,
            });
        }

        let raw_val = model.physical_to_raw(&ch.compu_method, phys_val);
        let (raw_min, raw_max) = ch.data_type.raw_bounds();
        let rounded_raw = if ch.data_type.is_integer() {
            raw_val.round()
        } else {
            raw_val
        };
        if !rounded_raw.is_finite() || rounded_raw < raw_min - eps || rounded_raw > raw_max + eps {
            return Err(CalibrationValidationError::OutOfRawTypeBounds {
                data_type: ch.data_type.as_str().to_string(),
                raw: rounded_raw,
                min: raw_min,
                max: raw_max,
            });
        }

        encoded_bytes.extend_from_slice(&ch.data_type.encode_raw_le(rounded_raw));
    }

    Ok(encoded_bytes)
}

fn parse_typed_scalar_token(
    ch: &Characteristic,
    model: &A2lModel,
    token: &str,
) -> Result<f64, CalibrationValidationError> {
    // Support boolean keywords "true"/"false" for UByte boolean parameters
    if ch.data_type == A2lDataType::UByte && ch.upper_limit <= 1.0 && ch.lower_limit >= 0.0 {
        if token.eq_ignore_ascii_case("true") {
            return Ok(1.0);
        } else if token.eq_ignore_ascii_case("false") {
            return Ok(0.0);
        }
    }

    // Determine whether the user-facing physical value must be a strict integer
    // (when the underlying data type is integer AND the conversion is Identical)
    let is_strict_int = ch.data_type.is_integer()
        && model
            .compu_methods
            .get(&ch.compu_method)
            .map(|cm| matches!(cm.compu_type, crate::a2l::model::CompuType::Identical))
            .unwrap_or(true);

    if is_strict_int {
        let parsed = if let Some(hex) = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
        {
            i128::from_str_radix(hex, 16).ok().map(|v| v as f64)
        } else {
            token.parse::<i128>().ok().map(|v| v as f64)
        };
        parsed.ok_or_else(|| CalibrationValidationError::InvalidTypeSyntax {
            data_type: ch.data_type.as_str().to_string(),
            token: token.to_string(),
            expected_kind: "integer",
        })
    } else {
        let val =
            token
                .parse::<f64>()
                .map_err(|_| CalibrationValidationError::InvalidTypeSyntax {
                    data_type: ch.data_type.as_str().to_string(),
                    token: token.to_string(),
                    expected_kind: "numeric value",
                })?;
        if !val.is_finite() {
            return Err(CalibrationValidationError::InvalidTypeSyntax {
                data_type: ch.data_type.as_str().to_string(),
                token: token.to_string(),
                expected_kind: "finite number",
            });
        }
        Ok(val)
    }
}

/// Decodes raw ECU bytes for a [`Characteristic`] into a user-friendly string representation
/// suitable for populating the input field in the Calibration Tab.
pub fn decode_characteristic_to_display(
    ch: &Characteristic,
    model: &A2lModel,
    raw_bytes: &[u8],
) -> String {
    let elem_size = ch.data_type.byte_size();
    let count = ch.element_count();
    let enum_table = model.enum_table_for(&ch.compu_method);
    let is_strict_int = ch.data_type.is_integer()
        && model
            .compu_methods
            .get(&ch.compu_method)
            .map(|cm| matches!(cm.compu_type, crate::a2l::model::CompuType::Identical))
            .unwrap_or(true);

    let mut formatted_elems = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * elem_size;
        let raw_val = if offset + elem_size <= raw_bytes.len() {
            ch.data_type
                .decode_raw_le(&raw_bytes[offset..offset + elem_size])
                .unwrap_or(0.0)
        } else {
            0.0
        };

        if let Some(vtab) = enum_table {
            let key = raw_val.round() as i64;
            if let Some(label) = vtab.get(&key) {
                formatted_elems.push(label.clone());
                continue;
            }
        }

        let phys = model.raw_to_physical(&ch.compu_method, raw_val);
        if is_strict_int {
            formatted_elems.push(format!("{}", phys.round() as i64));
        } else {
            // Trim trailing zeroes cleanly while keeping precision
            let s = format!("{:.6}", phys);
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            formatted_elems.push(if s.is_empty() || s == "-0" {
                "0".to_string()
            } else {
                s
            });
        }
    }

    if count == 1 {
        formatted_elems.into_iter().next().unwrap_or_default()
    } else {
        format!("[{}]", formatted_elems.join(", "))
    }
}
