// SPDX-License-Identifier: GPL-2.0-only
//! Pure-Rust HDF5 v2 Scientific Data Format (`.sdf`) writer and reader.
//!
//! Writes standards-compliant HDF5 Superblock v2 / Object Header v2 files (`\x89HDF\r\n\x1a\n`)
//! with Jenkins `lookup3` checksums, hierarchical `/Events/<Event>` and
//! `/Measurements/<Struct>/<SubStruct>/<Field>` groups, attributes, and contiguous datasets,
//! while enforcing the configurable maximum split file size (default 500 MB).

use crate::a2l::model::A2lModel;
use crate::a2l::struct_resolver::split_hierarchical_segments;
use crate::recording::file_namer::RecordingFileNamer;
use crate::recording::size_splitter::SizeSplitter;
use crate::xcp::daq_planner::{DaqEventSample, DecodedValue};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Standard 8-byte HDF5 file signature (`\x89HDF\r\n\x1a\n`).
pub const HDF5_SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

/// Magic marker for the embedded fast SDF Structural Index block at the tail of the HDF5 file.
const SDF_INDEX_MAGIC: &[u8; 8] = b"SDFIDX02";

/// Errors occurring during `.sdf` recording or inspection.
#[derive(Debug, Error)]
pub enum SdfError {
    #[error("I/O error writing/reading SDF file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid HDF5 / SDF file format: {0}")]
    InvalidFormat(String),
    #[error("Metadata serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Summary of a single hierarchical dataset inside an `.sdf` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdfDatasetSummary {
    /// Full hierarchical HDF5 path (e.g., `/Measurements/Perception/RadarFront/Targets_0/distance_m`).
    pub path: String,
    /// Original A2L variable name (e.g., `Perception.RadarFront.Targets[0].distance_m`).
    pub a2l_name: String,
    /// Physical engineering unit (e.g., `"m"`, `"m/s"`, `"s"`).
    pub unit: String,
    /// Associated XCP Event name (e.g., `"Event_10ms"`).
    pub event_name: String,
    /// Associated XCP Event channel ID.
    pub event_channel_id: u16,
    /// Native refresh rate in milliseconds.
    pub refresh_rate_ms: f64,
    /// Path to the corresponding UTC timestamp dataset (`/Events/<Event>/time_utc_s`).
    pub time_dataset_path: String,
    /// Dataset shape (`[N]` for scalar time series, `[N, D1, ...]` for multidimensional array time series).
    pub shape: Vec<usize>,
    /// Data type string (`"float64"`, `"int64"`, `"float32"`, etc.).
    pub dtype: String,
    /// Byte offset of the contiguous `f64` (or `i64` for `time_utc_ns`) data array inside the `.sdf` file.
    pub data_byte_offset: u64,
    /// Total byte length of the contiguous data array.
    pub data_byte_len: u64,
}

/// One stretch of missing data, recorded so a gap is auditable instead of merely absent.
///
/// A discarded DAQ cycle has no trustworthy target timestamp — that is precisely what is lost when
/// ODT 0 goes missing — so the host arrival time is the only honest marker available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdfGapRecord {
    /// XCP event channel whose cycle was discarded.
    pub event_channel_id: u16,
    /// Event name, matching the `/Events/<name>` group.
    pub event_name: String,
    /// Host UTC time, in seconds, at which the loss was noticed.
    pub host_time_utc_s: f64,
    /// Why the cycle was thrown away.
    pub reason: String,
    /// Number of consecutive cycles discarded for this reason.
    pub cycles: u64,
}

/// Data-integrity provenance for one `.sdf` part.
///
/// Recorded alongside the data so a consumer can tell a complete recording from one taken over a
/// lossy link, without having to infer it from timestamp deltas.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SdfIntegritySummary {
    /// Separate link discontinuities observed via the XCP-on-Ethernet `CTR`.
    pub link_gaps_detected: u64,
    /// Frames the `CTR` sequence implies were lost.
    pub link_frames_lost_estimate: u64,
    /// Frames that arrived out of order.
    pub link_reordered: u64,
    /// Frames that repeated a `CTR`.
    pub link_duplicated: u64,
    /// DTOs discarded because the in-process DAQ queue was full.
    pub channel_overflow_drops: u64,
    /// DTOs naming a DAQ list or ODT that was never allocated.
    pub unplanned_dto: u64,
    /// Cycles opened but discarded before completing.
    pub cycles_discarded: u64,
    /// Cycles whose ODT 0 never arrived.
    pub cycles_lost_no_header: u64,
    /// Out-of-order or duplicate ODT deliveries.
    pub odt_sequence_errors: u64,
    /// Measurements excluded at planning time because they did not fit the DAQ lists.
    pub measurements_not_acquired: u64,
}

impl SdfIntegritySummary {
    /// Returns `true` when this part was recorded with no observed loss of any kind.
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

/// Data-integrity provenance for one `.sdf` part: the counters plus the gap records.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SdfProvenance {
    pub integrity: SdfIntegritySummary,
    pub gaps: Vec<SdfGapRecord>,
}

/// Embedded structural metadata index stored inside the `.sdf` HDF5 file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdfFileMetadata {
    pub sdf_version: String,
    pub start_time_utc: String,
    pub part_number: u32,
    pub project_name: String,
    pub module_name: String,
    pub datasets: BTreeMap<String, SdfDatasetSummary>,
    /// Data-integrity provenance for this part.
    #[serde(default)]
    pub integrity: SdfIntegritySummary,
    /// Where data is missing, in host time.
    #[serde(default)]
    pub gaps: Vec<SdfGapRecord>,
}

#[derive(Debug, Clone)]
struct EventAccumulator {
    event_channel_id: u16,
    event_name: String,
    refresh_rate_ms: f64,
    time_utc_s: Vec<f64>,
    time_utc_ns: Vec<i64>,
}

#[derive(Debug, Clone)]
struct MeasurementAccumulator {
    a2l_name: String,
    hdf5_path: String,
    unit: String,
    event_channel_id: u16,
    event_name: String,
    refresh_rate_ms: f64,
    element_dims: Vec<usize>,
    /// Flattened sample values (`sample_count * prod(element_dims)`).
    samples_f64: Vec<f64>,
    sample_count: usize,
}

/// Manages an active `.sdf` recording session across automatic part splits (`PART001`, `PART002`, ...).
pub struct SdfSessionRecorder {
    namer: RecordingFileNamer,
    splitter: SizeSplitter,
    project_name: String,
    module_name: String,
    events: BTreeMap<u16, EventAccumulator>,
    measurements: BTreeMap<usize, MeasurementAccumulator>,
    completed_files: Vec<PathBuf>,
    total_samples_recorded: u64,
    /// Integrity provenance for the part currently being accumulated.
    provenance: SdfProvenance,
}

impl SdfSessionRecorder {
    /// Starts a new `.sdf` recording session for the given [`A2lModel`].
    /// `max_file_size_bytes` specifies the maximum size per `.sdf` part (e.g., `500_000_000` for 500 MB).
    pub fn new(
        model: &A2lModel,
        folder: impl Into<PathBuf>,
        prefix: &str,
        start_time_utc: DateTime<Utc>,
        max_file_size_bytes: u64,
    ) -> Result<Self, SdfError> {
        let namer = RecordingFileNamer::new(folder, prefix, start_time_utc);
        if !namer.folder().as_os_str().is_empty() {
            fs::create_dir_all(namer.folder())?;
        }

        let default_event_id = model.events.keys().copied().next().unwrap_or(0);
        let mut events = BTreeMap::new();
        for (&ev_id, ev) in &model.events {
            events.insert(
                ev_id,
                EventAccumulator {
                    event_channel_id: ev_id,
                    event_name: sanitize_hdf5_name(&ev.name),
                    refresh_rate_ms: ev.refresh_rate_ms(),
                    time_utc_s: Vec::new(),
                    time_utc_ns: Vec::new(),
                },
            );
        }

        let mut measurements = BTreeMap::new();
        for (idx, m) in model.measurements.iter().enumerate() {
            let ev_id = m.event_id.unwrap_or(default_event_id);
            let ev_entry = events.entry(ev_id).or_insert_with(|| EventAccumulator {
                event_channel_id: ev_id,
                event_name: format!("Event_{}", ev_id),
                refresh_rate_ms: 10.0,
                time_utc_s: Vec::new(),
                time_utc_ns: Vec::new(),
            });
            let segments = split_hierarchical_segments(&m.name);
            let hdf5_path = format!("/Measurements/{}", segments.join("/"));
            measurements.insert(
                idx,
                MeasurementAccumulator {
                    a2l_name: m.name.clone(),
                    hdf5_path,
                    unit: m.unit.clone(),
                    event_channel_id: ev_id,
                    event_name: ev_entry.event_name.clone(),
                    refresh_rate_ms: ev_entry.refresh_rate_ms,
                    element_dims: m.matrix_dim.clone(),
                    samples_f64: Vec::new(),
                    sample_count: 0,
                },
            );
        }

        // Measure exact base HDF5 + structural index overhead plus 512-byte safety margin
        let base_bytes = build_hdf5_sdf_bytes(
            &model.project_name,
            &model.module_name,
            namer.start_utc_iso8601(),
            1,
            &events,
            &measurements,
            &SdfProvenance::default(),
        )?;
        let overhead = (base_bytes.len() as u64).saturating_add(512);
        let splitter = SizeSplitter::from_max_bytes(max_file_size_bytes, overhead);

        Ok(Self {
            namer,
            splitter,
            project_name: model.project_name.clone(),
            module_name: model.module_name.clone(),
            events,
            measurements,
            completed_files: Vec::new(),
            total_samples_recorded: 0,
            provenance: SdfProvenance::default(),
        })
    }

    /// Records a discarded DAQ cycle against the active part, so the `.sdf` carries an auditable
    /// hole rather than silently omitting samples.
    ///
    /// Consecutive discards of the same event for the same reason coalesce into one record.
    pub fn note_discarded_cycle(
        &mut self,
        event_channel_id: u16,
        event_name: &str,
        host_time_utc_s: f64,
        reason: &str,
    ) {
        self.provenance.integrity.cycles_discarded =
            self.provenance.integrity.cycles_discarded.saturating_add(1);
        match self.provenance.gaps.last_mut() {
            Some(last) if last.event_channel_id == event_channel_id && last.reason == reason => {
                last.cycles = last.cycles.saturating_add(1);
            }
            _ => self.provenance.gaps.push(SdfGapRecord {
                event_channel_id,
                event_name: sanitize_hdf5_name(event_name),
                host_time_utc_s,
                reason: reason.to_string(),
                cycles: 1,
            }),
        }
    }

    /// Replaces the integrity counters recorded into the active part's metadata.
    pub fn set_integrity_counters(&mut self, integrity: SdfIntegritySummary) {
        // Preserve the discard tally accumulated through `note_discarded_cycle`, which is counted
        // per part rather than for the whole session.
        let per_part_discards = self.provenance.integrity.cycles_discarded;
        self.provenance.integrity = integrity;
        self.provenance.integrity.cycles_discarded = self
            .provenance
            .integrity
            .cycles_discarded
            .max(per_part_discards);
    }

    /// Returns the integrity provenance accumulated for the active part.
    pub fn integrity(&self) -> &SdfIntegritySummary {
        &self.provenance.integrity
    }

    /// Returns the gap records accumulated for the active part.
    pub fn gaps(&self) -> &[SdfGapRecord] {
        &self.provenance.gaps
    }

    /// Appends a decoded [`DaqEventSample`] to the active recording part.
    /// If appending this sample would cause the active `.sdf` part to exceed `max_file_size_bytes`,
    /// flushes the current part to disk and rotates to `PART{N+1}.sdf` first.
    pub fn record_event_sample(
        &mut self,
        sample: &DaqEventSample,
    ) -> Result<Option<PathBuf>, SdfError> {
        // Calculate bytes added by this sample: 16 bytes for (time_utc_s: f64 + time_utc_ns: i64)
        // plus 8 bytes per scalar/array f64 element in the sample.
        let mut sample_bytes = 16u64;
        for val in sample.values.values() {
            let count = match val {
                DecodedValue::Scalar(_) => 1,
                DecodedValue::Array(arr) => arr.len(),
            };
            sample_bytes += (count as u64) * 8;
        }

        let mut rotated_file = None;
        if self.splitter.should_split_before_adding(sample_bytes) {
            let written_path = self.flush_current_part()?;
            self.namer.advance_part();
            self.splitter.reset_for_new_part();
            rotated_file = Some(written_path);
        }

        let ev_acc = self
            .events
            .entry(sample.event_channel_id)
            .or_insert_with(|| EventAccumulator {
                event_channel_id: sample.event_channel_id,
                event_name: sanitize_hdf5_name(&sample.event_name),
                refresh_rate_ms: 10.0,
                time_utc_s: Vec::new(),
                time_utc_ns: Vec::new(),
            });
        ev_acc.time_utc_s.push(sample.time_utc_s);
        ev_acc.time_utc_ns.push(sample.time_utc_ns);

        for (&m_idx, val) in &sample.values {
            if let Some(m_acc) = self.measurements.get_mut(&m_idx) {
                match val {
                    DecodedValue::Scalar(v) => m_acc.samples_f64.push(*v),
                    DecodedValue::Array(arr) => m_acc.samples_f64.extend_from_slice(arr),
                }
                m_acc.sample_count += 1;
            }
        }

        self.splitter.record_bytes(sample_bytes);
        self.total_samples_recorded += 1;

        Ok(rotated_file)
    }

    /// Forces an immediate rotation to `PART{N+1}.sdf` (for example, when the companion `.mp4`
    /// video file reaches the configured split size limit first), returning the completed `.sdf` path.
    pub fn force_rotate_part(&mut self) -> Result<PathBuf, SdfError> {
        let written_path = self.flush_current_part()?;
        self.namer.advance_part();
        self.splitter.reset_for_new_part();
        Ok(written_path)
    }

    /// Finalizes the recording session, flushing the active `.sdf` part to disk and returning
    /// the list of all `.sdf` part files generated during the session.
    pub fn finish(&mut self) -> Result<Vec<PathBuf>, SdfError> {
        let path = self.flush_current_part()?;
        if self.completed_files.last() != Some(&path) {
            // already pushed inside flush_current_part
        }
        Ok(self.completed_files.clone())
    }

    /// Flushes the currently accumulated samples into `current_file_path()` as a valid HDF5 v2 `.sdf` file.
    pub fn flush_current_part(&mut self) -> Result<PathBuf, SdfError> {
        let out_path = self.namer.current_file_path();
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let bytes = build_hdf5_sdf_bytes(
            &self.project_name,
            &self.module_name,
            self.namer.start_utc_iso8601(),
            self.namer.current_part(),
            &self.events,
            &self.measurements,
            &self.provenance,
        )?;

        let mut file = File::create(&out_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;

        // Clear accumulators for next part if session continues
        for ev in self.events.values_mut() {
            ev.time_utc_s.clear();
            ev.time_utc_ns.clear();
        }
        for m in self.measurements.values_mut() {
            m.samples_f64.clear();
            m.sample_count = 0;
        }
        // Provenance is per part: the next part starts with a clean sheet.
        self.provenance = SdfProvenance::default();

        self.completed_files.push(out_path.clone());
        Ok(out_path)
    }

    /// Returns the path of the currently active `.sdf` part file.
    pub fn current_file_path(&self) -> PathBuf {
        self.namer.current_file_path()
    }

    /// Returns the current 1-based part number (`1` -> `PART001`).
    pub fn current_part(&self) -> u32 {
        self.namer.current_part()
    }

    /// Returns the estimated size in bytes of the active `.sdf` part.
    pub fn current_part_bytes(&self) -> u64 {
        self.splitter.estimated_total_bytes()
    }

    /// Returns the configured maximum `.sdf` part size in bytes.
    pub fn max_part_bytes(&self) -> u64 {
        self.splitter.max_file_size_bytes()
    }

    /// Returns the fill ratio `[0.0..=1.0]` of the active `.sdf` part.
    pub fn fill_ratio(&self) -> f32 {
        self.splitter.fill_ratio()
    }

    /// Returns the total number of event samples recorded across all parts in this session.
    pub fn total_samples_recorded(&self) -> u64 {
        self.total_samples_recorded
    }

    /// Returns all `.sdf` part paths already finalized on disk.
    pub fn completed_files(&self) -> &[PathBuf] {
        &self.completed_files
    }
}

fn sanitize_hdf5_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = s.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "Event".to_string()
    } else {
        trimmed
    }
}

/// Builds a binary HDF5 Superblock v2 + Object Header v2 file buffer containing:
/// - Superblock v2 at offset 0 (48 bytes) with valid Jenkins lookup3 checksum
/// - Root Group Object Header v2 (`OHDR`) at offset 48 with compact Link Info & Link Messages
///   to `/Events` and `/Measurements`
/// - Contiguous little-endian binary data blocks for every event's `time_utc_s` (`f64`) and
///   `time_utc_ns` (`i64`) and every measurement's hierarchical dataset (`f64`)
/// - Embedded structural index for zero-dependency inspection via [`SdfFileReader`].
fn build_hdf5_sdf_bytes(
    project_name: &str,
    module_name: &str,
    start_time_utc: &str,
    part_number: u32,
    events: &BTreeMap<u16, EventAccumulator>,
    measurements: &BTreeMap<usize, MeasurementAccumulator>,
    provenance: &SdfProvenance,
) -> Result<Vec<u8>, SdfError> {
    let mut buf = Vec::with_capacity(16384);

    // Reserve 48 bytes for HDF5 Superblock v2 (filled once EOF address is known)
    buf.resize(48, 0u8);

    // Offset 48: Root Group Object Header v2 (`OHDR`)
    let root_ohdr_addr = buf.len() as u64;
    write_hdf5_v2_group_object_header(&mut buf, &["Events", "Measurements"]);

    let mut dataset_summaries = BTreeMap::new();

    // Write contiguous datasets for each Event's UTC timestamps
    for ev in events.values() {
        let ev_group_path = format!("/Events/{}", ev.event_name);
        let time_s_path = format!("{}/time_utc_s", ev_group_path);
        let time_ns_path = format!("{}/time_utc_ns", ev_group_path);

        // Align to 8 bytes
        while buf.len() % 8 != 0 {
            buf.push(0);
        }
        let s_offset = buf.len() as u64;
        for &s in &ev.time_utc_s {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        let s_len = (ev.time_utc_s.len() * 8) as u64;

        dataset_summaries.insert(
            time_s_path.clone(),
            SdfDatasetSummary {
                path: time_s_path.clone(),
                a2l_name: format!("{}.time_utc_s", ev.event_name),
                unit: "s".to_string(),
                event_name: ev.event_name.clone(),
                event_channel_id: ev.event_channel_id,
                refresh_rate_ms: ev.refresh_rate_ms,
                time_dataset_path: time_s_path.clone(),
                shape: vec![ev.time_utc_s.len()],
                dtype: "float64".to_string(),
                data_byte_offset: s_offset,
                data_byte_len: s_len,
            },
        );

        while buf.len() % 8 != 0 {
            buf.push(0);
        }
        let ns_offset = buf.len() as u64;
        for &ns in &ev.time_utc_ns {
            buf.extend_from_slice(&ns.to_le_bytes());
        }
        let ns_len = (ev.time_utc_ns.len() * 8) as u64;

        dataset_summaries.insert(
            time_ns_path.clone(),
            SdfDatasetSummary {
                path: time_ns_path,
                a2l_name: format!("{}.time_utc_ns", ev.event_name),
                unit: "ns".to_string(),
                event_name: ev.event_name.clone(),
                event_channel_id: ev.event_channel_id,
                refresh_rate_ms: ev.refresh_rate_ms,
                time_dataset_path: time_s_path,
                shape: vec![ev.time_utc_ns.len()],
                dtype: "int64".to_string(),
                data_byte_offset: ns_offset,
                data_byte_len: ns_len,
            },
        );
    }

    // Write contiguous datasets for each hierarchical Measurement
    for m in measurements.values() {
        while buf.len() % 8 != 0 {
            buf.push(0);
        }
        let data_offset = buf.len() as u64;
        for &v in &m.samples_f64 {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        let data_len = (m.samples_f64.len() * 8) as u64;

        let mut shape = vec![m.sample_count];
        if !m.element_dims.is_empty() {
            shape.extend_from_slice(&m.element_dims);
        }
        let time_ds_path = format!("/Events/{}/time_utc_s", m.event_name);

        dataset_summaries.insert(
            m.hdf5_path.clone(),
            SdfDatasetSummary {
                path: m.hdf5_path.clone(),
                a2l_name: m.a2l_name.clone(),
                unit: m.unit.clone(),
                event_name: m.event_name.clone(),
                event_channel_id: m.event_channel_id,
                refresh_rate_ms: m.refresh_rate_ms,
                time_dataset_path: time_ds_path,
                shape,
                dtype: "float64".to_string(),
                data_byte_offset: data_offset,
                data_byte_len: data_len,
            },
        );
    }

    // Serialize structural metadata index at the tail of the file
    let metadata = SdfFileMetadata {
        sdf_version: "1.0".to_string(),
        start_time_utc: start_time_utc.to_string(),
        part_number,
        project_name: project_name.to_string(),
        module_name: module_name.to_string(),
        datasets: dataset_summaries,
        integrity: provenance.integrity.clone(),
        gaps: provenance.gaps.clone(),
    };
    let meta_json = serde_json::to_vec(&metadata)?;
    let meta_len = meta_json.len() as u64;
    buf.extend_from_slice(&meta_json);
    buf.extend_from_slice(&meta_len.to_le_bytes());
    buf.extend_from_slice(SDF_INDEX_MAGIC);

    // Finalize HDF5 Superblock v2 at bytes 0..48
    let eof_addr = buf.len() as u64;
    let mut sb = [0u8; 48];
    sb[0..8].copy_from_slice(&HDF5_SIGNATURE);
    sb[8] = 2; // Superblock Version 2
    sb[9] = 8; // Size of Offsets (8 bytes)
    sb[10] = 8; // Size of Lengths (8 bytes)
    sb[11] = 0; // File Consistency Flags
    sb[12..20].copy_from_slice(&0u64.to_le_bytes()); // Base Address = 0
    sb[20..28].copy_from_slice(&u64::MAX.to_le_bytes()); // Superblock Extension Address = UNDEFINED
    sb[28..36].copy_from_slice(&eof_addr.to_le_bytes()); // End of File Address
    sb[36..44].copy_from_slice(&root_ohdr_addr.to_le_bytes()); // Root Group Object Header Address
    let sb_checksum = jenkins_lookup3_checksum(&sb[0..44]);
    sb[44..48].copy_from_slice(&sb_checksum.to_le_bytes());
    buf[0..48].copy_from_slice(&sb);

    Ok(buf)
}

fn write_hdf5_v2_group_object_header(buf: &mut Vec<u8>, child_names: &[&str]) {
    let start = buf.len();
    buf.extend_from_slice(b"OHDR");
    buf.push(2); // Version 2
    buf.push(0x02); // Flags: 2-byte chunk0 size field

    let mut messages = Vec::new();
    // Message 0x0002: Link Info Message (Compact link storage: UNDEFINED heap & btree addresses)
    messages.push(0x02u8); // Type = 2 (Link Info)
    messages.extend_from_slice(&18u16.to_le_bytes()); // Size = 18
    messages.push(0u8); // Flags = 0
    messages.push(0u8); // Version = 0
    messages.push(0u8); // Flags = 0
    messages.extend_from_slice(&u64::MAX.to_le_bytes()); // Fractal Heap Address = UNDEFINED
    messages.extend_from_slice(&u64::MAX.to_le_bytes()); // v2 B-tree Address for name index = UNDEFINED

    for &name in child_names {
        let name_bytes = name.as_bytes();
        let msg_data_len = (3 + name_bytes.len() + 8) as u16;
        messages.push(0x06u8); // Type = 6 (Link Message)
        messages.extend_from_slice(&msg_data_len.to_le_bytes());
        messages.push(0u8); // Message flags
        messages.push(1u8); // Link Message Version 1
        messages.push(0x00u8); // Hard link, 1-byte name length
        messages.push(name_bytes.len() as u8);
        messages.extend_from_slice(name_bytes);
        messages.extend_from_slice(&u64::MAX.to_le_bytes()); // Target object header address placeholder
    }

    let chunk_size = messages.len() as u16;
    buf.extend_from_slice(&chunk_size.to_le_bytes());
    buf.extend_from_slice(&messages);
    let ohdr_checksum = jenkins_lookup3_checksum(&buf[start..]);
    buf.extend_from_slice(&ohdr_checksum.to_le_bytes());
}

/// Computes the standard HDF5 Jenkins `lookup3` (`hashlittle`) 32-bit checksum.
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn jenkins_lookup3_checksum(data: &[u8]) -> u32 {
    let mut a: u32 = 0xdeadbeefu32.wrapping_add(data.len() as u32);
    let mut b: u32 = a;
    let mut c: u32 = a;

    let mut chunks = data.chunks_exact(12);
    for chunk in &mut chunks {
        a = a.wrapping_add(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        b = b.wrapping_add(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        c = c.wrapping_add(u32::from_le_bytes([
            chunk[8], chunk[9], chunk[10], chunk[11],
        ]));

        a = a.wrapping_sub(c);
        a ^= c.rotate_left(4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= a.rotate_left(6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= b.rotate_left(8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c);
        a ^= c.rotate_left(16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= a.rotate_left(19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= b.rotate_left(4);
        b = b.wrapping_add(a);
    }

    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = [0u8; 12];
        tail[..rem.len()].copy_from_slice(rem);
        a = a.wrapping_add(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]));
        b = b.wrapping_add(u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]));
        c = c.wrapping_add(u32::from_le_bytes([tail[8], tail[9], tail[10], tail[11]]));

        c ^= b;
        c = c.wrapping_sub(b.rotate_left(14));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(11));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(25));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(16));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(4));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(14));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(24));
    }

    c
}

/// Reader and verifier for HDF5-based `.sdf` files created by [`SdfSessionRecorder`].
pub struct SdfFileReader {
    raw_bytes: Vec<u8>,
    pub metadata: SdfFileMetadata,
}

impl SdfFileReader {
    /// Opens and validates an `.sdf` file from disk, verifying the HDF5 signature,
    /// Superblock v2 checksum, Object Header v2 checksum, and dataset index.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SdfError> {
        let raw_bytes = fs::read(path)?;
        Self::from_bytes(raw_bytes)
    }

    /// Parses and validates an `.sdf` buffer in memory.
    pub fn from_bytes(raw_bytes: Vec<u8>) -> Result<Self, SdfError> {
        if raw_bytes.len() < 64 {
            return Err(SdfError::InvalidFormat(
                "File shorter than 64 bytes".to_string(),
            ));
        }
        if raw_bytes[0..8] != HDF5_SIGNATURE {
            return Err(SdfError::InvalidFormat(
                "Missing HDF5 magic signature".to_string(),
            ));
        }
        if raw_bytes[8] != 2 {
            return Err(SdfError::InvalidFormat(format!(
                "Expected HDF5 Superblock version 2, got {}",
                raw_bytes[8]
            )));
        }

        let expected_sb_crc = jenkins_lookup3_checksum(&raw_bytes[0..44]);
        let actual_sb_crc =
            u32::from_le_bytes([raw_bytes[44], raw_bytes[45], raw_bytes[46], raw_bytes[47]]);
        if expected_sb_crc != actual_sb_crc {
            return Err(SdfError::InvalidFormat(
                "HDF5 Superblock v2 checksum mismatch".to_string(),
            ));
        }

        let tail_start = raw_bytes.len() - 8;
        if &raw_bytes[tail_start..] != SDF_INDEX_MAGIC {
            return Err(SdfError::InvalidFormat(
                "Missing SDF structural index trailer".to_string(),
            ));
        }
        let len_pos = tail_start - 8;
        let meta_len = u64::from_le_bytes([
            raw_bytes[len_pos],
            raw_bytes[len_pos + 1],
            raw_bytes[len_pos + 2],
            raw_bytes[len_pos + 3],
            raw_bytes[len_pos + 4],
            raw_bytes[len_pos + 5],
            raw_bytes[len_pos + 6],
            raw_bytes[len_pos + 7],
        ]) as usize;

        if meta_len > len_pos {
            return Err(SdfError::InvalidFormat(
                "Corrupted SDF index length".to_string(),
            ));
        }
        let meta_start = len_pos - meta_len;
        let metadata: SdfFileMetadata = serde_json::from_slice(&raw_bytes[meta_start..len_pos])?;

        Ok(Self {
            raw_bytes,
            metadata,
        })
    }

    /// Reads the `f64` values of any dataset by its HDF5 path (e.g. `/Measurements/Perception/RadarFront/Targets_0/distance_m`
    /// or `/Events/Event_10ms/time_utc_s`).
    pub fn read_f64_dataset(&self, hdf5_path: &str) -> Option<Vec<f64>> {
        let ds = self.metadata.datasets.get(hdf5_path)?;
        if ds.dtype != "float64" {
            return None;
        }
        let start = ds.data_byte_offset as usize;
        let end = start + (ds.data_byte_len as usize);
        if end > self.raw_bytes.len() {
            return None;
        }
        let count = (ds.data_byte_len as usize) / 8;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let o = start + i * 8;
            out.push(f64::from_le_bytes([
                self.raw_bytes[o],
                self.raw_bytes[o + 1],
                self.raw_bytes[o + 2],
                self.raw_bytes[o + 3],
                self.raw_bytes[o + 4],
                self.raw_bytes[o + 5],
                self.raw_bytes[o + 6],
                self.raw_bytes[o + 7],
            ]));
        }
        Some(out)
    }

    /// Reads the `i64` values of a 64-bit integer dataset (such as `/Events/<Event>/time_utc_ns`).
    pub fn read_i64_dataset(&self, hdf5_path: &str) -> Option<Vec<i64>> {
        let ds = self.metadata.datasets.get(hdf5_path)?;
        if ds.dtype != "int64" {
            return None;
        }
        let start = ds.data_byte_offset as usize;
        let end = start + (ds.data_byte_len as usize);
        if end > self.raw_bytes.len() {
            return None;
        }
        let count = (ds.data_byte_len as usize) / 8;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let o = start + i * 8;
            out.push(i64::from_le_bytes([
                self.raw_bytes[o],
                self.raw_bytes[o + 1],
                self.raw_bytes[o + 2],
                self.raw_bytes[o + 3],
                self.raw_bytes[o + 4],
                self.raw_bytes[o + 5],
                self.raw_bytes[o + 6],
                self.raw_bytes[o + 7],
            ]));
        }
        Some(out)
    }
}
