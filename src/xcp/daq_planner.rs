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

/// Why a partially received DAQ cycle was thrown away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDiscardReason {
    /// An ODT other than 0 arrived with no cycle open: this cycle's header (ODT 0, the carrier of
    /// the DAQ timestamp) was lost, so neither its data nor its timestamp can be trusted.
    MissingHeader,
    /// A new ODT 0 arrived while the previous cycle was still incomplete.
    Superseded,
    /// An ODT arrived out of order or twice within one cycle.
    SequenceError,
    /// The link lost, reordered or duplicated frames while this cycle was open.
    LinkDiscontinuity,
    /// The ODT payload was shorter than the plan requires.
    ShortPayload,
}

impl CycleDiscardReason {
    /// Short label for status messages and `.sdf` gap records.
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingHeader => "missing ODT 0 (no timestamp)",
            Self::Superseded => "incomplete when the next cycle began",
            Self::SequenceError => "ODT out of order or duplicated",
            Self::LinkDiscontinuity => "link gap while the cycle was open",
            Self::ShortPayload => "ODT payload shorter than planned",
        }
    }
}

/// A discarded DAQ cycle, reported so the loss is auditable instead of silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleDiscard {
    pub daq_list_num: u16,
    pub event_channel_id: u16,
    pub reason: CycleDiscardReason,
}

/// Per-DAQ-list integrity counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaqListLossStats {
    /// Cycles emitted complete, in order, with a target timestamp.
    pub cycles_emitted: u64,
    /// Cycles opened but discarded before completing.
    pub cycles_discarded: u64,
    /// ODTs discarded because no cycle was open (their ODT 0 was lost).
    pub cycles_lost_no_header: u64,
    /// Out-of-order or duplicate ODT deliveries.
    pub odt_sequence_errors: u64,
}

impl DaqListLossStats {
    /// Returns `true` when every cycle of this list completed cleanly.
    pub fn is_clean(&self) -> bool {
        self.cycles_discarded == 0
            && self.cycles_lost_no_header == 0
            && self.odt_sequence_errors == 0
    }
}

/// Reception state of the DAQ cycle currently being assembled for one DAQ list.
///
/// A "cycle" is one firing of the list's XCP event. ODT 0 carries the DAQ timestamp and is
/// therefore the only packet permitted to open a cycle — see [`DaqPlanner::process_dto_packet`].
#[derive(Debug, Clone)]
struct DaqCycleState {
    open: bool,
    seen: Vec<bool>,
    seen_count: usize,
    last_odt_num: u8,
    /// Link generation when this cycle's ODT 0 arrived. If the link loses frames before the cycle
    /// completes, the generation moves on and the cycle is abandoned.
    link_generation: u64,
}

impl DaqCycleState {
    fn new(odt_count: usize) -> Self {
        Self {
            open: false,
            seen: vec![false; odt_count],
            seen_count: 0,
            last_odt_num: 0,
            link_generation: 0,
        }
    }

    fn reset(&mut self) {
        self.open = false;
        self.seen.iter_mut().for_each(|b| *b = false);
        self.seen_count = 0;
        self.last_odt_num = 0;
    }
}

/// Plans DAQ lists from an [`A2lModel`] and stateful-decodes incoming DTO packets.
#[derive(Debug, Clone)]
pub struct DaqPlanner {
    pub lists: Vec<DaqListPlan>,
    /// Per-DAQ-list staging buffer accumulating raw bytes for each measurement across ODTs.
    staging_bytes: Vec<BTreeMap<usize, Vec<u8>>>,
    /// Per-DAQ-list staging timestamp captured on `odt_num == 0`.
    staging_ts: Vec<Option<u32>>,
    /// Per-DAQ-list cycle assembly state.
    cycle_state: Vec<DaqCycleState>,
    /// Per-DAQ-list integrity counters.
    loss_stats: Vec<DaqListLossStats>,
    /// Discarded cycles awaiting collection by the worker.
    pending_discards: Vec<CycleDiscard>,
    /// DTOs naming a DAQ list or ODT that was never allocated.
    unplanned_dto: u64,
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

        let cycle_state = lists
            .iter()
            .map(|l| DaqCycleState::new(l.odts.len()))
            .collect();
        let loss_stats = vec![DaqListLossStats::default(); lists.len()];

        Self {
            lists,
            staging_bytes,
            staging_ts,
            cycle_state,
            loss_stats,
            pending_discards: Vec::new(),
            unplanned_dto: 0,
        }
    }

    /// Returns how many DTOs named a DAQ list or ODT that was never allocated.
    ///
    /// A non-zero count means the slave is streaming something this client did not configure —
    /// typically a stale DAQ list left running from an earlier session.
    pub fn unplanned_dto_count(&self) -> u64 {
        self.unplanned_dto
    }

    /// Returns the per-DAQ-list integrity counters, indexed as [`DaqPlanner::lists`].
    pub fn loss_stats(&self) -> &[DaqListLossStats] {
        &self.loss_stats
    }

    /// Returns the integrity counters for the DAQ list serving `event_channel_id`.
    pub fn loss_stats_for_event(&self, event_channel_id: u16) -> Option<&DaqListLossStats> {
        let idx = self
            .lists
            .iter()
            .position(|l| l.event_channel_id == event_channel_id)?;
        self.loss_stats.get(idx)
    }

    /// Returns `true` when every DAQ list has assembled every cycle cleanly.
    pub fn is_clean(&self) -> bool {
        self.loss_stats.iter().all(DaqListLossStats::is_clean)
    }

    /// Takes the discarded-cycle reports accumulated since the last call.
    pub fn take_cycle_discards(&mut self) -> Vec<CycleDiscard> {
        std::mem::take(&mut self.pending_discards)
    }

    /// Abandons every partially received cycle, for example after the link lost frames.
    ///
    /// The 16-bit `CTR` is shared by every frame on the connection, so a discontinuity proves loss
    /// happened without revealing which DAQ list it hit. Every open cycle is therefore suspect and
    /// is thrown away rather than completed from a mixture of firings.
    pub fn invalidate_open_cycles(&mut self, reason: CycleDiscardReason) -> usize {
        let mut abandoned = 0;
        for idx in 0..self.cycle_state.len() {
            if self.cycle_state[idx].open {
                self.discard_cycle(idx, reason);
                abandoned += 1;
            }
        }
        abandoned
    }

    /// Resets a list's cycle state and records why its in-flight cycle was thrown away.
    fn discard_cycle(&mut self, list_idx: usize, reason: CycleDiscardReason) {
        self.cycle_state[list_idx].reset();
        self.staging_ts[list_idx] = None;
        let stats = &mut self.loss_stats[list_idx];
        match reason {
            CycleDiscardReason::MissingHeader => {
                stats.cycles_lost_no_header = stats.cycles_lost_no_header.saturating_add(1);
            }
            CycleDiscardReason::SequenceError => {
                stats.odt_sequence_errors = stats.odt_sequence_errors.saturating_add(1);
                stats.cycles_discarded = stats.cycles_discarded.saturating_add(1);
            }
            _ => {
                stats.cycles_discarded = stats.cycles_discarded.saturating_add(1);
            }
        }
        let list = &self.lists[list_idx];
        self.pending_discards.push(CycleDiscard {
            daq_list_num: list.daq_list_num,
            event_channel_id: list.event_channel_id,
            reason,
        });
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

    /// Processes a single incoming DAQ DTO packet, returning a [`DaqEventSample`] only when a
    /// complete, in-order cycle carrying its own target timestamp has been assembled.
    ///
    /// # Cycle framing
    ///
    /// ODT 0 carries the DAQ timestamp, which makes it the cycle delimiter: **only ODT 0 may open a
    /// cycle**, and a sample is emitted only when every planned ODT of that cycle has arrived
    /// exactly once, in increasing order, since that ODT 0.
    ///
    /// Tracking "all ODTs seen" with a mask reset on ODT 0 is *not* enough, and neither is adding
    /// strictly increasing ODT numbers. On a two-ODT list, losing ODT 1 of cycle `N` and then ODT 0
    /// of cycle `N+1` leaves ODT 1 of `N+1` arriving against a mask that still holds ODT 0 from
    /// cycle `N`; the ODT number has increased, the mask completes, and a sample stitched from two
    /// different event firings would be emitted as valid. Requiring ODT 0 to have opened the cycle
    /// rejects that case whatever the list length, and makes it impossible for the staging buffer
    /// to leak bytes across firings or for the host-clock timestamp fallback to be reached.
    ///
    /// `link_generation` is [`crate::xcp::link_stats::LinkStats::generation`] as observed for this
    /// DTO's frame. Non-header ODTs carry no cycle identity, so the two signals are needed
    /// together: ODT 0 anchors the cycle, and a change of generation proves that frames went
    /// missing in between. Any datagram the network drops has already consumed a `CTR` value, so
    /// network loss always moves the generation — the case where only the slave omits a packet
    /// would be a slave defect, not a transport loss.
    ///
    /// Anything incomplete is discarded and counted — see [`DaqPlanner::loss_stats`] and
    /// [`DaqPlanner::take_cycle_discards`] — never emitted with a flag, because a torn sample has
    /// no valid physical interpretation and a flag only protects consumers that check it.
    pub fn process_dto_packet(
        &mut self,
        odt_num: u8,
        daq_list_num: u8,
        payload: &[u8],
        link_generation: u64,
        model: &A2lModel,
        clock_sync: &mut TargetClockSync,
    ) -> Option<DaqEventSample> {
        let Some(list_idx) = self
            .lists
            .iter()
            .position(|l| l.daq_list_num == daq_list_num as u16)
        else {
            self.unplanned_dto = self.unplanned_dto.saturating_add(1);
            return None;
        };
        let Some(odt_idx) = self.lists[list_idx]
            .odts
            .iter()
            .position(|o| o.odt_num == odt_num)
        else {
            self.unplanned_dto = self.unplanned_dto.saturating_add(1);
            return None;
        };

        if odt_num == 0 {
            // A fresh header supersedes whatever was still being assembled.
            if self.cycle_state[list_idx].open {
                self.discard_cycle(list_idx, CycleDiscardReason::Superseded);
            }
            self.cycle_state[list_idx].reset();
            self.cycle_state[list_idx].open = true;
            self.cycle_state[list_idx].link_generation = link_generation;
        } else if self.cycle_state[list_idx].open
            && self.cycle_state[list_idx].link_generation != link_generation
        {
            // Frames went missing between this cycle's header and now. Non-header ODTs carry no
            // cycle identity of their own — only ODT 0 carries the DAQ timestamp — so an ODT that
            // arrives after a link discontinuity cannot be proven to belong to the open cycle.
            // This is what rejects the burst where the last ODT of one cycle and the header of the
            // next are both lost: the open cycle would otherwise be completed by the next firing's
            // ODTs, since their ODT numbers follow on correctly.
            self.discard_cycle(list_idx, CycleDiscardReason::LinkDiscontinuity);
            return None;
        } else if !self.cycle_state[list_idx].open {
            // This cycle's ODT 0 never arrived: without it there is no trustworthy timestamp and
            // the earlier ODTs are missing, so the payload is dropped before it can contaminate
            // the staging buffer.
            self.discard_cycle(list_idx, CycleDiscardReason::MissingHeader);
            return None;
        } else if odt_num <= self.cycle_state[list_idx].last_odt_num
            || self.cycle_state[list_idx].seen[odt_idx]
        {
            // Out-of-order or duplicate delivery within one cycle.
            self.discard_cycle(list_idx, CycleDiscardReason::SequenceError);
            return None;
        }

        let odt = &self.lists[list_idx].odts[odt_idx];
        let mut cursor = 0usize;
        if odt.has_timestamp {
            if payload.len() < 4 {
                self.discard_cycle(list_idx, CycleDiscardReason::ShortPayload);
                return None;
            }
            let ts = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            self.staging_ts[list_idx] = Some(ts);
            cursor += 4;
        }

        // Reject a short payload before writing any of it, so a truncated ODT cannot leave the
        // staging buffer half-updated.
        let required: usize = odt.entries.iter().map(|e| e.byte_size as usize).sum();
        if cursor + required > payload.len() {
            self.discard_cycle(list_idx, CycleDiscardReason::ShortPayload);
            return None;
        }

        for entry in &self.lists[list_idx].odts[odt_idx].entries {
            let sz = entry.byte_size as usize;
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

        let state = &mut self.cycle_state[list_idx];
        state.seen[odt_idx] = true;
        state.seen_count += 1;
        state.last_odt_num = odt_num;
        if state.seen_count < state.seen.len() {
            return None;
        }

        // Complete: every planned ODT of this cycle arrived once, in order, after its ODT 0.
        self.cycle_state[list_idx].reset();
        let dto_ts = self.staging_ts[list_idx].take();
        let (time_utc_s, time_utc_ns) = clock_sync.resolve_sample_utc(dto_ts);
        self.loss_stats[list_idx].cycles_emitted =
            self.loss_stats[list_idx].cycles_emitted.saturating_add(1);

        let list = &self.lists[list_idx];
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
