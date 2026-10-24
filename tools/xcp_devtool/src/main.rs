// SPDX-License-Identifier: GPL-2.0-only
//! `xcp_devtool`: developer CLI and MCP stdio server for `open_xcp_client`.
//!
//! Every subcommand prints one JSON document (the same shape the MCP tool returns) and
//! exits 0 when `ok` is true, 1 when `ok` is false, and 2 on an operational error.

mod error;
mod mcp;
mod ops;

use clap::{Args, Parser, Subcommand, ValueEnum};
use ops::a2l::{A2lFindSignalInput, A2lInspectInput, SignalKind};
use ops::check::{CheckStep, ProjectCheckInput};
use ops::daq::DaqPlanInput;
use ops::invariants::{InvariantName, InvariantsInput};
use ops::mock::{MockAction, MockRegistry, MockServerInput};
use ops::probe::{ProbeProtocol, XcpProbeInput};
use ops::sdf::SdfInspectInput;
use ops::A2lSource;
use serde::Serialize;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "xcp_devtool",
    version,
    about = "Developer CLI and MCP server for open_xcp_client"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// A2L source arguments shared by several subcommands.
#[derive(Args, Clone)]
struct SourceArgs {
    /// Path to an .a2l file.
    #[arg(long, conflicts_with = "builtin")]
    path: Option<String>,
    /// Built-in sample name (adas_sample).
    #[arg(long)]
    builtin: Option<String>,
}

impl SourceArgs {
    fn into_source(self) -> Result<A2lSource, String> {
        match (self.path, self.builtin) {
            (Some(path), _) => Ok(A2lSource::Path { path }),
            (None, Some(builtin)) => Ok(A2lSource::Builtin { builtin }),
            (None, None) => Err("pass --path <file.a2l> or --builtin adas_sample".to_string()),
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Measurement,
    Characteristic,
    Any,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProtoArg {
    Udp,
    Tcp,
}

#[derive(Clone, Copy, ValueEnum)]
enum StepArg {
    Fmt,
    Clippy,
    Test,
}

#[derive(Clone, Copy, ValueEnum)]
enum InvariantArg {
    Spdx,
    NoUnwrapInSrc,
    VersionConsistency,
    ReservedPidLimit,
}

#[derive(Subcommand)]
enum Cmd {
    /// Summarise an A2L file.
    A2lInspect {
        #[command(flatten)]
        source: SourceArgs,
        /// Structure tree depth.
        #[arg(long, default_value_t = 3)]
        max_tree_depth: usize,
    },
    /// Search measurements/characteristics by name.
    A2lFindSignal {
        #[command(flatten)]
        source: SourceArgs,
        /// Substring or * glob.
        query: String,
        #[arg(long, value_enum, default_value_t = KindArg::Any)]
        kind: KindArg,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Inspect and check one .sdf part.
    SdfInspect {
        path: String,
        /// Skip reading time datasets.
        #[arg(long)]
        no_data: bool,
        #[arg(long, default_value_t = 200)]
        max_datasets: usize,
    },
    /// Dry-run the DAQ/ODT planner.
    DaqPlanDryrun {
        #[command(flatten)]
        source: SourceArgs,
        #[arg(long, default_value_t = 256)]
        max_dto: usize,
        /// Measurement filters (repeatable; substring or * glob).
        #[arg(long = "signal")]
        signals: Vec<String>,
    },
    /// Start a mock xcplite server in the foreground (Ctrl-C to stop).
    MockServer {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// CONNECT / GET_STATUS / GET_COMM_MODE_INFO / DISCONNECT against a target.
    XcpProbe {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 5555)]
        port: u16,
        #[arg(long, value_enum, default_value_t = ProtoArg::Udp)]
        protocol: ProtoArg,
        #[arg(long, default_value_t = 1000)]
        timeout_ms: u64,
    },
    /// Run fmt / clippy / test for a package.
    ProjectCheck {
        /// Steps in order (default fmt clippy test).
        #[arg(long = "step", value_enum)]
        steps: Vec<StepArg>,
        #[arg(long, default_value = "open_xcp_client")]
        package: String,
        #[arg(long)]
        test_filter: Option<String>,
        /// Rewrite files with cargo fmt instead of checking.
        #[arg(long)]
        fix_fmt: bool,
        #[arg(long)]
        continue_on_fail: bool,
        #[arg(long, default_value_t = 600)]
        timeout_s: u64,
    },
    /// Check repository invariants.
    InvariantsCheck {
        /// Checks to run (default all).
        #[arg(long = "check", value_enum)]
        checks: Vec<InvariantArg>,
        #[arg(long)]
        repo_root: Option<String>,
    },
    /// Write the built-in sample A2L to a file (or stdout).
    DumpSampleA2l {
        #[arg(long)]
        out: Option<String>,
    },
    /// Run as an MCP server over stdio.
    Mcp,
}

fn print_json<T: Serialize>(value: &T, ok: bool) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(s) => {
            println!("{s}");
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("error: failed to serialise output: {e}");
            ExitCode::from(2)
        }
    }
}

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::A2lInspect {
            source,
            max_tree_depth,
        } => {
            let source = match source.into_source() {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            match ops::a2l::inspect(&A2lInspectInput {
                source,
                max_tree_depth,
            }) {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::A2lFindSignal {
            source,
            query,
            kind,
            limit,
        } => {
            let source = match source.into_source() {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let kind = match kind {
                KindArg::Measurement => SignalKind::Measurement,
                KindArg::Characteristic => SignalKind::Characteristic,
                KindArg::Any => SignalKind::Any,
            };
            match ops::a2l::find_signal(&A2lFindSignalInput {
                source,
                query,
                kind,
                limit,
            }) {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::SdfInspect {
            path,
            no_data,
            max_datasets,
        } => match ops::sdf::inspect(&SdfInspectInput {
            path,
            check_data: !no_data,
            max_datasets,
        }) {
            Ok(out) => print_json(&out, out.ok),
            Err(e) => fail(e),
        },
        Cmd::DaqPlanDryrun {
            source,
            max_dto,
            signals,
        } => {
            let source = match source.into_source() {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            match ops::daq::dryrun(&DaqPlanInput {
                source,
                max_dto,
                signals: if signals.is_empty() {
                    None
                } else {
                    Some(signals)
                },
            }) {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::MockServer { port } => {
            let mut reg = MockRegistry::default();
            match reg
                .apply(&MockServerInput {
                    action: MockAction::Start,
                    port,
                    id: None,
                })
                .await
            {
                Ok(out) => {
                    if let Ok(s) = serde_json::to_string_pretty(&out) {
                        println!("{s}");
                    }
                    eprintln!("mock server running; press Ctrl-C to stop");
                    let _ = tokio::signal::ctrl_c().await;
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        Cmd::XcpProbe {
            host,
            port,
            protocol,
            timeout_ms,
        } => {
            let protocol = match protocol {
                ProtoArg::Udp => ProbeProtocol::Udp,
                ProtoArg::Tcp => ProbeProtocol::Tcp,
            };
            match ops::probe::probe(&XcpProbeInput {
                host,
                port,
                protocol,
                timeout_ms,
            })
            .await
            {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::ProjectCheck {
            steps,
            package,
            test_filter,
            fix_fmt,
            continue_on_fail,
            timeout_s,
        } => {
            let steps: Vec<CheckStep> = if steps.is_empty() {
                vec![CheckStep::Fmt, CheckStep::Clippy, CheckStep::Test]
            } else {
                steps
                    .into_iter()
                    .map(|s| match s {
                        StepArg::Fmt => CheckStep::Fmt,
                        StepArg::Clippy => CheckStep::Clippy,
                        StepArg::Test => CheckStep::Test,
                    })
                    .collect()
            };
            match ops::check::run(&ProjectCheckInput {
                steps,
                package,
                test_filter,
                fix_fmt,
                continue_on_fail,
                timeout_s,
                repo_root: None,
            })
            .await
            {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::InvariantsCheck { checks, repo_root } => {
            let checks = if checks.is_empty() {
                None
            } else {
                Some(
                    checks
                        .into_iter()
                        .map(|c| match c {
                            InvariantArg::Spdx => InvariantName::Spdx,
                            InvariantArg::NoUnwrapInSrc => InvariantName::NoUnwrapInSrc,
                            InvariantArg::VersionConsistency => InvariantName::VersionConsistency,
                            InvariantArg::ReservedPidLimit => InvariantName::ReservedPidLimit,
                        })
                        .collect(),
                )
            };
            match ops::invariants::run(&InvariantsInput { checks, repo_root }) {
                Ok(out) => print_json(&out, out.ok),
                Err(e) => fail(e),
            }
        }
        Cmd::DumpSampleA2l { out } => {
            let text = open_xcp_client::mock_server::sample_adas_a2l::sample_adas_a2l_text();
            match out {
                Some(p) => match std::fs::write(&p, text) {
                    Ok(()) => {
                        println!("{{\"ok\": true, \"path\": {}}}", serde_json::json!(p));
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                },
                None => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
            }
        }
        Cmd::Mcp => match mcp::serve_stdio().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
    }
}
