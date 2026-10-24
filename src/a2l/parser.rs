// SPDX-License-Identifier: GPL-2.0-only
//! ASAM MCD-2 MC (A2L) parser tailored for Vector `xcplite` 2.2.2 generated A2L files.

use crate::a2l::lexer::{parse_i64_literal, parse_u64_literal, tokenize_a2l, A2lToken};
use crate::a2l::model::{
    A2lDataType, A2lEvent, A2lModel, Characteristic, CharacteristicType, CompuMethod, CompuType,
    Instance, Measurement, StructureComponent, TypedefCharacteristic, TypedefMeasurement,
    TypedefStructure, MAX_ELEMENTS_PER_SIGNAL,
};
use crate::a2l::struct_resolver::{build_measurement_hierarchy, resolve_a2l_structures};
use std::collections::BTreeMap;
use thiserror::Error;

/// Consumes the numeric literals that follow a `MATRIX_DIM`-style keyword, starting at `start`.
/// Every positive dimension is pushed to `out`; the index of the first non-numeric token is returned.
fn consume_matrix_dims(body: &[A2lToken<'_>], start: usize, out: &mut Vec<usize>) -> usize {
    let mut idx = start;
    while idx < body.len() {
        match parse_u64_literal(body[idx].as_str()) {
            Some(dim) => {
                if dim > 0 {
                    // Clamp each dimension so a hostile or broken A2L cannot request absurd
                    // element counts; the product is bounded again in `element_count()`.
                    out.push(
                        usize::try_from(dim)
                            .unwrap_or(usize::MAX)
                            .min(MAX_ELEMENTS_PER_SIGNAL),
                    );
                }
                idx += 1;
            }
            None => break,
        }
    }
    idx
}

/// Errors produced when parsing an A2L file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum A2lParseError {
    #[error("A2L input is empty or contains no valid MODULE/PROJECT blocks")]
    EmptyInput,
    #[error("Unexpected end of A2L block while parsing '{0}'")]
    UnexpectedEof(String),
}

/// Parses an A2L string into a resolved [`A2lModel`].
pub fn parse_a2l(input: &str) -> Result<A2lModel, A2lParseError> {
    let tokens = tokenize_a2l(input);
    if tokens.is_empty() {
        return Err(A2lParseError::EmptyInput);
    }

    let mut model = A2lModel {
        project_name: "DefaultProject".to_string(),
        module_name: "DefaultModule".to_string(),
        ..Default::default()
    };

    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == A2lToken::Begin {
            if i + 1 >= tokens.len() {
                return Err(A2lParseError::UnexpectedEof("root /begin".to_string()));
            }
            let block_type = tokens[i + 1].as_str().to_uppercase();
            match block_type.as_str() {
                "PROJECT" => {
                    if i + 2 < tokens.len() {
                        model.project_name = tokens[i + 2].as_str().to_string();
                    }
                    i += 2;
                }
                "MODULE" => {
                    if i + 2 < tokens.len() {
                        model.module_name = tokens[i + 2].as_str().to_string();
                    }
                    i += 2;
                }
                "EVENT" => {
                    let end = find_matching_end(&tokens, i, "EVENT")?;
                    if let Some(ev) = parse_event_block(&tokens[i + 2..end]) {
                        model.events.insert(ev.channel_id, ev);
                    }
                    i = end + 2;
                }
                "COMPU_METHOD" => {
                    let end = find_matching_end(&tokens, i, "COMPU_METHOD")?;
                    if let Some(cm) = parse_compu_method_block(&tokens[i + 2..end]) {
                        model.compu_methods.insert(cm.name.clone(), cm);
                    }
                    i = end + 2;
                }
                "COMPU_VTAB" => {
                    let end = find_matching_end(&tokens, i, "COMPU_VTAB")?;
                    if let Some((name, map)) = parse_compu_vtab_block(&tokens[i + 2..end]) {
                        model.compu_vtabs.insert(name, map);
                    }
                    i = end + 2;
                }
                "RECORD_LAYOUT" => {
                    let end = find_matching_end(&tokens, i, "RECORD_LAYOUT")?;
                    if let Some((name, dt)) = parse_record_layout_block(&tokens[i + 2..end]) {
                        model.record_layouts.insert(name, dt);
                    }
                    i = end + 2;
                }
                "MEASUREMENT" => {
                    let end = find_matching_end(&tokens, i, "MEASUREMENT")?;
                    if let Some(m) = parse_measurement_block(&tokens[i + 2..end], &model) {
                        model.measurements.push(m);
                    }
                    i = end + 2;
                }
                "CHARACTERISTIC" => {
                    let end = find_matching_end(&tokens, i, "CHARACTERISTIC")?;
                    if let Some(c) = parse_characteristic_block(&tokens[i + 2..end], &model) {
                        model.characteristics.push(c);
                    }
                    i = end + 2;
                }
                "TYPEDEF_MEASUREMENT" => {
                    let end = find_matching_end(&tokens, i, "TYPEDEF_MEASUREMENT")?;
                    if let Some(tm) = parse_typedef_measurement_block(&tokens[i + 2..end], &model) {
                        model.typedef_measurements.insert(tm.name.clone(), tm);
                    }
                    i = end + 2;
                }
                "TYPEDEF_CHARACTERISTIC" => {
                    let end = find_matching_end(&tokens, i, "TYPEDEF_CHARACTERISTIC")?;
                    if let Some(tc) =
                        parse_typedef_characteristic_block(&tokens[i + 2..end], &model)
                    {
                        model.typedef_characteristics.insert(tc.name.clone(), tc);
                    }
                    i = end + 2;
                }
                "TYPEDEF_STRUCTURE" => {
                    let end = find_matching_end(&tokens, i, "TYPEDEF_STRUCTURE")?;
                    if let Some(ts) = parse_typedef_structure_block(&tokens[i + 2..end]) {
                        model.typedef_structures.insert(ts.name.clone(), ts);
                    }
                    i = end + 2;
                }
                "INSTANCE" => {
                    let end = find_matching_end(&tokens, i, "INSTANCE")?;
                    if let Some(inst) = parse_instance_block(&tokens[i + 2..end]) {
                        model.instances.push(inst);
                    }
                    i = end + 2;
                }
                _ => {
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }

    // Backfill characteristic data_types and units from RECORD_LAYOUT and COMPU_METHOD if parsed later
    for ch in &mut model.characteristics {
        if let Some(&dt) = model.record_layouts.get(&ch.record_layout) {
            ch.data_type = dt;
        } else if let Some(dt) = infer_datatype_from_layout_name(&ch.record_layout) {
            ch.data_type = dt;
        }
        if ch.unit.is_empty() {
            if let Some(cm) = model.compu_methods.get(&ch.compu_method) {
                ch.unit = cm.unit.clone();
            }
        }
    }
    for m in &mut model.measurements {
        if m.unit.is_empty() {
            if let Some(cm) = model.compu_methods.get(&m.compu_method) {
                m.unit = cm.unit.clone();
            }
        }
    }

    resolve_a2l_structures(&mut model);
    model.hierarchy_root = build_measurement_hierarchy(&model.measurements);

    Ok(model)
}

fn find_matching_end(
    tokens: &[A2lToken<'_>],
    start_begin_idx: usize,
    block_name: &str,
) -> Result<usize, A2lParseError> {
    let mut depth = 0usize;
    let mut j = start_begin_idx;
    while j < tokens.len() {
        if tokens[j] == A2lToken::Begin {
            if j + 1 < tokens.len() && tokens[j + 1].as_str().eq_ignore_ascii_case(block_name) {
                depth += 1;
            }
        } else if tokens[j] == A2lToken::End
            && j + 1 < tokens.len()
            && tokens[j + 1].as_str().eq_ignore_ascii_case(block_name)
        {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Ok(j);
            }
        }
        j += 1;
    }
    Err(A2lParseError::UnexpectedEof(block_name.to_string()))
}

fn parse_event_block(body: &[A2lToken<'_>]) -> Option<A2lEvent> {
    // Format inside IF_DATA XCP /begin EVENT:
    // "event_name" "short_name" channel_id DAQ max_daq time_cycle time_unit priority
    if body.len() < 3 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let short_name = body[1].as_str().to_string();
    let channel_id = parse_u64_literal(body[2].as_str())? as u16;
    // Standard XCP EVENT: body[3]=DAQ/STIM, body[4]=max_daq, body[5]=time_cycle, body[6]=time_unit
    let mut cycle_time_us = 0u64;
    if body.len() >= 7 {
        let time_cycle = parse_u64_literal(body[5].as_str()).unwrap_or(0);
        let time_unit = parse_u64_literal(body[6].as_str()).unwrap_or(6);
        // ASAM time_unit: 3=1us, 4=10us, 5=100us, 6=1ms, 7=10ms, 8=100ms, 9=1s
        let unit_us = match time_unit {
            0..=3 => 1,
            4 => 10,
            5 => 100,
            6 => 1_000,
            7 => 10_000,
            8 => 100_000,
            9 => 1_000_000,
            _ => 1_000,
        };
        cycle_time_us = time_cycle.saturating_mul(unit_us);
    }
    Some(A2lEvent {
        channel_id,
        name,
        short_name,
        cycle_time_us,
    })
}

fn parse_compu_method_block(body: &[A2lToken<'_>]) -> Option<CompuMethod> {
    // Name "Description" ConversionType "Format" "Unit" ...
    if body.len() < 5 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let conv_type_str = body[2].as_str().to_uppercase();
    let unit = body[4].as_str().to_string();

    let mut compu_type = CompuType::Identical;
    match conv_type_str.as_str() {
        "LINEAR" => {
            for idx in 5..body.len() {
                if body[idx].as_str().eq_ignore_ascii_case("COEFFS_LINEAR") && idx + 2 < body.len()
                {
                    let a = body[idx + 1].as_str().parse::<f64>().unwrap_or(1.0);
                    let b = body[idx + 2].as_str().parse::<f64>().unwrap_or(0.0);
                    compu_type = CompuType::Linear { a, b };
                    break;
                }
            }
        }
        "RAT_FUNC" => {
            for idx in 5..body.len() {
                if body[idx].as_str().eq_ignore_ascii_case("COEFFS") && idx + 6 < body.len() {
                    let a = body[idx + 1].as_str().parse::<f64>().unwrap_or(0.0);
                    let b = body[idx + 2].as_str().parse::<f64>().unwrap_or(1.0);
                    let c = body[idx + 3].as_str().parse::<f64>().unwrap_or(0.0);
                    let d = body[idx + 4].as_str().parse::<f64>().unwrap_or(0.0);
                    let e = body[idx + 5].as_str().parse::<f64>().unwrap_or(0.0);
                    let f = body[idx + 6].as_str().parse::<f64>().unwrap_or(1.0);
                    compu_type = CompuType::RatFunc { a, b, c, d, e, f };
                    break;
                }
            }
        }
        "TAB_VERB" => {
            for idx in 5..body.len() {
                if body[idx].as_str().eq_ignore_ascii_case("COMPU_TAB_REF") && idx + 1 < body.len()
                {
                    compu_type = CompuType::TabVerb {
                        table_name: body[idx + 1].as_str().to_string(),
                    };
                    break;
                }
            }
        }
        _ => {}
    }

    Some(CompuMethod {
        name,
        description,
        unit,
        compu_type,
    })
}

fn parse_compu_vtab_block(body: &[A2lToken<'_>]) -> Option<(String, BTreeMap<i64, String>)> {
    // Name "Desc" TAB_VERB count (key "label")*
    if body.len() < 4 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let count = parse_u64_literal(body[3].as_str()).unwrap_or(0) as usize;
    let mut map = BTreeMap::new();
    let mut idx = 4;
    for _ in 0..count {
        if idx + 1 >= body.len() {
            break;
        }
        if let Some(k) = parse_i64_literal(body[idx].as_str()) {
            let v = body[idx + 1].as_str().to_string();
            map.insert(k, v);
        }
        idx += 2;
    }
    Some((name, map))
}

fn parse_record_layout_block(body: &[A2lToken<'_>]) -> Option<(String, A2lDataType)> {
    if body.is_empty() {
        return None;
    }
    let name = body[0].as_str().to_string();
    for idx in 1..body.len() {
        if body[idx].as_str().eq_ignore_ascii_case("FNC_VALUES") && idx + 2 < body.len() {
            if let Some(dt) = A2lDataType::from_keyword(body[idx + 2].as_str()) {
                return Some((name, dt));
            }
        }
    }
    infer_datatype_from_layout_name(&name).map(|dt| (name, dt))
}

pub(crate) fn infer_datatype_from_layout_name(layout: &str) -> Option<A2lDataType> {
    let up = layout.to_uppercase();
    if up.contains("UBYTE") || up.contains("U8") || up.contains("UINT8") || up.contains("BOOL") {
        Some(A2lDataType::UByte)
    } else if up.contains("SBYTE") || up.contains("I8") || up.contains("INT8") {
        Some(A2lDataType::SByte)
    } else if up.contains("UWORD") || up.contains("U16") || up.contains("UINT16") {
        Some(A2lDataType::UWord)
    } else if up.contains("SWORD") || up.contains("I16") || up.contains("INT16") {
        Some(A2lDataType::SWord)
    } else if up.contains("A_UINT64") || up.contains("U64") || up.contains("UINT64") {
        Some(A2lDataType::AUint64)
    } else if up.contains("A_INT64") || up.contains("I64") || up.contains("INT64") {
        Some(A2lDataType::AInt64)
    } else if up.contains("ULONG") || up.contains("U32") || up.contains("UINT32") {
        Some(A2lDataType::ULong)
    } else if up.contains("SLONG") || up.contains("I32") || up.contains("INT32") {
        Some(A2lDataType::SLong)
    } else if up.contains("FLOAT64") || up.contains("F64") || up.contains("DOUBLE") {
        Some(A2lDataType::Float64Ieee)
    } else if up.contains("FLOAT32") || up.contains("F32") || up.contains("FLOAT") {
        Some(A2lDataType::Float32Ieee)
    } else {
        None
    }
}

fn parse_measurement_block(body: &[A2lToken<'_>], model: &A2lModel) -> Option<Measurement> {
    // Name "Description" DataType CompuMethod Resolution Accuracy LowerLimit UpperLimit ...
    if body.len() < 8 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let data_type = A2lDataType::from_keyword(body[2].as_str())?;
    let compu_method = body[3].as_str().to_string();
    let lower_limit = body[6].as_str().parse::<f64>().unwrap_or(f64::MIN);
    let upper_limit = body[7].as_str().parse::<f64>().unwrap_or(f64::MAX);

    let mut address = 0u32;
    let mut address_extension = 0u8;
    let mut matrix_dim = Vec::new();
    let mut unit = model
        .compu_methods
        .get(&compu_method)
        .map(|cm| cm.unit.clone())
        .unwrap_or_default();
    let mut event_id = None;

    let mut idx = 8;
    while idx < body.len() {
        let kw = body[idx].as_str();
        if kw.eq_ignore_ascii_case("ECU_ADDRESS") && idx + 1 < body.len() {
            address = parse_u64_literal(body[idx + 1].as_str()).unwrap_or(0) as u32;
            idx += 2;
        } else if kw.eq_ignore_ascii_case("ECU_ADDRESS_EXTENSION") && idx + 1 < body.len() {
            address_extension = parse_u64_literal(body[idx + 1].as_str()).unwrap_or(0) as u8;
            idx += 2;
        } else if kw.eq_ignore_ascii_case("PHYS_UNIT") && idx + 1 < body.len() {
            unit = body[idx + 1].as_str().to_string();
            idx += 2;
        } else if (kw.eq_ignore_ascii_case("MATRIX_DIM") || kw.eq_ignore_ascii_case("ARRAY_SIZE"))
            && idx + 1 < body.len()
        {
            idx = consume_matrix_dims(body, idx + 1, &mut matrix_dim);
        } else if (kw.eq_ignore_ascii_case("EVENT")
            || kw.eq_ignore_ascii_case("DEFAULT_EVENT_LIST")
            || kw.eq_ignore_ascii_case("FIXED_EVENT_LIST"))
            && idx + 1 < body.len()
        {
            if let Some(ev) = parse_u64_literal(body[idx + 1].as_str()) {
                event_id = Some(ev as u16);
                idx += 2;
            } else {
                idx += 1;
            }
        } else {
            idx += 1;
        }
    }

    Some(Measurement {
        name,
        description,
        data_type,
        compu_method,
        lower_limit,
        upper_limit,
        address,
        address_extension,
        matrix_dim,
        unit,
        event_id,
    })
}

fn parse_characteristic_block(body: &[A2lToken<'_>], model: &A2lModel) -> Option<Characteristic> {
    // Name "Description" Type Address RecordLayout MaxDiff CompuMethod LowerLimit UpperLimit ...
    if body.len() < 9 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let char_type = CharacteristicType::from_keyword(body[2].as_str())?;
    let address = parse_u64_literal(body[3].as_str()).unwrap_or(0) as u32;
    let record_layout = body[4].as_str().to_string();
    let compu_method = body[6].as_str().to_string();
    let lower_limit = body[7].as_str().parse::<f64>().unwrap_or(f64::MIN);
    let upper_limit = body[8].as_str().parse::<f64>().unwrap_or(f64::MAX);

    let data_type = model
        .record_layouts
        .get(&record_layout)
        .copied()
        .or_else(|| infer_datatype_from_layout_name(&record_layout))
        .unwrap_or(A2lDataType::Float32Ieee);

    let mut address_extension = 0u8;
    let mut matrix_dim = Vec::new();
    let mut unit = model
        .compu_methods
        .get(&compu_method)
        .map(|cm| cm.unit.clone())
        .unwrap_or_default();

    let mut idx = 9;
    while idx < body.len() {
        let kw = body[idx].as_str();
        if kw.eq_ignore_ascii_case("ECU_ADDRESS_EXTENSION") && idx + 1 < body.len() {
            address_extension = parse_u64_literal(body[idx + 1].as_str()).unwrap_or(0) as u8;
            idx += 2;
        } else if kw.eq_ignore_ascii_case("PHYS_UNIT") && idx + 1 < body.len() {
            unit = body[idx + 1].as_str().to_string();
            idx += 2;
        } else if (kw.eq_ignore_ascii_case("MATRIX_DIM") || kw.eq_ignore_ascii_case("NUMBER"))
            && idx + 1 < body.len()
        {
            idx = consume_matrix_dims(body, idx + 1, &mut matrix_dim);
        } else {
            idx += 1;
        }
    }

    Some(Characteristic {
        name,
        description,
        char_type,
        address,
        address_extension,
        record_layout,
        data_type,
        compu_method,
        lower_limit,
        upper_limit,
        matrix_dim,
        unit,
    })
}

fn parse_typedef_measurement_block(
    body: &[A2lToken<'_>],
    model: &A2lModel,
) -> Option<TypedefMeasurement> {
    if body.len() < 8 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let data_type = A2lDataType::from_keyword(body[2].as_str())?;
    let compu_method = body[3].as_str().to_string();
    let lower_limit = body[6].as_str().parse::<f64>().unwrap_or(f64::MIN);
    let upper_limit = body[7].as_str().parse::<f64>().unwrap_or(f64::MAX);
    let mut matrix_dim = Vec::new();
    let mut unit = model
        .compu_methods
        .get(&compu_method)
        .map(|cm| cm.unit.clone())
        .unwrap_or_default();

    let mut idx = 8;
    while idx < body.len() {
        let kw = body[idx].as_str();
        if kw.eq_ignore_ascii_case("PHYS_UNIT") && idx + 1 < body.len() {
            unit = body[idx + 1].as_str().to_string();
            idx += 2;
        } else if kw.eq_ignore_ascii_case("MATRIX_DIM") && idx + 1 < body.len() {
            idx = consume_matrix_dims(body, idx + 1, &mut matrix_dim);
        } else {
            idx += 1;
        }
    }

    Some(TypedefMeasurement {
        name,
        description,
        data_type,
        compu_method,
        lower_limit,
        upper_limit,
        matrix_dim,
        unit,
    })
}

fn parse_typedef_characteristic_block(
    body: &[A2lToken<'_>],
    model: &A2lModel,
) -> Option<TypedefCharacteristic> {
    // Name "Description" Type RecordLayout MaxDiff CompuMethod LowerLimit UpperLimit
    if body.len() < 8 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let char_type = CharacteristicType::from_keyword(body[2].as_str())?;
    let record_layout = body[3].as_str().to_string();
    let compu_method = body[5].as_str().to_string();
    let lower_limit = body[6].as_str().parse::<f64>().unwrap_or(f64::MIN);
    let upper_limit = body[7].as_str().parse::<f64>().unwrap_or(f64::MAX);
    let data_type = model
        .record_layouts
        .get(&record_layout)
        .copied()
        .or_else(|| infer_datatype_from_layout_name(&record_layout))
        .unwrap_or(A2lDataType::Float32Ieee);

    let mut matrix_dim = Vec::new();
    let mut unit = model
        .compu_methods
        .get(&compu_method)
        .map(|cm| cm.unit.clone())
        .unwrap_or_default();

    let mut idx = 8;
    while idx < body.len() {
        let kw = body[idx].as_str();
        if kw.eq_ignore_ascii_case("PHYS_UNIT") && idx + 1 < body.len() {
            unit = body[idx + 1].as_str().to_string();
            idx += 2;
        } else if (kw.eq_ignore_ascii_case("MATRIX_DIM") || kw.eq_ignore_ascii_case("NUMBER"))
            && idx + 1 < body.len()
        {
            idx = consume_matrix_dims(body, idx + 1, &mut matrix_dim);
        } else {
            idx += 1;
        }
    }

    Some(TypedefCharacteristic {
        name,
        description,
        char_type,
        record_layout,
        data_type,
        compu_method,
        lower_limit,
        upper_limit,
        matrix_dim,
        unit,
    })
}

fn parse_typedef_structure_block(body: &[A2lToken<'_>]) -> Option<TypedefStructure> {
    // Name "Description" ByteSize ... /begin STRUCTURE_COMPONENT Name TypeRef Offset ... /end STRUCTURE_COMPONENT
    if body.len() < 3 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let byte_size = parse_u64_literal(body[2].as_str()).unwrap_or(0) as u32;
    let mut components = Vec::new();

    let mut i = 3;
    while i < body.len() {
        if body[i] == A2lToken::Begin
            && i + 1 < body.len()
            && body[i + 1]
                .as_str()
                .eq_ignore_ascii_case("STRUCTURE_COMPONENT")
        {
            if let Ok(end) = find_matching_end(body, i, "STRUCTURE_COMPONENT") {
                let comp_body = &body[i + 2..end];
                if comp_body.len() >= 3 {
                    let comp_name = comp_body[0].as_str().to_string();
                    let type_ref = comp_body[1].as_str().to_string();
                    let address_offset =
                        parse_u64_literal(comp_body[2].as_str()).unwrap_or(0) as u32;
                    let mut matrix_dim = Vec::new();
                    let mut k = 3;
                    while k < comp_body.len() {
                        if comp_body[k].as_str().eq_ignore_ascii_case("MATRIX_DIM") {
                            k = consume_matrix_dims(comp_body, k + 1, &mut matrix_dim);
                        } else {
                            k += 1;
                        }
                    }
                    components.push(StructureComponent {
                        name: comp_name,
                        type_ref,
                        address_offset,
                        matrix_dim,
                    });
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }

    Some(TypedefStructure {
        name,
        description,
        byte_size,
        components,
    })
}

fn parse_instance_block(body: &[A2lToken<'_>]) -> Option<Instance> {
    // Name "Description" TypeRef Address ...
    if body.len() < 4 {
        return None;
    }
    let name = body[0].as_str().to_string();
    let description = body[1].as_str().to_string();
    let type_ref = body[2].as_str().to_string();
    let address = parse_u64_literal(body[3].as_str()).unwrap_or(0) as u32;

    let mut address_extension = 0u8;
    let mut matrix_dim = Vec::new();
    let mut event_id = None;

    let mut idx = 4;
    while idx < body.len() {
        let kw = body[idx].as_str();
        if kw.eq_ignore_ascii_case("ECU_ADDRESS_EXTENSION") && idx + 1 < body.len() {
            address_extension = parse_u64_literal(body[idx + 1].as_str()).unwrap_or(0) as u8;
            idx += 2;
        } else if kw.eq_ignore_ascii_case("MATRIX_DIM") && idx + 1 < body.len() {
            idx = consume_matrix_dims(body, idx + 1, &mut matrix_dim);
        } else if (kw.eq_ignore_ascii_case("EVENT")
            || kw.eq_ignore_ascii_case("DEFAULT_EVENT_LIST")
            || kw.eq_ignore_ascii_case("FIXED_EVENT_LIST"))
            && idx + 1 < body.len()
        {
            if let Some(ev) = parse_u64_literal(body[idx + 1].as_str()) {
                event_id = Some(ev as u16);
                idx += 2;
            } else {
                idx += 1;
            }
        } else {
            idx += 1;
        }
    }

    Some(Instance {
        name,
        description,
        type_ref,
        address,
        address_extension,
        matrix_dim,
        event_id,
    })
}
