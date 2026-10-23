// SPDX-License-Identifier: GPL-2.0-only
//! Physical <-> Raw value conversion rules (`COMPU_METHOD`) for A2L variables.

use crate::a2l::model::{CompuMethod, CompuType};

/// Converts an ECU raw value into its physical value using the given [`CompuMethod`].
pub fn apply_raw_to_physical(cm: &CompuMethod, raw: f64) -> f64 {
    match &cm.compu_type {
        CompuType::Identical | CompuType::TabVerb { .. } => raw,
        CompuType::Linear { a, b } => a * raw + b,
        CompuType::RatFunc { a, b, c, d, e, f } => {
            // In ASAM MCD-2 MC RAT_FUNC:
            // f(phys) = (a*phys^2 + b*phys + c) / (d*phys^2 + e*phys + f) = raw
            // For standard linear RAT_FUNC (a == 0, d == 0, e == 0):
            // (b * phys + c) / f = raw  =>  phys = (raw * f - c) / b
            if a.abs() < f64::EPSILON
                && d.abs() < f64::EPSILON
                && e.abs() < f64::EPSILON
                && b.abs() > f64::EPSILON
            {
                (raw * f - c) / b
            } else {
                raw
            }
        }
    }
}

/// Converts a physical value into its ECU raw value using the given [`CompuMethod`].
pub fn apply_physical_to_raw(cm: &CompuMethod, phys: f64) -> f64 {
    match &cm.compu_type {
        CompuType::Identical | CompuType::TabVerb { .. } => phys,
        CompuType::Linear { a, b } => {
            if a.abs() > f64::EPSILON {
                (phys - b) / a
            } else {
                phys
            }
        }
        CompuType::RatFunc { a, b, c, d, e, f } => {
            let num = a * phys * phys + b * phys + c;
            let den = d * phys * phys + e * phys + f;
            if den.abs() > f64::EPSILON {
                num / den
            } else {
                phys
            }
        }
    }
}
