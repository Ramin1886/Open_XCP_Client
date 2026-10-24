// SPDX-License-Identifier: GPL-2.0-only
//! Groups A2L measurements by XCP Event channel, plans DAQ lists and ODT entries,
//! and decodes incoming DTO packets into time-correlated measurement samples.

use crate::a2l::model::{A2lModel, Measurement};
use crate::xcp::clock_sync::TargetClockSync;
use std::collections::BTreeMap;

/// Maximum number of ODT entries in a single ODT.
///
/// `ALLOC_ODT_ENTRY` and `SET_DAQ_PTR` carry the ODT entry number in one byte, so an ODT can
/// hold at most 255 entries regardless of how many would still fit in `MAX_DTO`. Exceeding this
/// makes a slave reject the allocation with `ERR_MEMORY_OVERFLOW (0x22)`.
pub const MAX_ODT_ENTRIES: usize = 255;

/// Maximum number of ODTs in a single DAQ list.
///
/// `ALLOC_ODT` and `SET_DAQ_PTR` carry the ODT number in one byte.
pub const MAX_ODTS_PER_DAQ_LIST: usize = 255;

/// Decoded physical value of a measurement (either a scalar or a flattened N-D array).
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedValue {
    Scalar(f64),
    Array(Vec<f64>),
}

impl DecodedValue {
    /// Returns the first/scalar value for quick display in the UI.
    pub fn primary_scalar(&self) -> f64 {
        match self {
            Self::Scalar(v) => *v,
            Self::Array(arr) => arr.first().copied().unwrap_or(0.0),
        }
    }

    /// Formats the decoded value concisely for UI display.
    pub fn format_compact(&self) -> String {
        match self {
            Self::Scalar(v) => format!("{:.4}", v),
            Self::Array(arr) => {
                if arr.len() <= 4 {
                    let parts: Vec<String> = arr.iter().map(|x| format!("{:.3}", x)).collect();
                    format!("[{}]", parts.join(", "))
                } else {
                    format!("[{:.3}, {:.3}, ... ({} elems)]", arr[0], arr[1], arr.len())
                }
            }
        }
    }
}

/// A single ODT entry mapping a slice of target memory into an ODT packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaqOdtEntryPlan {
    pub measurement_index: usize,
    pub address: u32,
    pub address_extension: u8,
    pub byte_size: u8,
    /// Byte offset inside the measurement's raw representation (0 for scalar, >=0 for chunked arrays).
    pub element_byte_offset: usize,
}

/// Plan for a single ODT within a DAQ list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaqOdtPlan {
    pub odt_num: u8,
    pub has_timestamp: bool,
    pub entries: Vec<DaqOdtEntryPlan>,
}

/// Plan for a complete DAQ list bound to one XCP Event channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaqListPlan {
    pub daq_list_num: u16,
    pub event_channel_id: u16,
    pub event_name: String,
    pub refresh_rate_ms: u64,
    pub odts: Vec<DaqOdtPlan>,
    pub measurement_indices: Vec<usize>,
    /// Measurements that did not fit within [`MAX_ODTS_PER_DAQ_LIST`] and are therefore not
    /// acquired. Reported to the user rather than silently truncated into a malformed
    /// allocation.
    pub dropped_measurement_indices: Vec<usize>,
}

/// A complete decoded sample frame for one firing of an XCP Event channel.
#[derive(Debug, Clone, PartialEq)]
pub struct DaqEventSample {
    pub daq_list_num: u16,
    pub event_channel_id: u16,
    pub event_name: String,
    pub time_utc_s: f64,
    pub time_utc_ns: i64,
    /// Map of `measurement_index -> DecodedValue` for all measurements in this event.
    pub values: BTreeMap<usize, DecodedValue>,
}

/// Plans DAQ lists from an [`A2lModel`] and stateful-decodes incoming DTO packets.
#[derive(Debug, Clone)]
pub struct DaqPlanner {
    pub lists: Vec<DaqListPlan>,
    /// Per-DAQ-list staging buffer accumulating raw bytes for each measurement across ODTs.
    staging_bytes: Vec<BTreeMap<usize, Vec<u8>>>,
    /// Per-DAQ-list staging timestamp captured on `odt_num == 0`.
    staging_ts: Vec<Option<u32>>,
}

/// Returns the measurement-data budget of one ODT, reserving 4 bytes for the DAQ timestamp
/// that the slave prepends to the first ODT of every DAQ list.
fn odt_payload_budget(is_first_odt: bool, max_payload: usize) -> usize {
    if is_first_odt {
        max_payload.saturating_sub(4).max(1)
    } else {
        max_payload
    }
}

impl DaqPlanner {
    /// Builds a `DaqPlanner` grouping all measurements in `model` by their A2L `event_id`.
    /// `max_odt_data_bytes` specifies the maximum measurement payload per ODT (e.g., 248 for Ethernet).
    ///
    /// Each ODT is bounded both by that payload and by [`MAX_ODT_ENTRIES`], and each DAQ list by
    /// [`MAX_ODTS_PER_DAQ_LIST`], so the `u8` ODT and entry counters in `ALLOC_ODT`,
    /// `ALLOC_ODT_ENTRY` and `SET_DAQ_PTR` can never be truncated. Exactly one DAQ list is
    /// planned per XCP event channel; measurements that do not fit are reported through
    /// [`DaqPlanner::dropped_measurement_indices`] instead of being silently truncated.
    pub fn from_a2l(model: &A2lModel, max_odt_data_bytes: usize) -> Self {
        let max_payload = max_odt_data_bytes.clamp(16, 1024);
        let default_event_id = model.events.keys().copied().next().unwrap_or(0);

        let mut grouped: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (idx, m) in model.measurements.iter().enumerate() {
            let ev_id = m.event_id.unwrap_or(default_event_id);
            grouped.entry(ev_id).or_default().push(idx);
        }

        let mut lists = Vec::new();
        let mut staging_bytes = Vec::new();
        let mut staging_ts = Vec::new();

        for (daq_idx, (event_channel_id, meas_indices)) in grouped.into_iter().enumerate() {
            let daq_list_num = daq_idx as u16;
            let (event_name, refresh_rate_ms) = model
                .events
                .get(&event_channel_id)
                .map(|ev| (ev.name.clone(), (ev.cycle_time_us / 1_000).max(1)))
                .unwrap_or_else(|| (format!("Event_{}", event_channel_id), 10));

            // An ODT is closed when either the DTO payload is full or it already holds
            // MAX_ODT_ENTRIES entries. The first ODT of a list reserves 4 bytes for the
            // DAQ timestamp, so it carries correspondingly less measurement data.
            let mut odts: Vec<DaqOdtPlan> = Vec::new();
            let mut current_entries: Vec<DaqOdtEntryPlan> = Vec::new();
            let mut current_bytes = 0usize;
            let mut kept_indices: Vec<usize> = Vec::new();
            let mut dropped_indices: Vec<usize> = Vec::new();
            let mut list_full = false;

            for &m_idx in &meas_indices {
                if list_full {
                    dropped_indices.push(m_idx);
                    continue;
                }
                let m = &model.measurements[m_idx];
                let total_bytes = m.total_byte_size();
                let mut offset = 0usize;

                while offset < total_bytes {
                    let budget = odt_payload_budget(odts.is_empty(), max_payload);
                    let chunk = (total_bytes - offset).min(128).min(budget);
                    let full = !current_entries.is_empty()
                        && (current_bytes + chunk > budget
                            || current_entries.len() >= MAX_ODT_ENTRIES);
                    if full {
                        if odts.len() >= MAX_ODTS_PER_DAQ_LIST {
                            list_full = true;
                            break;
                        }
                        odts.push(DaqOdtPlan {
                            odt_num: odts.len() as u8,
                            has_timestamp: odts.is_empty(),
                            entries: std::mem::take(&mut current_entries),
                        });
                        current_bytes = 0;
                        continue;
                    }
                    current_entries.push(DaqOdtEntryPlan {
                        measurement_index: m_idx,
                        address: m.address.wrapping_add(offset as u32),
                        address_extension: m.address_extension,
                        byte_size: chunk as u8,
                        element_byte_offset: offset,
                    });
                    current_bytes += chunk;
                    offset += chunk;
                }

                if list_full {
                    // This measurement is only partially planned; drop it whole — including any
                    // entries already placed in earlier ODTs — so no measurement is ever
                    // half-acquired.
                    current_entries.retain(|e| e.measurement_index != m_idx);
                    for odt in &mut odts {
                        odt.entries.retain(|e| e.measurement_index != m_idx);
                    }
                    dropped_indices.push(m_idx);
                } else {
                    kept_indices.push(m_idx);
                }
            }

            if !current_entries.is_empty() && odts.len() < MAX_ODTS_PER_DAQ_LIST {
                odts.push(DaqOdtPlan {
                    odt_num: odts.len() as u8,
                    has_timestamp: odts.is_empty(),
                    entries: current_entries,
                });
            }

            let mut init_map = BTreeMap::new();
            for &m_idx in &kept_indices {
                init_map.insert(
                    m_idx,
                    vec![0u8; model.measurements[m_idx].total_byte_size()],
                );
            }

            lists.push(DaqListPlan {
                daq_list_num,
                event_channel_id,
                event_name,
                refresh_rate_ms,
                odts,
                measurement_indices: kept_indices,
                dropped_measurement_indices: dropped_indices,
            });
            staging_bytes.push(init_map);
            staging_ts.push(None);
        }

        Self {
            lists,
            staging_bytes,
            staging_ts,
        }
    }

    /// Returns every measurement index that did not fit into its DAQ list and is therefore
    /// not acquired.
    pub fn dropped_measurement_indices(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .lists
            .iter()
            .flat_map(|l| l.dropped_measurement_indices.iter().copied())
            .collect();
        out.sort_unstable();
        out
    }

    /// Returns the total number of ODT entries across all planned DAQ lists.
    pub fn total_odt_entries(&self) -> usize {
        self.lists
            .iter()
            .flat_map(|l| l.odts.iter())
            .map(|o| o.entries.len())
            .sum()
    }

    /// Returns the total measurement payload in bytes across all planned DAQ lists.
    pub fn total_daq_bytes(&self) -> usize {
        self.lists
            .iter()
            .flat_map(|l| l.odts.iter())
            .flat_map(|o| o.entries.iter())
            .map(|e| e.byte_size as usize)
            .sum()
    }

    /// Processes a single incoming DAQ DTO packet. When the final ODT of a DAQ list is received,
    /// returns the completed [`DaqEventSample`] with correlated UTC timestamps.
    pub fn process_dto_packet(
        &mut self,
        odt_num: u8,
        daq_list_num: u8,
        payload: &[u8],
        model: &A2lModel,
        clock_sync: &mut TargetClockSync,
    ) -> Option<DaqEventSample> {
        let list_idx = self
            .lists
            .iter()
            .position(|l| l.daq_list_num == daq_list_num as u16)?;
        let list = &self.lists[list_idx];
        let odt = list.odts.iter().find(|o| o.odt_num == odt_num)?;

        let mut cursor = 0usize;
        if odt.has_timestamp {
            if payload.len() < 4 {
                return None;
            }
            let ts = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            self.staging_ts[list_idx] = Some(ts);
            cursor += 4;
        }

        for entry in &odt.entries {
            let sz = entry.byte_size as usize;
            if cursor + sz > payload.len() {
                return None;
            }
            if let Some(buf) = self.staging_bytes[list_idx].get_mut(&entry.measurement_index) {
                let end = (entry.element_byte_offset + sz).min(buf.len());
                if entry.element_byte_offset < end {
                    let copy_len = end - entry.element_byte_offset;
                    buf[entry.element_byte_offset..end]
                        .copy_from_slice(&payload[cursor..cursor + copy_len]);
                }
            }
            cursor += sz;
        }

        let is_last_odt = list
            .odts
            .last()
            .map(|last| last.odt_num == odt_num)
            .unwrap_or(false);
        if !is_last_odt {
            return None;
        }

        let dto_ts = self.staging_ts[list_idx].take();
        let (time_utc_s, time_utc_ns) = clock_sync.resolve_sample_utc(dto_ts);

        let mut values = BTreeMap::new();
        for &m_idx in &list.measurement_indices {
            if let (Some(m), Some(raw_bytes)) = (
                model.measurements.get(m_idx),
                self.staging_bytes[list_idx].get(&m_idx),
            ) {
                let decoded = decode_measurement_bytes(m, raw_bytes, model);
                values.insert(m_idx, decoded);
            }
        }

        Some(DaqEventSample {
            daq_list_num: list.daq_list_num,
            event_channel_id: list.event_channel_id,
            event_name: list.event_name.clone(),
            time_utc_s,
            time_utc_ns,
            values,
        })
    }
}

/// Decodes raw target memory bytes into a physical [`DecodedValue`] using the measurement's
/// [`A2lDataType`] and [`CompuMethod`].
pub fn decode_measurement_bytes(
    m: &Measurement,
    raw_bytes: &[u8],
    model: &A2lModel,
) -> DecodedValue {
    let elem_size = m.data_type.byte_size();
    let elem_count = m.element_count();
    if elem_count <= 1 {
        let raw = m.data_type.decode_raw_le(raw_bytes).unwrap_or(0.0);
        let phys = model.raw_to_physical(&m.compu_method, raw);
        DecodedValue::Scalar(phys)
    } else {
        let mut out = Vec::with_capacity(elem_count);
        for i in 0..elem_count {
            let start = i * elem_size;
            let raw = if start + elem_size <= raw_bytes.len() {
                m.data_type
                    .decode_raw_le(&raw_bytes[start..start + elem_size])
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            out.push(model.raw_to_physical(&m.compu_method, raw));
        }
        DecodedValue::Array(out)
    }
}
