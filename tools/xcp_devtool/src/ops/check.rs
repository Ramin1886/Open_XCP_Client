// SPDX-License-Identifier: GPL-2.0-only
//! `project_check`: run fmt / clippy / test for one package and return structured results.

use crate::error::{DevtoolError, DevtoolResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// A verification step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStep {
    Fmt,
    Clippy,
    Test,
}

/// Input of `project_check`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectCheckInput {
    /// Steps to run in order (default fmt, clippy, test).
    #[serde(default = "default_steps")]
    pub steps: Vec<CheckStep>,
    /// Cargo package (default `open_xcp_client`).
    #[serde(default = "default_package")]
    pub package: String,
    /// Optional `cargo test` name filter.
    #[serde(default)]
    pub test_filter: Option<String>,
    /// Run `cargo fmt` without `--check`, rewriting files (default false).
    #[serde(default)]
    pub fix_fmt: bool,
    /// Keep running later steps after a failure (default false).
    #[serde(default)]
    pub continue_on_fail: bool,
    /// Per-step timeout in seconds (default 600).
    #[serde(default = "default_timeout")]
    pub timeout_s: u64,
    /// Repository root; defaults to the git top level of the current directory.
    #[serde(default)]
    pub repo_root: Option<String>,
}

fn default_steps() -> Vec<CheckStep> {
    vec![CheckStep::Fmt, CheckStep::Clippy, CheckStep::Test]
}
fn default_package() -> String {
    "open_xcp_client".to_string()
}
fn default_timeout() -> u64 {
    600
}

/// Result of one step.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StepResult {
    pub name: CheckStep,
    pub pass: bool,
    pub skipped: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub command: String,
    /// First `error` lines (with their `-->` location) from the output, at most 10.
    pub first_errors: Vec<String>,
    /// Last lines of output, for context.
    pub tail: Vec<String>,
}

/// Output of `project_check`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectCheckOutput {
    pub ok: bool,
    pub package: String,
    pub steps: Vec<StepResult>,
    pub problems: Vec<String>,
}

fn args_for(step: CheckStep, input: &ProjectCheckInput) -> Vec<String> {
    let mut a: Vec<String> = match step {
        CheckStep::Fmt => {
            let mut v = vec!["fmt".to_string(), "--all".to_string()];
            if !input.fix_fmt {
                v.extend(["--".to_string(), "--check".to_string()]);
            }
            v
        }
        CheckStep::Clippy => vec![
            "clippy".into(),
            "-p".into(),
            input.package.clone(),
            "--all-targets".into(),
            "--".into(),
            "-D".into(),
            "warnings".into(),
        ],
        CheckStep::Test => {
            let mut v = vec!["test".to_string(), "-p".to_string(), input.package.clone()];
            if let Some(f) = &input.test_filter {
                v.push(f.clone());
            }
            v
        }
    };
    a.insert(0, "cargo".to_string());
    a
}

/// Extracts `error...` lines with the following `-->` location line.
pub fn parse_first_errors(output: &str, max: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() && out.len() < max {
        let l = lines[i].trim_start();
        if l.starts_with("error") && !l.starts_with("error: could not compile") {
            let mut entry = l.to_string();
            for next in lines.iter().skip(i + 1).take(4) {
                if next.trim_start().starts_with("-->") {
                    entry.push(' ');
                    entry.push_str(next.trim());
                    break;
                }
            }
            out.push(entry);
        } else if l.starts_with("test ") && l.ends_with("FAILED") {
            out.push(l.to_string());
        }
        i += 1;
    }
    out
}

/// Runs `project_check`.
pub async fn run(input: &ProjectCheckInput) -> DevtoolResult<ProjectCheckOutput> {
    let root = super::invariants::repo_root(input.repo_root.as_deref())?;
    let mut steps = Vec::new();
    let mut failed = false;
    for &step in &input.steps {
        if failed && !input.continue_on_fail {
            steps.push(StepResult {
                name: step,
                pass: false,
                skipped: true,
                exit_code: None,
                duration_ms: 0,
                command: args_for(step, input).join(" "),
                first_errors: Vec::new(),
                tail: Vec::new(),
            });
            continue;
        }
        let res = run_step(step, input, &root).await?;
        if !res.pass {
            failed = true;
        }
        steps.push(res);
    }
    let problems = steps
        .iter()
        .filter(|s| !s.pass && !s.skipped)
        .map(|s| {
            format!(
                "{:?} failed (exit {:?}): {}",
                s.name,
                s.exit_code,
                s.first_errors.first().cloned().unwrap_or_default()
            )
        })
        .collect();
    Ok(ProjectCheckOutput {
        ok: !failed,
        package: input.package.clone(),
        steps,
        problems,
    })
}

async fn run_step(
    step: CheckStep,
    input: &ProjectCheckInput,
    root: &Path,
) -> DevtoolResult<StepResult> {
    let argv = args_for(step, input);
    let start = Instant::now();
    let fut = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(input.timeout_s), fut).await {
        Ok(r) => r.map_err(|e| DevtoolError::Process(format!("{}: {e}", argv.join(" "))))?,
        Err(_) => {
            return Ok(StepResult {
                name: step,
                pass: false,
                skipped: false,
                exit_code: None,
                duration_ms: start.elapsed().as_millis(),
                command: argv.join(" "),
                first_errors: vec![format!("timed out after {} s", input.timeout_s)],
                tail: Vec::new(),
            })
        }
    };
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let tail: Vec<String> = combined
        .lines()
        .rev()
        .take(15)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(StepResult {
        name: step,
        pass: output.status.success(),
        skipped: false,
        exit_code: output.status.code(),
        duration_ms: start.elapsed().as_millis(),
        command: argv.join(" "),
        first_errors: parse_first_errors(&combined, 10),
        tail,
    })
}
