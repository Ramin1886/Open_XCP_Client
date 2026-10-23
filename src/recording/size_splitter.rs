// SPDX-License-Identifier: GPL-2.0-only
//! Tracks the byte footprint of an active `.sdf` recording part and determines when
//! to rotate to the next `PARTxxx.sdf` file so no split file ever exceeds the configured
//! maximum size (default 500 MB from `settings.json`).

/// Tracks current part byte size against a hard maximum byte ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeSplitter {
    /// Hard maximum file size in bytes (e.g., 500_000_000 bytes for 500 MB).
    max_file_size_bytes: u64,
    /// Reserved overhead bytes for HDF5 object headers, group links, and metadata tables.
    header_overhead_bytes: u64,
    /// Accumulated raw dataset payload bytes in the current part.
    current_payload_bytes: u64,
}

impl SizeSplitter {
    /// Creates a new `SizeSplitter` for a given maximum size in megabytes (`1 MB = 1_000_000 bytes`).
    pub fn from_max_mb(max_size_mb: u64, estimated_header_overhead_bytes: u64) -> Self {
        let max_bytes = max_size_mb.max(1).saturating_mul(1_000_000);
        Self::from_max_bytes(max_bytes, estimated_header_overhead_bytes)
    }

    /// Creates a new `SizeSplitter` with an explicit byte limit (useful for fine-grained tests).
    pub fn from_max_bytes(max_file_size_bytes: u64, estimated_header_overhead_bytes: u64) -> Self {
        let clamped_max = max_file_size_bytes.max(8_192);
        let overhead = estimated_header_overhead_bytes.min(clamped_max.saturating_sub(512));
        Self {
            max_file_size_bytes: clamped_max,
            header_overhead_bytes: overhead,
            current_payload_bytes: 0,
        }
    }

    /// Returns the configured maximum file size in bytes.
    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_bytes
    }

    /// Returns the estimated total size in bytes of the current part if flushed now.
    pub fn estimated_total_bytes(&self) -> u64 {
        self.header_overhead_bytes
            .saturating_add(self.current_payload_bytes)
    }

    /// Returns the current payload bytes accumulated in this part.
    pub fn current_payload_bytes(&self) -> u64 {
        self.current_payload_bytes
    }

    /// Returns `true` if adding `additional_bytes` would cause the file to exceed `max_file_size_bytes`
    /// and the current part already contains at least one sample (`current_payload_bytes > 0`).
    pub fn should_split_before_adding(&self, additional_bytes: u64) -> bool {
        if self.current_payload_bytes == 0 {
            return false;
        }
        self.estimated_total_bytes()
            .saturating_add(additional_bytes)
            > self.max_file_size_bytes
    }

    /// Records `added_bytes` into the current part counter.
    pub fn record_bytes(&mut self, added_bytes: u64) {
        self.current_payload_bytes = self.current_payload_bytes.saturating_add(added_bytes);
    }

    /// Resets the current part payload byte counter after rotating to a new part file.
    pub fn reset_for_new_part(&mut self) {
        self.current_payload_bytes = 0;
    }

    /// Returns the fill ratio `[0.0 ..= 1.0]` of the current part for UI progress bars.
    pub fn fill_ratio(&self) -> f32 {
        (self.estimated_total_bytes() as f64 / self.max_file_size_bytes as f64).clamp(0.0, 1.0)
            as f32
    }
}
