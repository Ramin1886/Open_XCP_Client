// SPDX-License-Identifier: GPL-2.0-only
//! `invariants_check`: repository-level rules that do not need a compiler.

use crate::error::{DevtoolError, DevtoolResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Which checks to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvariantName {
    /// Every tracked `.rs`, `.toml`, `.yml`, `.sh` file carries the SPDX header in its first 3 lines.
    Spdx,
    /// No `.unwrap()` / `.expect(` in `src/` outside the allowlist.
    NoUnwrapInSrc,
    /// `Cargo.toml` version equals the `open_xcp_client` entry in `Cargo.lock`.
    VersionConsistency,
    /// `MAX_ODTS_PER_DAQ_LIST` leaves packet identifiers 0xFC..0xFF free.
    ReservedPidLimit,
}

const ALL: [InvariantName; 4] = [
    InvariantName::Spdx,
    InvariantName::NoUnwrapInSrc,
    InvariantName::VersionConsistency,
    InvariantName::ReservedPidLimit,
];

/// Input of `invariants_check`.
#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
pub struct InvariantsInput {
    /// Subset of checks; `null` runs all.
    #[serde(default)]
    pub checks: Option<Vec<InvariantName>>,
    /// Repository root; defaults to the current directory's git top level.
    #[serde(default)]
    pub repo_root: Option<String>,
}

/// Severity of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Failing flips `ok` to false.
    Error,
    /// Reported but does not flip `ok`.
    Warning,
}

/// Result of one check.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InvariantResult {
    pub name: InvariantName,
    pub severity: Severity,
    pub pass: bool,
    pub violations: Vec<String>,
    pub detail: String,
}

/// Output of `invariants_check`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InvariantsOutput {
    pub ok: bool,
    pub repo_root: String,
    pub checks: Vec<InvariantResult>,
    pub problems: Vec<String>,
}

/// Resolves the repository root from the current directory.
pub fn repo_root(explicit: Option<&str>) -> DevtoolResult<PathBuf> {
    if let Some(r) = explicit {
        return Ok(PathBuf::from(r));
    }
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| DevtoolError::Process(format!("git rev-parse: {e}")))?;
    if !out.status.success() {
        return Err(DevtoolError::Process(
            "not inside a git repository; pass repo_root".to_string(),
        ));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn tracked_files(root: &Path) -> DevtoolResult<Vec<PathBuf>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| DevtoolError::Process(format!("git ls-files: {e}")))?;
    Ok(out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| root.join(String::from_utf8_lossy(s).as_ref()))
        .collect())
}

const SPDX_LINE: &str = "SPDX-License-Identifier: GPL-2.0-only";

fn check_spdx(root: &Path) -> DevtoolResult<InvariantResult> {
    let mut violations = Vec::new();
    let mut scanned = 0;
    for f in tracked_files(root)? {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "rs" | "toml" | "yml" | "yaml" | "sh") {
            continue;
        }
        if f.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock") {
            continue;
        }
        scanned += 1;
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        if !text.lines().take(3).any(|l| l.contains(SPDX_LINE)) {
            violations.push(rel(root, &f));
        }
    }
    Ok(InvariantResult {
        name: InvariantName::Spdx,
        severity: Severity::Error,
        pass: violations.is_empty(),
        detail: format!("{scanned} files scanned"),
        violations,
    })
}

fn rel(root: &Path, f: &Path) -> String {
    f.strip_prefix(root)
        .unwrap_or(f)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Allowlist file: one `path/within/repo.rs|exact line substring` per line; `#` comments.
pub const UNWRAP_ALLOWLIST: &str = "tools/xcp_devtool/unwrap_allowlist.txt";

fn check_no_unwrap(root: &Path) -> DevtoolResult<InvariantResult> {
    let allow: Vec<(String, String)> = std::fs::read_to_string(root.join(UNWRAP_ALLOWLIST))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            l.split_once('|')
                .map(|(p, s)| (p.trim().to_string(), s.trim().to_string()))
        })
        .collect();
    let mut violations = Vec::new();
    let mut hits = 0;
    for f in tracked_files(root)? {
        let r = rel(root, &f);
        if !(r.starts_with("src/") && r.ends_with(".rs")) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        let mut in_test_mod = false;
        for (i, line) in text.lines().enumerate() {
            if line.contains("#[cfg(test)]") {
                in_test_mod = true;
            }
            if in_test_mod {
                continue;
            }
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if line.contains(".unwrap()") || line.contains(".expect(") {
                hits += 1;
                let allowed = allow
                    .iter()
                    .any(|(p, s)| p == &r && line.contains(s.as_str()));
                if !allowed {
                    violations.push(format!("{r}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    Ok(InvariantResult {
        name: InvariantName::NoUnwrapInSrc,
        severity: Severity::Error,
        pass: violations.is_empty(),
        detail: format!("{hits} unwrap/expect sites, {} allowlisted", allow.len()),
        violations,
    })
}

fn toml_version(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
        } else if in_package {
            if let Some(v) = t.strip_prefix("version") {
                let v = v.trim_start().strip_prefix('=')?.trim();
                return Some(v.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn check_version(root: &Path) -> DevtoolResult<InvariantResult> {
    let cargo = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    let mut violations = Vec::new();
    let version = toml_version(&cargo).unwrap_or_default();
    if version.is_empty() {
        violations.push("Cargo.toml has no [package] version".to_string());
    }
    let lock_marker = "name = \"open_xcp_client\"";
    let lock_version = lock
        .split("[[package]]")
        .find(|blk| blk.contains(lock_marker))
        .and_then(|blk| {
            blk.lines()
                .find_map(|l| l.trim().strip_prefix("version = "))
                .map(|v| v.trim_matches('"').to_string())
        });
    if let Some(lv) = &lock_version {
        if lv != &version {
            violations.push(format!(
                "Cargo.lock has open_xcp_client {lv} but Cargo.toml says {version}; run cargo build"
            ));
        }
    }
    Ok(InvariantResult {
        name: InvariantName::VersionConsistency,
        severity: Severity::Error,
        pass: violations.is_empty(),
        detail: format!("Cargo.toml version {version}"),
        violations,
    })
}

fn check_reserved_pid(root: &Path) -> DevtoolResult<InvariantResult> {
    let text = std::fs::read_to_string(root.join("src/xcp/daq_planner.rs"))?;
    let value = text.lines().find_map(|l| {
        let t = l.trim();
        t.strip_prefix("pub const MAX_ODTS_PER_DAQ_LIST: usize = ")
            .and_then(|v| v.trim_end_matches(';').trim().parse::<usize>().ok())
    });
    let (pass, detail) = match value {
        Some(v) if v <= 252 => (true, format!("MAX_ODTS_PER_DAQ_LIST = {v}")),
        Some(v) => (
            false,
            format!("MAX_ODTS_PER_DAQ_LIST = {v}; ODT numbers 252..254 collide with reserved PIDs 0xFC..0xFF"),
        ),
        None => (false, "constant not found".to_string()),
    };
    Ok(InvariantResult {
        name: InvariantName::ReservedPidLimit,
        severity: Severity::Error,
        pass,
        violations: if pass {
            Vec::new()
        } else {
            vec![detail.clone()]
        },
        detail,
    })
}

/// Runs `invariants_check`.
pub fn run(input: &InvariantsInput) -> DevtoolResult<InvariantsOutput> {
    let root = repo_root(input.repo_root.as_deref())?;
    let wanted: Vec<InvariantName> = input.checks.clone().unwrap_or_else(|| ALL.to_vec());
    let mut checks = Vec::new();
    for name in wanted {
        checks.push(match name {
            InvariantName::Spdx => check_spdx(&root)?,
            InvariantName::NoUnwrapInSrc => check_no_unwrap(&root)?,
            InvariantName::VersionConsistency => check_version(&root)?,
            InvariantName::ReservedPidLimit => check_reserved_pid(&root)?,
        });
    }
    let problems: Vec<String> = checks
        .iter()
        .filter(|c| !c.pass)
        .map(|c| format!("{:?} ({:?}): {}", c.name, c.severity, c.detail))
        .collect();
    let ok = checks
        .iter()
        .all(|c| c.pass || c.severity == Severity::Warning);
    Ok(InvariantsOutput {
        ok,
        repo_root: root.to_string_lossy().to_string(),
        checks,
        problems,
    })
}
