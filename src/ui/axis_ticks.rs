// SPDX-License-Identifier: GPL-2.0-only
//! Zoom-adaptive 1-2-5 metric grid step and tick calculation for the 2D Live Plot
//! and Time Plot axes.

/// Rounds a raw metric step up to the nearest `1, 2, 5 × 10^n` value.
pub fn nice_step(raw_step: f64) -> f64 {
    if !raw_step.is_finite() || raw_step <= 0.0 {
        return 1.0;
    }
    let exp = raw_step.log10().floor();
    let base = 10.0_f64.powf(exp);
    let frac = raw_step / base;
    if frac <= 1.0 {
        1.0 * base
    } else if frac <= 2.0 {
        2.0 * base
    } else if frac <= 5.0 {
        5.0 * base
    } else {
        10.0 * base
    }
}

/// Computes `(minor_step, major_step)` in world units (`m` or `s`) given the visible
/// world `span` and the viewport size `canvas_px` in pixels.
/// Targets approximately 28 pixels between minor grid lines and 110 pixels between major grid lines.
pub fn grid_steps(span: f64, canvas_px: f64) -> (f64, f64) {
    let px = canvas_px.max(64.0);
    let world_span = span.abs().max(1e-3);
    let minor = nice_step(world_span / (px / 28.0)).max(0.05);
    let mut major = nice_step(world_span / (px / 110.0)).max(minor);
    let ratio = (major / minor).round();
    if ratio < 2.0
        || ((ratio - 2.0).abs() > 0.1 && (ratio - 5.0).abs() > 0.1 && (ratio - 10.0).abs() > 0.1)
    {
        major = minor * 5.0;
    }
    (minor, major)
}

/// Generates all grid tick coordinates in `[lo, hi]` spaced by `step`.
pub fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    if !step.is_finite() || step <= 0.0 || !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let (min_v, max_v) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let first = (min_v / step).ceil() as i64;
    let last = (max_v / step).floor() as i64;
    let count = (last - first + 1).clamp(0, 512) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut v = ((first + i as i64) as f64) * step;
        if v.abs() < 1e-12 {
            v = 0.0;
        }
        out.push(v);
    }
    out
}

/// Returns `true` if `value` lies on an integer multiple of `major_step`.
pub fn is_major(value: f64, major_step: f64) -> bool {
    if major_step <= 0.0 || !major_step.is_finite() {
        return false;
    }
    let k = (value / major_step).round();
    (value - k * major_step).abs() <= major_step * 1e-4
}

/// Formats a tick coordinate with precision matched to `step`.
pub fn tick_text(value: f64, step: f64) -> String {
    let v = if value.abs() < 1e-9 { 0.0 } else { value };
    if step >= 1.0 && (v - v.round()).abs() < 1e-6 {
        format!("{:.0}", v)
    } else if step >= 0.1 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}
