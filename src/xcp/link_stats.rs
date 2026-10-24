// SPDX-License-Identifier: GPL-2.0-only
//! Link-level integrity accounting for the XCP-on-Ethernet connection.
//!
//! Every XCP-on-Ethernet frame carries a 16-bit `CTR` field in its header. The slave increments it
//! per transmitted frame, so a discontinuity in the received sequence proves that datagrams were
//! lost, reordered or duplicated between the slave and this process. The counter is shared by CTO
//! and DTO frames on one connection, so it detects loss at the **link** level without identifying
//! which DAQ list was affected; per-DAQ-list completeness is tracked separately by
//! [`crate::xcp::daq_planner::DaqPlanner`].
//!
//! Without this, a dropped UDP datagram is invisible: the frame simply never arrives, and nothing
//! downstream can distinguish "the target sent nothing" from "the network ate it".

/// What observing one received `CTR` value revealed about the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrObservation {
    /// First frame of the connection; nothing to compare against yet.
    First,
    /// The expected successor of the previous frame.
    InOrder,
    /// `lost` frames went missing before this one.
    Gap { lost: u16 },
    /// The same `CTR` arrived twice.
    Duplicate,
    /// A `CTR` older than the one already seen arrived (out-of-order delivery).
    Reordered,
}

impl CtrObservation {
    /// Returns `true` when this observation means the stream is no longer continuous, so any
    /// partially received DAQ cycle can no longer be trusted.
    pub fn is_discontinuity(self) -> bool {
        matches!(self, Self::Gap { .. } | Self::Duplicate | Self::Reordered)
    }
}

/// Rolling integrity counters for one XCP connection.
///
/// `generation` increments on every discontinuity, giving consumers a cheap way to notice that the
/// link broke since they last looked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkStats {
    expected_next_ctr: Option<u16>,
    /// Frames whose `CTR` has been observed.
    pub frames_received: u64,
    /// Number of separate discontinuities observed (not the number of frames lost).
    pub gaps_detected: u64,
    /// Sum of the frame counts implied by every gap.
    pub frames_lost_estimate: u64,
    /// Frames that arrived with a `CTR` older than one already seen.
    pub reordered: u64,
    /// Frames that repeated the previous `CTR`.
    pub duplicated: u64,
    /// DTOs discarded because the in-process DAQ queue was full.
    pub channel_overflow_drops: u64,
    /// DTOs naming a DAQ list or ODT that was never allocated.
    pub unplanned_dto: u64,
    /// Bumped on every discontinuity.
    pub generation: u64,
}

/// A backward `CTR` delta larger than this is read as a forward gap rather than reordering,
/// since the 16-bit counter wraps.
const BACKWARD_WINDOW: u16 = 0x8000;

impl LinkStats {
    /// Creates empty statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one received frame's `CTR` and returns what it revealed.
    ///
    /// Arithmetic is wrapping throughout, so the `0xFFFF` → `0x0000` rollover is an ordinary
    /// in-order step and is never counted as loss.
    pub fn observe_ctr(&mut self, ctr: u16) -> CtrObservation {
        self.frames_received = self.frames_received.saturating_add(1);

        let Some(expected) = self.expected_next_ctr else {
            self.expected_next_ctr = Some(ctr.wrapping_add(1));
            return CtrObservation::First;
        };

        let delta = ctr.wrapping_sub(expected);
        let observation = if delta == 0 {
            CtrObservation::InOrder
        } else if delta < BACKWARD_WINDOW {
            // `delta` frames were skipped between the expected one and this one.
            self.gaps_detected = self.gaps_detected.saturating_add(1);
            self.frames_lost_estimate = self.frames_lost_estimate.saturating_add(delta as u64);
            CtrObservation::Gap { lost: delta }
        } else if ctr == expected.wrapping_sub(1) {
            self.duplicated = self.duplicated.saturating_add(1);
            CtrObservation::Duplicate
        } else {
            self.reordered = self.reordered.saturating_add(1);
            CtrObservation::Reordered
        };

        if observation.is_discontinuity() {
            self.generation = self.generation.saturating_add(1);
        }

        // Resynchronize on the frame just seen, so one gap is reported once rather than making
        // every subsequent frame look lost too. A duplicate must not rewind the expectation.
        if observation != CtrObservation::Duplicate {
            self.expected_next_ctr = Some(ctr.wrapping_add(1));
        }
        observation
    }

    /// Records a DTO dropped because the in-process DAQ queue was full.
    pub fn note_channel_overflow(&mut self, dropped: u64) {
        self.channel_overflow_drops = self.channel_overflow_drops.saturating_add(dropped);
    }

    /// Records a DTO naming a DAQ list or ODT that was never allocated.
    pub fn note_unplanned_dto(&mut self) {
        self.unplanned_dto = self.unplanned_dto.saturating_add(1);
    }

    /// Returns `true` when nothing has been lost, reordered, duplicated or dropped.
    pub fn is_clean(&self) -> bool {
        self.gaps_detected == 0
            && self.reordered == 0
            && self.duplicated == 0
            && self.channel_overflow_drops == 0
            && self.unplanned_dto == 0
    }

    /// One-line summary for the status bar and `.sdf` provenance.
    pub fn summary(&self) -> String {
        format!(
            "frames {}, lost ~{} in {} gap(s), reordered {}, duplicated {}, queue drops {}, unplanned {}",
            self.frames_received,
            self.frames_lost_estimate,
            self.gaps_detected,
            self.reordered,
            self.duplicated,
            self.channel_overflow_drops,
            self.unplanned_dto
        )
    }
}
