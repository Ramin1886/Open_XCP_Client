// SPDX-License-Identifier: GPL-2.0-only
//! Speaks JSON-RPC to the `mcp` subcommand over stdio: initialize, list tools, call one.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn read_response(reader: &mut impl BufRead, id: u64) -> Value {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read stdout");
        assert!(n > 0, "server closed stdout before answering id {id}");
        let v: Value = serde_json::from_str(line.trim()).expect("stdout must be JSON-RPC");
        if v["id"] == json!(id) {
            return v;
        }
    }
}

#[test]
fn mcp_stdio_lists_eight_tools_and_calls_a2l_inspect() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_xcp_devtool"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp server");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let send = |stdin: &mut std::process::ChildStdin, v: Value| {
        stdin.write_all(v.to_string().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    };

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"smoke","version":"0"}}}),
    );
    let init = read_response(&mut stdout, 1);
    assert_eq!(
        init["result"]["serverInfo"]["name"], "xcp_devtool",
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let list = read_response(&mut stdout, 2);
    let mut names: Vec<String> = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "a2l_find_signal",
            "a2l_inspect",
            "daq_plan_dryrun",
            "invariants_check",
            "mock_server",
            "project_check",
            "sdf_inspect",
            "xcp_probe",
        ]
    );
    for t in list["result"]["tools"].as_array().unwrap() {
        assert!(
            t["inputSchema"].is_object(),
            "tool {} has no input schema",
            t["name"]
        );
        assert!(
            t["description"].as_str().unwrap().len() > 20,
            "tool {} has no description",
            t["name"]
        );
    }

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"a2l_inspect","arguments":{"source":{"builtin":"adas_sample"}}}}),
    );
    let call = read_response(&mut stdout, 3);
    let structured = &call["result"]["structuredContent"];
    assert_eq!(structured["ok"], true, "{call}");
    assert_eq!(structured["counts"]["measurements"], 13);

    // Invalid builtin -> JSON-RPC error with invalid params, not a crash.
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"a2l_inspect","arguments":{"source":{"builtin":"nope"}}}}),
    );
    let bad = read_response(&mut stdout, 4);
    assert!(
        bad["error"].is_object() || bad["result"]["isError"] == json!(true),
        "{bad}"
    );

    drop(stdin);
    let status = child.wait().expect("wait");
    assert!(status.success(), "server exit status {status}");
}
