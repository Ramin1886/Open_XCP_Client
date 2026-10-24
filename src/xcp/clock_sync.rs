// SPDX-License-Identifier: GPL-2.0-only
//! Synchronizes target `xcplite` DAQ timestamps (`GET_DAQ_CLOCK` and DTO 32-bit timestamps)
//! to UTC epoch timestamps (`time_utc_s: f64` and `time_utc_ns: i64`).

use chrono::Utc;

/// Threshold (2020-01-01T00:00:00Z in nanoseconds) above which a 64-bit target clock
/// is treated as a real-time PTP/wallclock UTC nanosecond timestamp rather than monotonic boot ticks.
const REALTIME_UTC_NS_THRESHOLD: u64 = 1_577_836_800_000_000_000;

/// Correlates 32-bit DAQ packet timestamps (or `GET_DAQ_CLOCK` values) with UTC epoch time.
#[derive(Debug, Clone)]
pub struct TargetClockSync {
    prefer_target_utc: bool,
    /// Offset in nanoseconds added to target clock nanoseconds to yield Unix UTC nanoseconds.
    target_to_utc_offset_ns: i64,
    /// Upper 32 bits of the target nanosecond counter (for 32-bit DTO timestamp rollover tracking).
    high_u32_ns: u64,
    /// Last seen lower 32-bit timestamp from the target.
    last_low_u32_ns: Option<u32>,
    /// Whether the target clock has been anchored at least once.
    synchronized: bool,
    /// Whether the target clock natively reported real-time UTC epoch (`>= 2020-01-01`).
    target_is_native_utc: bool,
}

impl Default for TargetClockSync {
    fn default() -> Self {
        Self::new(true)
    }
}

impl TargetClockSync {
    /// Creates a new `TargetClockSync` instance.
    pub fn new(prefer_target_utc: bool) -> Self {
        Self {
            prefer_target_utc,
            target_to_utc_offset_ns: 0,
            high_u32_ns: 0,
            last_low_u32_ns: None,
            synchronized: false,
            target_is_native_utc: false,
        }
    }

    /// Returns whether the clock has been synchronized with a target `GET_DAQ_CLOCK` response.
    pub fn is_synchronized(&self) -> bool {
        self.synchronized
    }

    /// Returns whether the target reports native UTC epoch nanoseconds.
    pub fn is_target_native_utc(&self) -> bool {
        self.target_is_native_utc
    }

    /// Updates clock correlation from an XCP `GET_DAQ_CLOCK` response payload (`bytes` after `0xFF`).
    ///
    /// `xcplite` 2.2.2 supports both 32-bit (`[rsvd, rsvd, rsvd, ts0, ts1, ts2, ts3]`) and
    /// 64-bit (`[rsvd, trigger_info, payload_fmt, ts0..ts7]`) `GET_DAQ_CLOCK` responses.
    pub fn update_from_get_daq_clock_response(&mut self, res_payload: &[u8], host_utc_ns: i64) {
        let target_ns = if res_payload.len() >= 11 {
            u64::from_le_bytes([
                res_payload[3],
                res_payload[4],
                res_payload[5],
                res_payload[6],
                res_payload[7],
                res_payload[8],
                res_payload[9],
                res_payload[10],
            ])
        } else if res_payload.len() >= 7 {
            u32::from_le_bytes([
                res_payload[3],
                res_payload[4],
                res_payload[5],
                res_payload[6],
            ]) as u64
        } else {
            // Shorter replies carry only the reserved / trigger-info bytes and no timestamp
            // (bytes 0..3 are not clock data); stay unsynchronized rather than anchor on garbage.
            return;
        };

        self.anchor_target_clock(target_ns, host_utc_ns);
    }

    /// Anchors the target nanosecond counter against a known host UTC nanosecond timestamp.
    pub fn anchor_target_clock(&mut self, target_ns: u64, host_utc_ns: i64) {
        self.high_u32_ns = target_ns & 0xFFFF_FFFF_0000_0000;
        self.last_low_u32_ns = Some((target_ns & 0xFFFF_FFFF) as u32);
        if self.prefer_target_utc && target_ns >= REALTIME_UTC_NS_THRESHOLD {
            self.target_is_native_utc = true;
            self.target_to_utc_offset_ns = 0;
        } else {
            self.target_is_native_utc = false;
            self.target_to_utc_offset_ns = host_utc_ns.wrapping_sub(target_ns as i64);
        }
        self.synchronized = true;
    }

    /// Converts an incoming 32-bit DAQ DTO timestamp (nanoseconds or microseconds scaled to ns)
    /// into `(time_utc_s: f64, time_utc_ns: i64)`.
    /// If `dto_timestamp_ns` is `None` or `prefer_target_utc` is false and not synchronized,
    /// falls back to the current host UTC timestamp.
    pub fn resolve_sample_utc(&mut self, dto_timestamp_ns: Option<u32>) -> (f64, i64) {
        let host_now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        if !self.prefer_target_utc {
            let secs = (host_now_ns as f64) * 1e-9;
            return (secs, host_now_ns);
        }

        if let Some(low32) = dto_timestamp_ns {
            if !self.synchronized {
                self.anchor_target_clock(low32 as u64, host_now_ns);
            } else if let Some(prev_low) = self.last_low_u32_ns {
                // Detect 32-bit unsigned rollover (~4.29 seconds in 1ns ticks)
                if low32 < prev_low && (prev_low.wrapping_sub(low32) > 0x8000_0000) {
                    self.high_u32_ns = self.high_u32_ns.wrapping_add(0x1_0000_0000);
                }
            }
            self.last_low_u32_ns = Some(low32);
            let full_target_ns = self.high_u32_ns | (low32 as u64);
            let utc_ns = (full_target_ns as i64).wrapping_add(self.target_to_utc_offset_ns);
            let utc_s = (utc_ns as f64) * 1e-9;
            (utc_s, utc_ns)
        } else {
            let secs = (host_now_ns as f64) * 1e-9;
            (secs, host_now_ns)
        }
    }
}
