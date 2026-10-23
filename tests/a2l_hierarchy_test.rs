// SPDX-License-Identifier: GPL-2.0-only
//! Unit and integration tests for A2L lexing, parsing, complex C/C++ structure
//! resolution (`TYPEDEF_STRUCTURE` / `INSTANCE` and dot paths), and `COMPU_METHOD` conversions.

use open_xcp_client::a2l::compu_method::{apply_physical_to_raw, apply_raw_to_physical};
use open_xcp_client::a2l::lexer::{parse_i64_literal, parse_u64_literal, tokenize_a2l, A2lToken};
use open_xcp_client::a2l::model::{A2lDataType, CharacteristicType, CompuMethod, CompuType};
use open_xcp_client::a2l::parser::{parse_a2l, A2lParseError};
use open_xcp_client::a2l::struct_resolver::split_hierarchical_segments;
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;

#[test]
fn test_a2l_datatypes_encode_decode_and_bounds() {
    let all_types = [
        (A2lDataType::UByte, "UBYTE", 1, true, 200.0),
        (A2lDataType::SByte, "SBYTE", 1, true, -42.0),
        (A2lDataType::UWord, "UWORD", 2, true, 50000.0),
        (A2lDataType::SWord, "SWORD", 2, true, -12345.0),
        (A2lDataType::ULong, "ULONG", 4, true, 3_000_000_000.0),
        (A2lDataType::SLong, "SLONG", 4, true, -1_000_000.0),
        (A2lDataType::AUint64, "A_UINT64", 8, true, 123_456_789.0),
        (A2lDataType::AInt64, "A_INT64", 8, true, -987_654_321.0),
        (A2lDataType::Float32Ieee, "FLOAT32_IEEE", 4, false, 12.5),
        (A2lDataType::Float64Ieee, "FLOAT64_IEEE", 8, false, -99.125),
    ];

    for (dt, kw, sz, is_int, sample_val) in all_types {
        assert_eq!(A2lDataType::from_keyword(kw), Some(dt));
        assert_eq!(dt.as_str(), kw);
        assert_eq!(dt.byte_size(), sz);
        assert_eq!(dt.is_integer(), is_int);
        let (min_b, max_b) = dt.raw_bounds();
        assert!(min_b < max_b);

        let encoded = dt.encode_raw_le(sample_val);
        assert_eq!(encoded.len(), sz);
        let decoded = dt.decode_raw_le(&encoded).unwrap();
        assert!((decoded - sample_val).abs() < 1e-4);
        assert!(dt.decode_raw_le(&[]).is_none());
    }
    assert!(A2lDataType::from_keyword("UNKNOWN_TYPE").is_none());

    assert_eq!(
        CharacteristicType::from_keyword("VALUE"),
        Some(CharacteristicType::Value)
    );
    assert_eq!(
        CharacteristicType::from_keyword("VAL_BLK"),
        Some(CharacteristicType::ValBlk)
    );
    assert_eq!(
        CharacteristicType::from_keyword("CURVE"),
        Some(CharacteristicType::Curve)
    );
    assert_eq!(
        CharacteristicType::from_keyword("MAP"),
        Some(CharacteristicType::Map)
    );
    assert_eq!(
        CharacteristicType::from_keyword("ASCII"),
        Some(CharacteristicType::Ascii)
    );
    assert_eq!(CharacteristicType::from_keyword("INVALID"), None);
}

#[test]
fn test_a2l_lexer_and_literals() {
    let src = r#"
        /* multi-line
           comment */
        // single line comment
        /begin PROJECT Demo "Quoted \"String\"" 0x1000 -42 /end PROJECT
    "#;
    let tokens = tokenize_a2l(src);
    assert_eq!(tokens[0], A2lToken::Begin);
    assert_eq!(tokens[0].as_str(), "/begin");
    assert_eq!(tokens[1].as_str(), "PROJECT");
    assert_eq!(tokens[2].as_str(), "Demo");
    assert_eq!(tokens[3], A2lToken::Quoted("Quoted \\\"String\\\""));
    assert_eq!(parse_u64_literal(tokens[4].as_str()), Some(0x1000));
    assert_eq!(parse_i64_literal(tokens[4].as_str()), Some(0x1000));
    assert_eq!(parse_i64_literal(tokens[5].as_str()), Some(-42));
    assert_eq!(tokens[6], A2lToken::End);
    assert_eq!(tokens[6].as_str(), "/end");
    assert_eq!(parse_u64_literal("not_a_number"), None);
    assert_eq!(parse_i64_literal("not_a_number"), None);
}

#[test]
fn test_compu_methods_roundtrip() {
    let cm_id = CompuMethod {
        name: "ID".into(),
        description: "".into(),
        unit: "m".into(),
        compu_type: CompuType::Identical,
    };
    assert_eq!(apply_raw_to_physical(&cm_id, 42.0), 42.0);
    assert_eq!(apply_physical_to_raw(&cm_id, 42.0), 42.0);

    let cm_lin = CompuMethod {
        name: "LIN".into(),
        description: "".into(),
        unit: "m/s".into(),
        compu_type: CompuType::Linear { a: 0.5, b: 10.0 },
    };
    let phys = apply_raw_to_physical(&cm_lin, 20.0);
    assert!((phys - 20.0).abs() < 1e-9);
    let raw = apply_physical_to_raw(&cm_lin, phys);
    assert!((raw - 20.0).abs() < 1e-9);

    let cm_rat = CompuMethod {
        name: "RAT".into(),
        description: "".into(),
        unit: "kPa".into(),
        compu_type: CompuType::RatFunc {
            a: 0.0,
            b: 2.0,
            c: 4.0,
            d: 0.0,
            e: 0.0,
            f: 1.0,
        },
    };
    let r = apply_physical_to_raw(&cm_rat, 10.0);
    assert!((r - 24.0).abs() < 1e-9);
    let p = apply_raw_to_physical(&cm_rat, r);
    assert!((p - 10.0).abs() < 1e-9);
}

#[test]
fn test_sample_adas_a2l_parsing_and_nested_structure_resolution() {
    let a2l_text = sample_adas_a2l_text();
    let model = parse_a2l(a2l_text).expect("Should parse sample ADAS A2L");

    assert_eq!(model.project_name, "OpenXcpAdasProject");
    assert_eq!(model.module_name, "AdasCoreModule");
    assert_eq!(model.events.len(), 2);
    assert!((model.events[&0].refresh_rate_ms() - 10.0).abs() < 1e-6);
    assert!((model.events[&1].refresh_rate_ms() - 50.0).abs() < 1e-6);

    // Check resolved INSTANCE -> TYPEDEF_STRUCTURE -> nested array of TS_RadarTarget
    let radar_t0_dist = model
        .measurements
        .iter()
        .find(|m| m.name == "Perception.RadarFront.targets[0].distance_m")
        .expect("Should resolve nested struct array target[0].distance_m");
    assert_eq!(radar_t0_dist.address, 0x1008);
    assert_eq!(radar_t0_dist.event_id, Some(0));
    assert_eq!(radar_t0_dist.unit, "m");

    let radar_t1_snr = model
        .measurements
        .iter()
        .find(|m| m.name == "Perception.RadarFront.targets[1].snr_db")
        .expect("Should resolve nested struct array target[1].snr_db");
    assert_eq!(radar_t1_snr.address, 0x1024);

    // Check resolved TYPEDEF_CHARACTERISTIC inside INSTANCE
    let radar_conf = model
        .characteristics
        .iter()
        .find(|c| c.name == "Perception.RadarFront.min_confidence")
        .expect("Should resolve characteristic inside struct instance");
    assert_eq!(radar_conf.address, 0x1004);

    // Check 1D array measurement MATRIX_DIM
    let cov = model
        .measurements
        .iter()
        .find(|m| m.name == "EgoMotion.Pose.covariance_diag")
        .unwrap();
    assert_eq!(cov.matrix_dim, vec![3]);
    assert_eq!(cov.element_count(), 3);
    assert_eq!(cov.total_byte_size(), 12);

    // Check hierarchy tree
    assert!(model.hierarchy_root.children.contains_key("Perception"));
    assert!(model.hierarchy_root.children.contains_key("EgoMotion"));
    assert_eq!(
        split_hierarchical_segments("Perception.RadarFront.targets[0].distance_m"),
        vec!["Perception", "RadarFront", "targets_0", "distance_m"]
    );

    // Check parser error cases
    assert_eq!(parse_a2l("   "), Err(A2lParseError::EmptyInput));
    assert!(matches!(
        parse_a2l("/begin MEASUREMENT unclosed"),
        Err(A2lParseError::UnexpectedEof(_))
    ));
}

#[test]
fn test_a2l_extended_constructs_and_edge_cases() {
    let custom_a2l = r#"
    /begin PROJECT ExtProj "Extended"
      /begin MODULE ExtMod "Module"
        /begin EVENT "Ev1us" "E0" 0 DAQ 1 5 3 0 /end EVENT
        /begin EVENT "Ev10us" "E1" 1 DAQ 1 5 4 0 /end EVENT
        /begin EVENT "Ev100us" "E2" 2 DAQ 1 5 5 0 /end EVENT
        /begin EVENT "Ev10ms" "E3" 3 DAQ 1 5 7 0 /end EVENT
        /begin EVENT "Ev100ms" "E4" 4 DAQ 1 5 8 0 /end EVENT
        /begin EVENT "Ev1s" "E5" 5 DAQ 1 5 9 0 /end EVENT
        /begin TYPEDEF_MEASUREMENT TM_Arr "Arr" SWORD CM_NONE 0 0 -100 100 MATRIX_DIM 2 PHYS_UNIT "deg" /end TYPEDEF_MEASUREMENT
        /begin TYPEDEF_CHARACTERISTIC TC_Arr "CArr" VAL_BLK Layout_S8 0 CM_NONE -10 10 MATRIX_DIM 2 PHYS_UNIT "rad" /end TYPEDEF_CHARACTERISTIC
        /begin TYPEDEF_STRUCTURE TS_Item "Item" 0x10
          /begin STRUCTURE_COMPONENT m_arr TM_Arr 0x00 /end STRUCTURE_COMPONENT
          /begin STRUCTURE_COMPONENT c_arr TC_Arr 0x04 /end STRUCTURE_COMPONENT
        /end TYPEDEF_STRUCTURE
        /begin INSTANCE MultiInst "Multi" TS_Item 0x4000 ECU_ADDRESS_EXTENSION 1 MATRIX_DIM 2 DEFAULT_EVENT_LIST 1 /end INSTANCE
        /begin CHARACTERISTIC C_I16 "desc" VALUE 0x5000 Layout_I16 0 CM_NONE -100 100 NUMBER 2 /end CHARACTERISTIC
        /begin CHARACTERISTIC C_I32 "desc" VALUE 0x5010 Layout_I32 0 CM_NONE -100 100 /end CHARACTERISTIC
        /begin CHARACTERISTIC C_U64 "desc" VALUE 0x5020 Layout_U64 0 CM_NONE 0 100 /end CHARACTERISTIC
        /begin CHARACTERISTIC C_I64 "desc" VALUE 0x5030 Layout_I64 0 CM_NONE -100 100 /end CHARACTERISTIC
      /end MODULE
    /end PROJECT
    "#;
    let model = parse_a2l(custom_a2l).unwrap();
    assert_eq!(model.events.len(), 6);
    assert_eq!(model.events[&0].cycle_time_us, 5);
    assert_eq!(model.events[&1].cycle_time_us, 50);
    assert_eq!(model.events[&2].cycle_time_us, 500);
    assert_eq!(model.events[&3].cycle_time_us, 50_000);
    assert_eq!(model.events[&4].cycle_time_us, 500_000);
    assert_eq!(model.events[&5].cycle_time_us, 5_000_000);

    // MultiInst[0] and MultiInst[1] expanded
    assert!(model
        .measurements
        .iter()
        .any(|m| m.name == "MultiInst[0].m_arr" && m.address_extension == 1));
    assert!(model
        .measurements
        .iter()
        .any(|m| m.name == "MultiInst[1].m_arr" && m.address == 0x4010));
    assert!(model
        .characteristics
        .iter()
        .any(|c| c.name == "MultiInst[1].c_arr"));

    // Nonlinear RatFunc & zero-slope Linear fallback
    let cm_zero = CompuMethod {
        name: "Z".into(),
        description: "".into(),
        unit: "".into(),
        compu_type: CompuType::Linear { a: 0.0, b: 5.0 },
    };
    assert_eq!(apply_physical_to_raw(&cm_zero, 12.0), 12.0);

    let cm_nonlin = CompuMethod {
        name: "NL".into(),
        description: "".into(),
        unit: "".into(),
        compu_type: CompuType::RatFunc {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        },
    };
    assert_eq!(apply_raw_to_physical(&cm_nonlin, 7.0), 7.0);
    assert_eq!(apply_physical_to_raw(&cm_nonlin, 7.0), 7.0);
}
