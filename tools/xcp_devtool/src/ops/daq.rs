// SPDX-License-Identifier: GPL-2.0-only
//! `daq_plan_dryrun`: run the library's DAQ/ODT planner on an A2L and report limits.

use super::{glob_match, load_a2l, A2lSource};
use crate::error::DevtoolResult;
use open_xcp_client::xcp::daq_planner::{
    DaqPlanner, DTO_HEADER_RESERVE_BYTES, MAX_ODTS_PER_DAQ_LIST, MAX_ODT_ENTRIES,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input of `daq_plan_dryrun`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DaqPlanInput {
    /// A2L source: `{"path": "..."}` or `{"builtin": "adas_sample"}`.
    pub source: A2lSource,
    /// Target `MAX_DTO` in bytes (default 256). The planner budget is `max_dto - 6`, as in the app.
    #[serde(default = "default_max_dto")]
    pub max_dto: usize,
    /// Optional measurement name filters (substring or `*` glob). `null` plans every measurement.
    #[serde(default)]
    pub signals: Option<Vec<String>>,
}

fn default_max_dto() -> usize {
    256
}

/// Per-DAQ-list plan summary.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DaqListSummary {
    pub daq_list_num: u16,
    pub event_channel_id: u16,
    pub event_name: String,
    pub refresh_rate_ms: u64,
    pub odt_count: usize,
    pub entry_count: usize,
    pub payload_bytes: usize,
    pub measurements_planned: usize,
    pub dropped_measurements: Vec<String>,
}

/// Totals across all DAQ lists.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DaqTotals {
    pub daq_lists: usize,
    pub odts: usize,
    pub odt_entries: usize,
    pub payload_bytes: usize,
}

/// The library's hard limits.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DaqLimits {
    pub max_odt_entries: usize,
    pub max_odts_per_daq_list: usize,
}

/// Output of `daq_plan_dryrun`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DaqPlanOutput {
    pub ok: bool,
    pub max_dto: usize,
    pub max_odt_data_bytes: usize,
    pub measurements_considered: usize,
    pub lists: Vec<DaqListSummary>,
    pub totals: DaqTotals,
    pub limits: DaqLimits,
    /// Limit violations; must always be empty. A non-empty list means the planner invariant broke.
    pub violations: Vec<String>,
    pub dropped_total: usize,
    pub problems: Vec<String>,
}

/// Runs `daq_plan_dryrun`.
pub fn dryrun(input: &DaqPlanInput) -> DevtoolResult<DaqPlanOutput> {
    let (_, mut model) = load_a2l(&input.source)?;
    let mut problems = Vec::new();
    if let Some(filters) = &input.signals {
        model
            .measurements
            .retain(|m| filters.iter().any(|f| glob_match(f, &m.name)));
        if model.measurements.is_empty() {
            problems.push("no measurement matched the given signal filters".to_string());
        }
    }
    let max_odt_data_bytes = input.max_dto.saturating_sub(DTO_HEADER_RESERVE_BYTES);
    let planner = DaqPlanner::from_a2l(&model, max_odt_data_bytes);

    let mut violations = Vec::new();
    let mut lists = Vec::new();
    let mut totals = DaqTotals {
        daq_lists: planner.lists.len(),
        odts: 0,
        odt_entries: 0,
        payload_bytes: 0,
    };
    let mut dropped_total = 0;
    for list in &planner.lists {
        if list.odts.len() > MAX_ODTS_PER_DAQ_LIST {
            violations.push(format!(
                "DAQ list {} has {} ODTs (> {})",
                list.daq_list_num,
                list.odts.len(),
                MAX_ODTS_PER_DAQ_LIST
            ));
        }
        let mut entry_count = 0;
        let mut payload_bytes = 0;
        for odt in &list.odts {
            if odt.entries.len() > MAX_ODT_ENTRIES {
                violations.push(format!(
                    "DAQ list {} ODT {} has {} entries (> {})",
                    list.daq_list_num,
                    odt.odt_num,
                    odt.entries.len(),
                    MAX_ODT_ENTRIES
                ));
            }
            let odt_bytes: usize = odt.entries.iter().map(|e| e.byte_size as usize).sum();
            let budget = if odt.has_timestamp {
                max_odt_data_bytes.clamp(16, 1024).saturating_sub(4).max(1)
            } else {
                max_odt_data_bytes.clamp(16, 1024)
            };
            if odt_bytes > budget {
                violations.push(format!(
                    "DAQ list {} ODT {} carries {} bytes (> budget {})",
                    list.daq_list_num, odt.odt_num, odt_bytes, budget
                ));
            }
            entry_count += odt.entries.len();
            payload_bytes += odt_bytes;
        }
        let dropped: Vec<String> = list
            .dropped_measurement_indices
            .iter()
            .filter_map(|&i| model.measurements.get(i).map(|m| m.name.clone()))
            .collect();
        dropped_total += dropped.len();
        totals.odts += list.odts.len();
        totals.odt_entries += entry_count;
        totals.payload_bytes += payload_bytes;
        lists.push(DaqListSummary {
            daq_list_num: list.daq_list_num,
            event_channel_id: list.event_channel_id,
            event_name: list.event_name.clone(),
            refresh_rate_ms: list.refresh_rate_ms,
            odt_count: list.odts.len(),
            entry_count,
            payload_bytes,
            measurements_planned: list.measurement_indices.len(),
            dropped_measurements: dropped,
        });
    }
    if dropped_total > 0 {
        problems.push(format!(
            "{dropped_total} measurement(s) do not fit the per-list ODT limit and would not be acquired"
        ));
    }
    Ok(DaqPlanOutput {
        ok: violations.is_empty(),
        max_dto: input.max_dto,
        max_odt_data_bytes,
        measurements_considered: model.measurements.len(),
        lists,
        totals,
        limits: DaqLimits {
            max_odt_entries: MAX_ODT_ENTRIES,
            max_odts_per_daq_list: MAX_ODTS_PER_DAQ_LIST,
        },
        violations,
        dropped_total,
        problems,
    })
}
