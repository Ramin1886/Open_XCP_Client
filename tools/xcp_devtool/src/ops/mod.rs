// SPDX-License-Identifier: GPL-2.0-only
//! Operations shared by the CLI subcommands and the MCP tools. Every op returns a
//! `Serialize + JsonSchema` struct so both front-ends emit identical JSON.

pub mod a2l;
pub mod check;
pub mod daq;
pub mod invariants;
pub mod mock;
pub mod probe;
pub mod sdf;

use crate::error::{DevtoolError, DevtoolResult};
use open_xcp_client::a2l::{parse_a2l, A2lModel};
use open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Name of the A2L text embedded in the library's mock server.
pub const BUILTIN_ADAS_SAMPLE: &str = "adas_sample";

/// Where an A2L file comes from: a path on disk or the built-in sample.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum A2lSource {
    /// Path to an `.a2l` file on disk.
    Path {
        /// Filesystem path to the A2L file.
        path: String,
    },
    /// A built-in sample; currently only `"adas_sample"`.
    Builtin {
        /// Built-in sample name.
        builtin: String,
    },
}

impl A2lSource {
    /// Human-readable description of the source for output fields.
    pub fn describe(&self) -> String {
        match self {
            A2lSource::Path { path } => path.clone(),
            A2lSource::Builtin { builtin } => format!("builtin:{builtin}"),
        }
    }
}

/// Reads the A2L text for `source`.
pub fn load_a2l_text(source: &A2lSource) -> DevtoolResult<String> {
    match source {
        A2lSource::Path { path } => {
            let p = Path::new(path);
            if !p.is_file() {
                return Err(DevtoolError::Invalid(format!("A2L file not found: {path}")));
            }
            Ok(std::fs::read_to_string(p)?)
        }
        A2lSource::Builtin { builtin } if builtin == BUILTIN_ADAS_SAMPLE => {
            Ok(sample_adas_a2l_text().to_string())
        }
        A2lSource::Builtin { builtin } => Err(DevtoolError::Invalid(format!(
            "unknown builtin A2L '{builtin}'; available: {BUILTIN_ADAS_SAMPLE}"
        ))),
    }
}

/// Reads and parses the A2L for `source`, returning both the text and the resolved model.
pub fn load_a2l(source: &A2lSource) -> DevtoolResult<(String, A2lModel)> {
    let text = load_a2l_text(source)?;
    let model = parse_a2l(&text)?;
    Ok((text, model))
}

/// Case-insensitive match of `name` against `pattern`, where `*` matches any run of characters.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    fn rec(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|k| rec(&p[1..], &n[k..])),
            Some(c) => n.first() == Some(c) && rec(&p[1..], &n[1..]),
        }
    }
    if pattern.contains('*') {
        rec(&p, &n)
    } else {
        name.to_lowercase().contains(&pattern.to_lowercase())
    }
}
