// SPDX-License-Identifier: GPL-2.0-only
//! `sdf_inspect`: open an `.sdf` part with the library reader and run structural checks.

use crate::error::{DevtoolError, DevtoolResult};
use open_xcp_client::recording::{SdfFileReader, SdfGapRecord, SdfIntegritySummary};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Input of `sdf_inspect`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SdfInspectInput {
    /// Path to one `.sdf` part file.
    pub path: String,
    /// Also read the time datasets and check monotonicity (default true).
    #[serde(default = "default_true")]
    pub check_data: bool,
    /// Maximum number of dataset entries returned (default 200).
    #[serde(default = "default_max_datasets")]
    pub max_datasets: usize,
}

fn default_true() -> bool {
    true
}
fn default_max_datasets() -> usize {
    200
}

/// One named structural check.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NamedCheck {
    pub name: String,
    pub pass: bool,
    pub detail: String,
}

/// Per-event summary.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SdfEventSummary {
    pub name: String,
    pub channel_id: u16,
    pub dataset_count: usize,
    pub sample_count: Option<usize>,
    pub time_monotonic: Option<bool>,
}

/// One dataset entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SdfDatasetEntry {
    pub path: String,
    pub a2l_name: String,
    pub unit: String,
    pub event_name: String,
    pub shape: Vec<usize>,
    pub dtype: String,
    pub byte_len: u64,
}

/// Serializable mirror of the library's integrity counters.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IntegrityOut {
    pub is_clean: bool,
    pub link_gaps_detected: u64,
    pub link_frames_lost_estimate: u64,
    pub link_reordered: u64,
    pub link_duplicated: u64,
    pub channel_overflow_drops: u64,
    pub unplanned_dto: u64,
    pub cycles_discarded: u64,
    pub cycles_lost_no_header: u64,
    pub odt_sequence_errors: u64,
    pub measurements_not_acquired: u64,
}

impl From<&SdfIntegritySummary> for IntegrityOut {
    fn from(i: &SdfIntegritySummary) -> Self {
        Self {
            is_clean: i.is_clean(),
            link_gaps_detected: i.link_gaps_detected,
            link_frames_lost_estimate: i.link_frames_lost_estimate,
            link_reordered: i.link_reordered,
            link_duplicated: i.link_duplicated,
            channel_overflow_drops: i.channel_overflow_drops,
            unplanned_dto: i.unplanned_dto,
            cycles_discarded: i.cycles_discarded,
            cycles_lost_no_header: i.cycles_lost_no_header,
            odt_sequence_errors: i.odt_sequence_errors,
            measurements_not_acquired: i.measurements_not_acquired,
        }
    }
}

/// Serializable mirror of a gap record.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GapOut {
    pub event_channel_id: u16,
    pub event_name: String,
    pub host_time_utc_s: f64,
    pub reason: String,
    pub cycles: u64,
}

impl From<&SdfGapRecord> for GapOut {
    fn from(g: &SdfGapRecord) -> Self {
        Self {
            event_channel_id: g.event_channel_id,
            event_name: g.event_name.clone(),
            host_time_utc_s: g.host_time_utc_s,
            reason: g.reason.clone(),
            cycles: g.cycles,
        }
    }
}

/// Output of `sdf_inspect`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SdfInspectOutput {
    pub ok: bool,
    pub path: String,
    pub file_bytes: u64,
    pub sdf_version: String,
    pub part_number: u32,
    pub start_time_utc: String,
    pub project_name: String,
    pub module_name: String,
    pub dataset_count: usize,
    pub events: Vec<SdfEventSummary>,
    pub datasets: Vec<SdfDatasetEntry>,
    pub datasets_truncated: bool,
    pub integrity: Option<IntegrityOut>,
    pub gaps: Vec<GapOut>,
    pub checks: Vec<NamedCheck>,
    pub problems: Vec<String>,
}

fn dtype_width(dtype: &str) -> Option<u64> {
    match dtype {
        "float64" | "int64" | "uint64" => Some(8),
        "float32" | "int32" | "uint32" => Some(4),
        "int16" | "uint16" => Some(2),
        "int8" | "uint8" => Some(1),
        _ => None,
    }
}

fn part_from_filename(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    let idx = stem.rfind("_PART")?;
    stem[idx + 5..].parse().ok()
}

/// Runs `sdf_inspect`.
pub fn inspect(input: &SdfInspectInput) -> DevtoolResult<SdfInspectOutput> {
    let path = Path::new(&input.path);
    if !path.is_file() {
        return Err(DevtoolError::Invalid(format!(
            "SDF file not found: {}",
            input.path
        )));
    }
    let file_bytes = std::fs::metadata(path)?.len();
    let mut checks = Vec::new();
    let mut problems = Vec::new();

    let reader = match SdfFileReader::open(path) {
        Ok(r) => r,
        Err(e) => {
            checks.push(NamedCheck {
                name: "hdf5_signature_checksums_and_tail_index".into(),
                pass: false,
                detail: e.to_string(),
            });
            problems.push(format!("reader rejected the file: {e}"));
            return Ok(SdfInspectOutput {
                ok: false,
                path: input.path.clone(),
                file_bytes,
                sdf_version: String::new(),
                part_number: 0,
                start_time_utc: String::new(),
                project_name: String::new(),
                module_name: String::new(),
                dataset_count: 0,
                events: Vec::new(),
                datasets: Vec::new(),
                datasets_truncated: false,
                integrity: None,
                gaps: Vec::new(),
                checks,
                problems,
            });
        }
    };
    checks.push(NamedCheck {
        name: "hdf5_signature_checksums_and_tail_index".into(),
        pass: true,
        detail: "SdfFileReader::open accepted the superblock, checksums and SDFIDX02 tail".into(),
    });
    let meta = &reader.metadata;

    // Dataset ranges and shapes.
    let mut range_bad = Vec::new();
    let mut shape_bad = Vec::new();
    for (p, d) in &meta.datasets {
        if d.data_byte_offset.saturating_add(d.data_byte_len) > file_bytes {
            range_bad.push(p.clone());
        }
        if let Some(w) = dtype_width(&d.dtype) {
            let n: u64 = d.shape.iter().map(|&s| s as u64).product();
            if n * w != d.data_byte_len {
                shape_bad.push(format!("{p} ({:?} x {w} != {})", d.shape, d.data_byte_len));
            }
        }
    }
    checks.push(NamedCheck {
        name: "dataset_ranges_within_file".into(),
        pass: range_bad.is_empty(),
        detail: if range_bad.is_empty() {
            format!("{} datasets", meta.datasets.len())
        } else {
            range_bad.join(", ")
        },
    });
    checks.push(NamedCheck {
        name: "shape_matches_byte_len".into(),
        pass: shape_bad.is_empty(),
        detail: shape_bad.join(", "),
    });

    // Events and time datasets.
    let mut per_event: BTreeMap<(String, u16), Vec<String>> = BTreeMap::new();
    for (p, d) in &meta.datasets {
        per_event
            .entry((d.event_name.clone(), d.event_channel_id))
            .or_default()
            .push(p.clone());
    }
    let mut missing_time = Vec::new();
    let mut non_monotonic = Vec::new();
    let mut events = Vec::new();
    for ((name, ch), paths) in &per_event {
        let time_path = meta
            .datasets
            .get(&paths[0])
            .map(|d| d.time_dataset_path.clone())
            .unwrap_or_default();
        let (sample_count, time_monotonic) = if input.check_data {
            match reader.read_f64_dataset(&time_path) {
                Some(t) => {
                    let mono = t.windows(2).all(|w| w[1] >= w[0]);
                    if !mono {
                        non_monotonic.push(name.clone());
                    }
                    (Some(t.len()), Some(mono))
                }
                None => {
                    missing_time.push(name.clone());
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        events.push(SdfEventSummary {
            name: name.clone(),
            channel_id: *ch,
            dataset_count: paths.len(),
            sample_count,
            time_monotonic,
        });
    }
    if input.check_data {
        checks.push(NamedCheck {
            name: "every_event_has_time_dataset".into(),
            pass: missing_time.is_empty(),
            detail: missing_time.join(", "),
        });
        checks.push(NamedCheck {
            name: "time_utc_s_monotonic".into(),
            pass: non_monotonic.is_empty(),
            detail: non_monotonic.join(", "),
        });
    }

    // Filename part number.
    let fname_part = part_from_filename(path);
    checks.push(NamedCheck {
        name: "filename_part_matches_metadata".into(),
        pass: fname_part == Some(meta.part_number),
        detail: format!(
            "filename PART{:?} vs metadata part {}",
            fname_part, meta.part_number
        ),
    });

    for c in checks.iter().filter(|c| !c.pass) {
        problems.push(format!("check failed: {} ({})", c.name, c.detail));
    }
    if !meta.integrity.is_clean() {
        problems.push("recording reports data loss (see integrity counters)".to_string());
    }

    let mut datasets: Vec<SdfDatasetEntry> = meta
        .datasets
        .values()
        .map(|d| SdfDatasetEntry {
            path: d.path.clone(),
            a2l_name: d.a2l_name.clone(),
            unit: d.unit.clone(),
            event_name: d.event_name.clone(),
            shape: d.shape.clone(),
            dtype: d.dtype.clone(),
            byte_len: d.data_byte_len,
        })
        .collect();
    let datasets_truncated = datasets.len() > input.max_datasets;
    datasets.truncate(input.max_datasets);

    Ok(SdfInspectOutput {
        ok: checks.iter().all(|c| c.pass),
        path: input.path.clone(),
        file_bytes,
        sdf_version: meta.sdf_version.clone(),
        part_number: meta.part_number,
        start_time_utc: meta.start_time_utc.clone(),
        project_name: meta.project_name.clone(),
        module_name: meta.module_name.clone(),
        dataset_count: meta.datasets.len(),
        events,
        datasets,
        datasets_truncated,
        integrity: Some(IntegrityOut::from(&meta.integrity)),
        gaps: meta.gaps.iter().map(GapOut::from).collect(),
        checks,
        problems,
    })
}
