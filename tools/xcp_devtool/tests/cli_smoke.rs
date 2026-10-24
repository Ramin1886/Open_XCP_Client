// SPDX-License-Identifier: GPL-2.0-only
//! CLI smoke tests: every subcommand that needs no network produces the documented JSON.

use chrono::{TimeZone, Utc};
use open_xcp_client::a2l::parse_a2l;
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use open_xcp_client::recording::SdfSessionRecorder;
use open_xcp_client::xcp::daq_planner::{DaqEventSample, DecodedValue};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn devtool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xcp_devtool"))
}

fn run_json(args: &[&str]) -> (Value, i32) {
    let out = devtool().args(args).output().expect("spawn devtool");
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "non-JSON output for {:?} (exit {code}): {e}\nstdout: {stdout}\nstderr: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (v, code)
}

#[test]
fn a2l_inspect_reports_sample_events_and_counts() {
    let (v, code) = run_json(&["a2l-inspect", "--builtin", "adas_sample"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    let events: Vec<&str> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(events, vec!["Event_10ms", "Event_50ms"]);
    assert_eq!(v["counts"]["measurements"], 13);
    assert!(v["struct_tree"].as_array().unwrap().len() >= 2);
    assert!(
        v["unrecognized_blocks"].as_array().unwrap().is_empty(),
        "{v}"
    );
}

#[test]
fn a2l_inspect_rejects_missing_file_with_exit_2() {
    let out = devtool()
        .args(["a2l-inspect", "--path", "/nonexistent/file.a2l"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}

#[test]
fn a2l_find_signal_substring_and_glob() {
    let (v, _) = run_json(&["a2l-find-signal", "--builtin", "adas_sample", "radarfront"]);
    assert!(v["total_matches"].as_u64().unwrap() >= 1, "{v}");
    let (v, _) = run_json(&["a2l-find-signal", "--builtin", "adas_sample", "*distance*"]);
    assert!(v["total_matches"].as_u64().unwrap() >= 1, "{v}");
    let (v, _) = run_json(&[
        "a2l-find-signal",
        "--builtin",
        "adas_sample",
        "zzz_no_such_signal",
    ]);
    assert_eq!(v["total_matches"], 0);
}

#[test]
fn daq_plan_dryrun_has_no_violations_across_mtu_range() {
    for mtu in ["8", "32", "256", "1024"] {
        let (v, code) = run_json(&[
            "daq-plan-dryrun",
            "--builtin",
            "adas_sample",
            "--max-dto",
            mtu,
        ]);
        assert_eq!(code, 0, "max_dto {mtu}: {v}");
        assert_eq!(
            v["violations"].as_array().unwrap().len(),
            0,
            "max_dto {mtu}"
        );
        assert_eq!(v["totals"]["daq_lists"], 2);
        assert_eq!(v["totals"]["odt_entries"], 13);
    }
}

#[test]
fn daq_plan_dryrun_signal_filter_narrows_plan() {
    let (v, _) = run_json(&[
        "daq-plan-dryrun",
        "--builtin",
        "adas_sample",
        "--signal",
        "EgoMotion*",
    ]);
    assert_eq!(v["ok"], true);
    assert!(v["measurements_considered"].as_u64().unwrap() < 13);
}

#[test]
fn invariants_check_passes_on_clean_tree() {
    let (v, code) = run_json(&["invariants-check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    let names: Vec<&str> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "spdx",
            "no_unwrap_in_src",
            "version_consistency",
            "reserved_pid_limit"
        ]
    );
}

#[test]
fn invariants_check_flags_missing_spdx_in_synthetic_repo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(st.status.success(), "git {:?}", args);
    };
    git(&["init", "-q"]);
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.1\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn f() -> i32 { \"1\".parse().unwrap() }\n",
    )
    .unwrap();
    git(&["add", "."]);
    let (v, code) = run_json(&[
        "invariants-check",
        "--repo-root",
        root.to_str().unwrap(),
        "--check",
        "spdx",
        "--check",
        "no-unwrap-in-src",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    let checks = v["checks"].as_array().unwrap();
    assert_eq!(checks[0]["pass"], false);
    assert!(checks[0]["violations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("src/lib.rs")));
    assert_eq!(checks[1]["pass"], false);
}

fn write_sample_sdf(folder: &Path) -> std::path::PathBuf {
    let model = parse_a2l(sample_adas_a2l_text()).unwrap();
    let start = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let mut rec =
        SdfSessionRecorder::new(&model, folder, "Smoke", start, 64 * 1024 * 1024).unwrap();
    let ev0: Vec<usize> = model
        .measurements
        .iter()
        .enumerate()
        .filter(|(_, m)| m.event_id == Some(0))
        .map(|(i, _)| i)
        .collect();
    for k in 0..50u32 {
        let mut values = BTreeMap::new();
        for &i in &ev0 {
            let m = &model.measurements[i];
            let v = if m.element_count() > 1 {
                DecodedValue::Array(vec![k as f64; m.element_count()])
            } else {
                DecodedValue::Scalar(k as f64)
            };
            values.insert(i, v);
        }
        let t = 1_767_322_245.0 + k as f64 * 0.01;
        rec.record_event_sample(&DaqEventSample {
            daq_list_num: 0,
            event_channel_id: 0,
            event_name: "Event_10ms".to_string(),
            time_utc_s: t,
            time_utc_ns: (t * 1e9) as i64,
            values,
        })
        .unwrap();
    }
    let files = rec.finish().unwrap();
    files[0].clone()
}

#[test]
fn sdf_inspect_passes_on_freshly_recorded_part_and_fails_on_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let sdf = write_sample_sdf(dir.path());
    let (v, code) = run_json(&["sdf-inspect", sdf.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["part_number"], 1);
    for c in v["checks"].as_array().unwrap() {
        assert_eq!(
            c["pass"], true,
            "check {} failed: {}",
            c["name"], c["detail"]
        );
    }
    let ev = &v["events"].as_array().unwrap()[0];
    assert_eq!(ev["sample_count"], 50);
    assert_eq!(ev["time_monotonic"], true);
    assert_eq!(v["integrity"]["is_clean"], true);

    // Truncate the tail index and expect the reader to refuse the file.
    let bytes = std::fs::read(&sdf).unwrap();
    let truncated = dir.path().join("Smoke_truncated_PART001.sdf");
    std::fs::write(&truncated, &bytes[..bytes.len() - 100]).unwrap();
    let (v, code) = run_json(&["sdf-inspect", truncated.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert_eq!(v["checks"][0]["pass"], false);
}

#[test]
fn dump_sample_a2l_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("s.a2l");
    let (v, code) = run_json(&["dump-sample-a2l", "--out", out.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        sample_adas_a2l_text()
    );
}
