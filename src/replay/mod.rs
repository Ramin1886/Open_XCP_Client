// SPDX-License-Identifier: GPL-2.0-only
//! Multi-part `.sdf` recording replay engine.
//!
//! Loads one or multiple `.sdf` part files (`PART001.sdf`, `PART002.sdf`, ...) created by
//! [`crate::recording::SdfSessionRecorder`], reconstructs the [`A2lModel`] and chronological
//! [`DaqEventSample`] timeline, and provides interactive `Play`, `Pause`, `Stop`, speed scaling
//! (`0.25×`..`10×` and `max`), and time-slider seeking with 5-second pre-roll state reconstruction.

use crate::a2l::model::{A2lDataType, A2lEvent, A2lModel, CompuMethod, CompuType, Measurement};
use crate::a2l::struct_resolver::build_measurement_hierarchy;
use crate::recording::sdf_writer::{SdfError, SdfFileReader};
use crate::xcp::daq_planner::{DaqEventSample, DecodedValue};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Samples emitted per tick at the unbounded ("max") replay speed; bounds UI work per frame.
const MAX_SPEED_BATCH_SAMPLES: usize = 250;

/// Pre-roll window (in seconds) emitted when seeking so all plots and numerical displays
/// immediately reflect the signal state at the target seek timestamp.
pub const SEEK_PREROLL_SECONDS: f64 = 5.0;

/// Supported replay speed multipliers (`0.0` represents `max` unthrottled speed).
pub const REPLAY_SPEEDS: &[(f64, &str)] = &[
    (0.25, "0.25×"),
    (0.5, "0.5×"),
    (1.0, "1×"),
    (2.0, "2×"),
    (5.0, "5×"),
    (10.0, "10×"),
    (0.0, "max"),
];

/// Stateful `.sdf` replay player supporting single-file and multi-part (`PART001`, `PART002`, ...)
/// playback and time-slider scrubbing.
#[derive(Debug, Clone)]
pub struct SdfReplayPlayer {
    loaded_files: Vec<PathBuf>,
    a2l_model: A2lModel,
    samples: Vec<DaqEventSample>,
    start_utc_s: f64,
    end_utc_s: f64,
    duration_s: f64,
    position_s: f64,
    cursor_index: usize,
    playing: bool,
    speed: f64,
}

impl SdfReplayPlayer {
    /// Loads and merges one or more `.sdf` files into a continuous chronological replay timeline.
    /// Automatically sorts paths by part number / filename before merging.
    pub fn load_files<P: AsRef<Path>>(paths: &[P]) -> Result<Self, SdfError> {
        if paths.is_empty() {
            return Err(SdfError::InvalidFormat(
                "No .sdf files provided for replay".to_string(),
            ));
        }

        let mut sorted_paths: Vec<PathBuf> =
            paths.iter().map(|p| p.as_ref().to_path_buf()).collect();
        sorted_paths.sort();

        let mut readers = Vec::with_capacity(sorted_paths.len());
        for p in &sorted_paths {
            readers.push(SdfFileReader::open(p)?);
        }

        let first_meta = &readers[0].metadata;
        let mut events_map: BTreeMap<u16, A2lEvent> = BTreeMap::new();
        let mut meas_index_by_name: BTreeMap<String, usize> = BTreeMap::new();
        let mut measurements: Vec<Measurement> = Vec::new();

        // Collect all unique events and measurements across the loaded parts
        for reader in &readers {
            for ds in reader.metadata.datasets.values() {
                events_map
                    .entry(ds.event_channel_id)
                    .or_insert_with(|| A2lEvent {
                        channel_id: ds.event_channel_id,
                        name: ds.event_name.clone(),
                        short_name: ds.event_name.clone(),
                        cycle_time_us: (ds.refresh_rate_ms.max(0.1) * 1_000.0).round() as u64,
                    });

                if ds.path.starts_with("/Measurements/")
                    && !meas_index_by_name.contains_key(&ds.a2l_name)
                {
                    let idx = measurements.len();
                    let matrix_dim = if ds.shape.len() > 1 {
                        ds.shape[1..].to_vec()
                    } else {
                        Vec::new()
                    };
                    meas_index_by_name.insert(ds.a2l_name.clone(), idx);
                    measurements.push(Measurement {
                        name: ds.a2l_name.clone(),
                        description: format!("Replayed from {}", ds.path),
                        data_type: A2lDataType::Float64Ieee,
                        compu_method: "IDENTICAL".to_string(),
                        lower_limit: f64::MIN,
                        upper_limit: f64::MAX,
                        address: 0,
                        address_extension: 0,
                        matrix_dim,
                        unit: ds.unit.clone(),
                        event_id: Some(ds.event_channel_id),
                    });
                }
            }
        }

        let mut compu_methods = BTreeMap::new();
        compu_methods.insert(
            "IDENTICAL".to_string(),
            CompuMethod {
                name: "IDENTICAL".to_string(),
                description: "1:1 physical identity".to_string(),
                unit: String::new(),
                compu_type: CompuType::Identical,
            },
        );

        let hierarchy_root = build_measurement_hierarchy(&measurements);

        let a2l_model = A2lModel {
            project_name: first_meta.project_name.clone(),
            module_name: first_meta.module_name.clone(),
            events: events_map.clone(),
            compu_methods,
            compu_vtabs: BTreeMap::new(),
            record_layouts: BTreeMap::new(),
            typedef_measurements: BTreeMap::new(),
            typedef_characteristics: BTreeMap::new(),
            typedef_structures: BTreeMap::new(),
            instances: Vec::new(),
            measurements,
            characteristics: Vec::new(),
            hierarchy_root,
        };

        // Reconstruct DaqEventSamples for each reader and each event
        let mut all_samples: Vec<DaqEventSample> = Vec::new();
        for reader in &readers {
            let mut meas_by_event: BTreeMap<u16, Vec<(usize, usize, Vec<f64>)>> = BTreeMap::new();
            for ds in reader.metadata.datasets.values() {
                if !ds.path.starts_with("/Measurements/") {
                    continue;
                }
                let Some(&m_idx) = meas_index_by_name.get(&ds.a2l_name) else {
                    continue;
                };
                let Some(values_f64) = reader.read_f64_dataset(&ds.path) else {
                    continue;
                };
                let elem_count: usize = if ds.shape.len() > 1 {
                    ds.shape[1..].iter().product::<usize>().max(1)
                } else {
                    1
                };
                meas_by_event
                    .entry(ds.event_channel_id)
                    .or_default()
                    .push((m_idx, elem_count, values_f64));
            }

            for (&ev_id, ev_info) in &events_map {
                let time_s_path = format!("/Events/{}/time_utc_s", ev_info.name);
                let time_ns_path = format!("/Events/{}/time_utc_ns", ev_info.name);
                let Some(times_s) = reader.read_f64_dataset(&time_s_path) else {
                    continue;
                };
                let times_ns = reader.read_i64_dataset(&time_ns_path).unwrap_or_default();
                let ev_meas = meas_by_event.remove(&ev_id).unwrap_or_default();

                for (sample_i, &t_s) in times_s.iter().enumerate() {
                    let t_ns = times_ns
                        .get(sample_i)
                        .copied()
                        .unwrap_or((t_s * 1e9) as i64);
                    let mut values = BTreeMap::new();
                    for (m_idx, elem_count, flat_data) in &ev_meas {
                        let start = sample_i * (*elem_count);
                        let end = start + (*elem_count);
                        if end <= flat_data.len() {
                            if *elem_count == 1 {
                                values.insert(*m_idx, DecodedValue::Scalar(flat_data[start]));
                            } else {
                                values.insert(
                                    *m_idx,
                                    DecodedValue::Array(flat_data[start..end].to_vec()),
                                );
                            }
                        }
                    }
                    all_samples.push(DaqEventSample {
                        daq_list_num: ev_id,
                        event_channel_id: ev_id,
                        event_name: ev_info.name.clone(),
                        time_utc_ns: t_ns,
                        time_utc_s: t_s,
                        values,
                    });
                }
            }
        }

        all_samples.sort_by(|a, b| {
            a.time_utc_s
                .partial_cmp(&b.time_utc_s)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let start_utc_s = all_samples.first().map(|s| s.time_utc_s).unwrap_or(0.0);
        let end_utc_s = all_samples
            .last()
            .map(|s| s.time_utc_s)
            .unwrap_or(start_utc_s);
        let duration_s = (end_utc_s - start_utc_s).max(0.0);

        Ok(Self {
            loaded_files: sorted_paths,
            a2l_model,
            samples: all_samples,
            start_utc_s,
            end_utc_s,
            duration_s,
            position_s: 0.0,
            cursor_index: 0,
            playing: false,
            speed: 1.0,
        })
    }

    /// Returns the paths of the `.sdf` files loaded in this replay session.
    pub fn loaded_files(&self) -> &[PathBuf] {
        &self.loaded_files
    }

    /// Returns the reconstructed [`A2lModel`] for the loaded `.sdf` files.
    pub fn a2l_model(&self) -> &A2lModel {
        &self.a2l_model
    }

    /// Returns the total number of chronological event samples across all loaded parts.
    pub fn total_samples(&self) -> usize {
        self.samples.len()
    }

    /// Returns the total duration in seconds (`end_utc_s - start_utc_s`).
    pub fn duration_s(&self) -> f64 {
        self.duration_s
    }

    /// Returns the starting UTC timestamp in seconds of the loaded recording.
    pub fn start_utc_s(&self) -> f64 {
        self.start_utc_s
    }

    /// Returns the ending UTC timestamp in seconds of the loaded recording.
    pub fn end_utc_s(&self) -> f64 {
        self.end_utc_s
    }

    /// Returns the current playback offset in seconds (`0.0..=duration_s`).
    pub fn position_s(&self) -> f64 {
        self.position_s
    }

    /// Returns the current UTC timestamp in seconds (`start_utc_s + position_s`).
    pub fn current_utc_s(&self) -> f64 {
        self.start_utc_s + self.position_s
    }

    /// Returns `true` if replay is currently playing.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Starts or resumes playback. If already at the end of the timeline, restarts from `0.0`.
    pub fn play(&mut self) {
        if self.position_s >= self.duration_s && self.duration_s > 0.0 {
            self.position_s = 0.0;
            self.cursor_index = 0;
        }
        self.playing = true;
    }

    /// Pauses playback at the current position.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Toggles between playing and paused states.
    pub fn toggle_play_pause(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Stops playback and resets the cursor to `0.0 s`.
    pub fn stop(&mut self) {
        self.playing = false;
        self.position_s = 0.0;
        self.cursor_index = 0;
    }

    /// Returns the playback speed multiplier (`0.0` = `max`).
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Sets the playback speed multiplier (`0.25..=10.0`, or `0.0` for `max`).
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.max(0.0);
    }

    /// Seeks to `target_pos_s` (`0.0..=duration_s`) and returns all [`DaqEventSample`]s in the
    /// `[target_pos_s - SEEK_PREROLL_SECONDS, target_pos_s]` window so the UI state reflects
    /// the seeked time point immediately.
    pub fn seek_to_seconds(&mut self, target_pos_s: f64) -> Vec<DaqEventSample> {
        let clamped_pos = target_pos_s.clamp(0.0, self.duration_s);
        self.position_s = clamped_pos;
        let target_utc = self.start_utc_s + clamped_pos;
        let preroll_utc = self.start_utc_s + (clamped_pos - SEEK_PREROLL_SECONDS).max(0.0);

        let start_idx = self.samples.partition_point(|s| s.time_utc_s < preroll_utc);
        let end_idx = self.samples.partition_point(|s| s.time_utc_s <= target_utc);
        self.cursor_index = end_idx;
        self.samples[start_idx..end_idx].to_vec()
    }

    /// Advances playback by `wall_dt_s` real-world seconds (when `is_playing() == true`)
    /// and returns all [`DaqEventSample`]s crossed during the step.
    pub fn advance_by_wall_dt(&mut self, wall_dt_s: f64) -> Vec<DaqEventSample> {
        if !self.playing || self.samples.is_empty() {
            return Vec::new();
        }

        if self.speed == 0.0 {
            let end_idx = (self.cursor_index + MAX_SPEED_BATCH_SAMPLES).min(self.samples.len());
            let emitted = self.samples[self.cursor_index..end_idx].to_vec();
            self.cursor_index = end_idx;
            if let Some(last) = emitted.last() {
                self.position_s = (last.time_utc_s - self.start_utc_s).clamp(0.0, self.duration_s);
            }
            if self.cursor_index >= self.samples.len() {
                self.position_s = self.duration_s;
                self.playing = false;
            }
            return emitted;
        }

        let dt = wall_dt_s.clamp(0.0, 0.5) * self.speed;
        self.position_s = (self.position_s + dt).min(self.duration_s);
        let target_utc = self.start_utc_s + self.position_s;

        let end_idx = self.samples.partition_point(|s| s.time_utc_s <= target_utc);
        let emitted = if end_idx > self.cursor_index {
            self.samples[self.cursor_index..end_idx].to_vec()
        } else {
            Vec::new()
        };
        self.cursor_index = end_idx;

        if self.position_s >= self.duration_s && self.cursor_index >= self.samples.len() {
            self.playing = false;
        }
        emitted
    }
}
