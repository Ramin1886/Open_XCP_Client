// SPDX-License-Identifier: GPL-2.0-only
//! `a2l_inspect` and `a2l_find_signal`.

use super::{glob_match, load_a2l, A2lSource};
use crate::error::DevtoolResult;
use open_xcp_client::a2l::{A2lModel, StructTreeNode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Block keywords the library parser either consumes semantically or tolerates as structural
/// containers (PROJECT, MODULE, IF_DATA and the xcplite `DAQ_EVENT` / `TIMESTAMP_SUPPORTED`
/// sub-blocks). Anything else found after `/begin` is reported as `unrecognized_blocks` so agents
/// know what the parser silently skipped.
const KNOWN_BLOCKS: &[&str] = &[
    "PROJECT",
    "MODULE",
    "HEADER",
    "MOD_COMMON",
    "MOD_PAR",
    "IF_DATA",
    "XCP",
    "DAQ",
    "DAQ_EVENT",
    "TIMESTAMP_SUPPORTED",
    "EVENT",
    "MEASUREMENT",
    "CHARACTERISTIC",
    "COMPU_METHOD",
    "COMPU_VTAB",
    "RECORD_LAYOUT",
    "TYPEDEF_MEASUREMENT",
    "TYPEDEF_CHARACTERISTIC",
    "TYPEDEF_STRUCTURE",
    "STRUCTURE_COMPONENT",
    "INSTANCE",
    "GROUP",
    "FUNCTION",
    "ANNOTATION",
    "ANNOTATION_TEXT",
];

/// Input of `a2l_inspect`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct A2lInspectInput {
    /// A2L source: `{"path": "..."}` or `{"builtin": "adas_sample"}`.
    pub source: A2lSource,
    /// Maximum depth of the returned structure tree (default 3).
    #[serde(default = "default_tree_depth")]
    pub max_tree_depth: usize,
}

fn default_tree_depth() -> usize {
    3
}

/// One XCP event channel summary.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EventSummary {
    pub id: u16,
    pub name: String,
    pub refresh_rate_ms: f64,
    pub measurement_count: usize,
}

/// Counts of parsed A2L blocks.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BlockCounts {
    pub measurements: usize,
    pub characteristics: usize,
    pub compu_methods: usize,
    pub compu_vtabs: usize,
    pub record_layouts: usize,
    pub typedef_measurements: usize,
    pub typedef_characteristics: usize,
    pub typedef_structures: usize,
    pub instances: usize,
}

/// Structure tree node truncated to `max_tree_depth`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TreeNode {
    pub name: String,
    /// Number of leaf measurements under this node (counted to full depth).
    pub leaf_count: usize,
    /// Children, empty when the depth limit was reached or the node is a leaf.
    pub children: Vec<TreeNode>,
}

/// A `/begin` keyword the parser does not handle.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UnrecognizedBlock {
    pub keyword: String,
    pub count: usize,
}

/// Output of `a2l_inspect`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct A2lInspectOutput {
    pub ok: bool,
    pub source: String,
    pub project_name: String,
    pub module_name: String,
    pub events: Vec<EventSummary>,
    pub counts: BlockCounts,
    /// Sum of all measurement byte sizes, i.e. the DAQ payload per full cycle of every event.
    pub total_measurement_bytes: usize,
    pub struct_tree: Vec<TreeNode>,
    pub unrecognized_blocks: Vec<UnrecognizedBlock>,
    pub problems: Vec<String>,
}

fn leaf_count(node: &StructTreeNode) -> usize {
    if node.children.is_empty() {
        usize::from(node.measurement_index.is_some())
    } else {
        node.children.values().map(leaf_count).sum()
    }
}

fn to_tree(node: &StructTreeNode, depth_left: usize) -> TreeNode {
    TreeNode {
        name: node.name.clone(),
        leaf_count: leaf_count(node),
        children: if depth_left == 0 {
            Vec::new()
        } else {
            node.children
                .values()
                .map(|c| to_tree(c, depth_left - 1))
                .collect()
        },
    }
}

/// Strips `/* */` comments and returns the keywords following `/begin`.
fn begin_keywords(text: &str) -> BTreeMap<String, usize> {
    let mut cleaned = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        cleaned.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    cleaned.push_str(rest);
    let mut counts = BTreeMap::new();
    let mut tokens = cleaned.split_whitespace();
    while let Some(tok) = tokens.next() {
        if tok == "/begin" {
            if let Some(kw) = tokens.next() {
                *counts.entry(kw.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Runs `a2l_inspect`.
pub fn inspect(input: &A2lInspectInput) -> DevtoolResult<A2lInspectOutput> {
    let (text, model) = load_a2l(&input.source)?;
    let mut problems = Vec::new();

    let mut per_event: BTreeMap<u16, usize> = BTreeMap::new();
    for m in &model.measurements {
        if let Some(ev) = m.event_id {
            *per_event.entry(ev).or_insert(0) += 1;
        }
    }
    let unassigned = model
        .measurements
        .iter()
        .filter(|m| m.event_id.is_none())
        .count();
    if unassigned > 0 {
        problems.push(format!(
            "{unassigned} measurement(s) have no XCP event and will be planned on the first event"
        ));
    }
    let events = model
        .events
        .values()
        .map(|e| EventSummary {
            id: e.channel_id,
            name: e.name.clone(),
            refresh_rate_ms: e.refresh_rate_ms(),
            measurement_count: per_event.get(&e.channel_id).copied().unwrap_or(0),
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        problems.push("no XCP EVENT blocks found; DAQ planning is impossible".to_string());
    }

    let unrecognized_blocks = begin_keywords(&text)
        .into_iter()
        .filter(|(kw, _)| !KNOWN_BLOCKS.contains(&kw.as_str()))
        .map(|(keyword, count)| UnrecognizedBlock { keyword, count })
        .collect();

    Ok(A2lInspectOutput {
        ok: problems.is_empty(),
        source: input.source.describe(),
        project_name: model.project_name.clone(),
        module_name: model.module_name.clone(),
        events,
        counts: BlockCounts {
            measurements: model.measurements.len(),
            characteristics: model.characteristics.len(),
            compu_methods: model.compu_methods.len(),
            compu_vtabs: model.compu_vtabs.len(),
            record_layouts: model.record_layouts.len(),
            typedef_measurements: model.typedef_measurements.len(),
            typedef_characteristics: model.typedef_characteristics.len(),
            typedef_structures: model.typedef_structures.len(),
            instances: model.instances.len(),
        },
        total_measurement_bytes: model.measurements.iter().map(|m| m.total_byte_size()).sum(),
        struct_tree: model
            .hierarchy_root
            .children
            .values()
            .map(|c| to_tree(c, input.max_tree_depth.saturating_sub(1)))
            .collect(),
        unrecognized_blocks,
        problems,
    })
}

/// Which A2L object kinds `a2l_find_signal` searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Measurement,
    Characteristic,
    #[default]
    Any,
}

/// Input of `a2l_find_signal`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct A2lFindSignalInput {
    /// A2L source: `{"path": "..."}` or `{"builtin": "adas_sample"}`.
    pub source: A2lSource,
    /// Case-insensitive substring, or a glob when it contains `*`.
    pub query: String,
    /// Restrict to measurements, characteristics or both (default any).
    #[serde(default)]
    pub kind: SignalKind,
    /// Maximum number of matches returned (default 50).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// One match of `a2l_find_signal`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SignalMatch {
    pub name: String,
    pub kind: SignalKind,
    pub data_type: String,
    pub address: String,
    pub address_extension: u8,
    pub byte_size: usize,
    pub element_count: usize,
    pub dims: Vec<usize>,
    pub event_id: Option<u16>,
    pub event_name: Option<String>,
    pub unit: String,
    pub compu_method: String,
    pub lower_limit: f64,
    pub upper_limit: f64,
    pub description: String,
}

/// Output of `a2l_find_signal`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct A2lFindSignalOutput {
    pub ok: bool,
    pub query: String,
    pub matches: Vec<SignalMatch>,
    pub total_matches: usize,
    pub truncated: bool,
    pub problems: Vec<String>,
}

fn event_name(model: &A2lModel, id: Option<u16>) -> Option<String> {
    id.and_then(|i| model.events.get(&i))
        .map(|e| e.name.clone())
}

/// Runs `a2l_find_signal`.
pub fn find_signal(input: &A2lFindSignalInput) -> DevtoolResult<A2lFindSignalOutput> {
    let (_, model) = load_a2l(&input.source)?;
    let mut all = Vec::new();
    if matches!(input.kind, SignalKind::Measurement | SignalKind::Any) {
        for m in &model.measurements {
            if glob_match(&input.query, &m.name) {
                all.push(SignalMatch {
                    name: m.name.clone(),
                    kind: SignalKind::Measurement,
                    data_type: m.data_type.as_str().to_string(),
                    address: format!("0x{:08X}", m.address),
                    address_extension: m.address_extension,
                    byte_size: m.total_byte_size(),
                    element_count: m.element_count(),
                    dims: m.matrix_dim.clone(),
                    event_id: m.event_id,
                    event_name: event_name(&model, m.event_id),
                    unit: m.unit.clone(),
                    compu_method: m.compu_method.clone(),
                    lower_limit: m.lower_limit,
                    upper_limit: m.upper_limit,
                    description: m.description.clone(),
                });
            }
        }
    }
    if matches!(input.kind, SignalKind::Characteristic | SignalKind::Any) {
        for c in &model.characteristics {
            if glob_match(&input.query, &c.name) {
                all.push(SignalMatch {
                    name: c.name.clone(),
                    kind: SignalKind::Characteristic,
                    data_type: c.data_type.as_str().to_string(),
                    address: format!("0x{:08X}", c.address),
                    address_extension: c.address_extension,
                    byte_size: c.total_byte_size(),
                    element_count: c.element_count(),
                    dims: c.matrix_dim.clone(),
                    event_id: None,
                    event_name: None,
                    unit: c.unit.clone(),
                    compu_method: c.compu_method.clone(),
                    lower_limit: c.lower_limit,
                    upper_limit: c.upper_limit,
                    description: c.description.clone(),
                });
            }
        }
    }
    let total_matches = all.len();
    let truncated = total_matches > input.limit;
    all.truncate(input.limit);
    Ok(A2lFindSignalOutput {
        ok: true,
        query: input.query.clone(),
        matches: all,
        total_matches,
        truncated,
        problems: Vec::new(),
    })
}
