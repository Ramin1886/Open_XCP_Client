// SPDX-License-Identifier: GPL-2.0-only
//! Expands A2L v1.7 `INSTANCE` + `TYPEDEF_STRUCTURE` definitions into concrete
//! leaf measurements/characteristics and builds a unified hierarchical tree
//! preserving C/C++ complex data structures.

use crate::a2l::model::{A2lModel, Characteristic, Measurement, StructTreeNode};
use crate::a2l::parser::infer_datatype_from_layout_name;

/// Expands all `INSTANCE` declarations referencing `TYPEDEF_STRUCTURE`, `TYPEDEF_MEASUREMENT`,
/// or `TYPEDEF_CHARACTERISTIC` recursively into concrete [`Measurement`] and [`Characteristic`] items.
pub fn resolve_a2l_structures(model: &mut A2lModel) {
    let instances = model.instances.clone();
    for inst in &instances {
        let count = if inst.matrix_dim.is_empty() {
            1
        } else {
            inst.matrix_dim.iter().copied().product::<usize>().max(1)
        };
        let stride = model
            .typedef_structures
            .get(&inst.type_ref)
            .map(|s| s.byte_size)
            .unwrap_or(0);

        for elem_idx in 0..count {
            let prefix = if count > 1 {
                format!("{}[{}]", inst.name, elem_idx)
            } else {
                inst.name.clone()
            };
            let base_addr = inst
                .address
                .wrapping_add((elem_idx as u32).wrapping_mul(stride));
            expand_type_ref(
                model,
                &prefix,
                &inst.description,
                &inst.type_ref,
                base_addr,
                inst.address_extension,
                inst.event_id,
                &[],
                0,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_type_ref(
    model: &mut A2lModel,
    path_prefix: &str,
    description: &str,
    type_ref: &str,
    base_addr: u32,
    addr_ext: u8,
    event_id: Option<u16>,
    comp_matrix_dim: &[usize],
    depth: usize,
) {
    if depth > 16 {
        return;
    }

    if let Some(tm) = model.typedef_measurements.get(type_ref).cloned() {
        let dims = if !comp_matrix_dim.is_empty() {
            comp_matrix_dim.to_vec()
        } else {
            tm.matrix_dim.clone()
        };
        let unit = if tm.unit.is_empty() {
            model
                .compu_methods
                .get(&tm.compu_method)
                .map(|c| c.unit.clone())
                .unwrap_or_default()
        } else {
            tm.unit
        };
        model.measurements.push(Measurement {
            name: path_prefix.to_string(),
            description: if tm.description.is_empty() {
                description.to_string()
            } else {
                tm.description
            },
            data_type: tm.data_type,
            compu_method: tm.compu_method,
            lower_limit: tm.lower_limit,
            upper_limit: tm.upper_limit,
            address: base_addr,
            address_extension: addr_ext,
            matrix_dim: dims,
            unit,
            event_id,
        });
        return;
    }

    if let Some(tc) = model.typedef_characteristics.get(type_ref).cloned() {
        let dims = if !comp_matrix_dim.is_empty() {
            comp_matrix_dim.to_vec()
        } else {
            tc.matrix_dim.clone()
        };
        let dt = model
            .record_layouts
            .get(&tc.record_layout)
            .copied()
            .or_else(|| infer_datatype_from_layout_name(&tc.record_layout))
            .unwrap_or(tc.data_type);
        let unit = if tc.unit.is_empty() {
            model
                .compu_methods
                .get(&tc.compu_method)
                .map(|c| c.unit.clone())
                .unwrap_or_default()
        } else {
            tc.unit
        };
        model.characteristics.push(Characteristic {
            name: path_prefix.to_string(),
            description: if tc.description.is_empty() {
                description.to_string()
            } else {
                tc.description
            },
            char_type: tc.char_type,
            address: base_addr,
            address_extension: addr_ext,
            record_layout: tc.record_layout,
            data_type: dt,
            compu_method: tc.compu_method,
            lower_limit: tc.lower_limit,
            upper_limit: tc.upper_limit,
            matrix_dim: dims,
            unit,
        });
        return;
    }

    if let Some(ts) = model.typedef_structures.get(type_ref).cloned() {
        for comp in &ts.components {
            let comp_addr = base_addr.wrapping_add(comp.address_offset);
            // Check if component is an array of nested structures
            let is_sub_struct = model.typedef_structures.contains_key(&comp.type_ref);
            if is_sub_struct && !comp.matrix_dim.is_empty() {
                let sub_stride = model
                    .typedef_structures
                    .get(&comp.type_ref)
                    .map(|s| s.byte_size)
                    .unwrap_or(0);
                let count = comp.matrix_dim.iter().copied().product::<usize>().max(1);
                for idx in 0..count {
                    let sub_prefix = format!("{}.{}[{}]", path_prefix, comp.name, idx);
                    let elem_addr = comp_addr.wrapping_add((idx as u32).wrapping_mul(sub_stride));
                    expand_type_ref(
                        model,
                        &sub_prefix,
                        &ts.description,
                        &comp.type_ref,
                        elem_addr,
                        addr_ext,
                        event_id,
                        &[],
                        depth + 1,
                    );
                }
            } else {
                let sub_prefix = format!("{}.{}", path_prefix, comp.name);
                expand_type_ref(
                    model,
                    &sub_prefix,
                    &ts.description,
                    &comp.type_ref,
                    comp_addr,
                    addr_ext,
                    event_id,
                    &comp.matrix_dim,
                    depth + 1,
                );
            }
        }
    }
}

/// Normalizes a variable name with dots, slashes, or array brackets (e.g.
/// `Perception.RadarFront.Targets[0].Vel.x`) into clean HDF5/hierarchy path segments:
/// `["Perception", "RadarFront", "Targets_0", "Vel", "x"]`.
pub fn split_hierarchical_segments(variable_name: &str) -> Vec<String> {
    let mut segments = Vec::new();
    for raw_part in variable_name.split(['.', '/']) {
        let trimmed = raw_part.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Convert `Targets[0][1]` -> `Targets_0_1` so HDF5 group/dataset names are clean
        let normalized = trimmed
            .replace('[', "_")
            .replace(']', "")
            .trim_matches('_')
            .to_string();
        if !normalized.is_empty() {
            segments.push(normalized);
        }
    }
    if segments.is_empty() {
        segments.push(variable_name.to_string());
    }
    segments
}

/// Builds a hierarchical [`StructTreeNode`] from all resolved measurements.
pub fn build_measurement_hierarchy(measurements: &[Measurement]) -> StructTreeNode {
    let mut root = StructTreeNode {
        name: "Measurements".to_string(),
        full_path: "/Measurements".to_string(),
        ..Default::default()
    };

    for (idx, m) in measurements.iter().enumerate() {
        let segments = split_hierarchical_segments(&m.name);
        let mut current = &mut root;
        let mut path_acc = String::from("/Measurements");
        for (seg_i, seg) in segments.iter().enumerate() {
            path_acc = format!("{}/{}", path_acc, seg);
            let is_leaf = seg_i + 1 == segments.len();
            current = current
                .children
                .entry(seg.clone())
                .or_insert_with(|| StructTreeNode {
                    name: seg.clone(),
                    full_path: path_acc.clone(),
                    children: Default::default(),
                    measurement_index: None,
                });
            if is_leaf {
                current.measurement_index = Some(idx);
            }
        }
    }

    root
}
