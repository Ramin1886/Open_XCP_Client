// SPDX-License-Identifier: GPL-2.0-only
//! MCP stdio server exposing the devtool operations.

use crate::ops::a2l::{A2lFindSignalInput, A2lFindSignalOutput, A2lInspectInput, A2lInspectOutput};
use crate::ops::check::{ProjectCheckInput, ProjectCheckOutput};
use crate::ops::daq::{DaqPlanInput, DaqPlanOutput};
use crate::ops::invariants::{InvariantsInput, InvariantsOutput};
use crate::ops::mock::{MockRegistry, MockServerInput, MockServerOutput};
use crate::ops::probe::{XcpProbeInput, XcpProbeOutput};
use crate::ops::sdf::{SdfInspectInput, SdfInspectOutput};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ErrorData, Json, ServerHandler, ServiceExt};
use std::sync::Arc;
use tokio::sync::Mutex;

/// The devtool MCP server.
#[derive(Clone)]
pub struct DevtoolServer {
    tool_router: ToolRouter<Self>,
    mocks: Arc<Mutex<MockRegistry>>,
}

impl Default for DevtoolServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl DevtoolServer {
    /// Creates a server with an empty mock-server registry.
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            mocks: Arc::new(Mutex::new(MockRegistry::default())),
        }
    }

    #[tool(
        description = "Parse an A2L file (path or builtin adas_sample) and summarise events, block counts, the structure tree and any /begin blocks the parser does not handle. Read-only."
    )]
    async fn a2l_inspect(
        &self,
        Parameters(input): Parameters<A2lInspectInput>,
    ) -> Result<Json<A2lInspectOutput>, ErrorData> {
        Ok(Json(crate::ops::a2l::inspect(&input)?))
    }

    #[tool(
        description = "Find measurements/characteristics in an A2L by case-insensitive substring or * glob, with address, type, dims, event and unit. Read-only."
    )]
    async fn a2l_find_signal(
        &self,
        Parameters(input): Parameters<A2lFindSignalInput>,
    ) -> Result<Json<A2lFindSignalOutput>, ErrorData> {
        Ok(Json(crate::ops::a2l::find_signal(&input)?))
    }

    #[tool(
        description = "Open one .sdf recording part with the library reader and run structural checks (checksums, SDFIDX02 tail, dataset ranges, shapes, time datasets, monotonic time, PART number). Returns integrity counters and gap records. Read-only."
    )]
    async fn sdf_inspect(
        &self,
        Parameters(input): Parameters<SdfInspectInput>,
    ) -> Result<Json<SdfInspectOutput>, ErrorData> {
        Ok(Json(crate::ops::sdf::inspect(&input)?))
    }

    #[tool(
        description = "Run the DAQ/ODT planner on an A2L for a given MAX_DTO (budget = max_dto - 6, as the app does) and report per-list ODT/entry counts, dropped measurements and any limit violations. Read-only."
    )]
    async fn daq_plan_dryrun(
        &self,
        Parameters(input): Parameters<DaqPlanInput>,
    ) -> Result<Json<DaqPlanOutput>, ErrorData> {
        Ok(Json(crate::ops::daq::dryrun(&input)?))
    }

    #[tool(
        description = "Start, stop or list in-process mock xcplite servers on 127.0.0.1 (UDP and TCP on the same port; port 0 = ephemeral). SIDE EFFECT: binds ports; servers live until stopped or the MCP process exits."
    )]
    async fn mock_server(
        &self,
        Parameters(input): Parameters<MockServerInput>,
    ) -> Result<Json<MockServerOutput>, ErrorData> {
        let mut reg = self.mocks.lock().await;
        Ok(Json(reg.apply(&input).await?))
    }

    #[tool(
        description = "Probe an XCP target: CONNECT, GET_STATUS, GET_COMM_MODE_INFO, DISCONNECT. No DAQ, no memory writes. SIDE EFFECT: opens and closes an XCP session on the target; only use against hardware when the user asked for it."
    )]
    async fn xcp_probe(
        &self,
        Parameters(input): Parameters<XcpProbeInput>,
    ) -> Result<Json<XcpProbeOutput>, ErrorData> {
        Ok(Json(crate::ops::probe::probe(&input).await?))
    }

    #[tool(
        description = "Run cargo fmt --check, clippy -D warnings and tests for one package and return structured pass/fail with the first errors. Writes files only when fix_fmt is true."
    )]
    async fn project_check(
        &self,
        Parameters(input): Parameters<ProjectCheckInput>,
    ) -> Result<Json<ProjectCheckOutput>, ErrorData> {
        Ok(Json(crate::ops::check::run(&input).await?))
    }

    #[tool(
        description = "Check repository invariants without compiling: SPDX headers, no unwrap/expect in src/ outside the allowlist, Cargo.toml vs Cargo.lock version, reserved-PID ODT limit (advisory). Read-only."
    )]
    async fn invariants_check(
        &self,
        Parameters(input): Parameters<InvariantsInput>,
    ) -> Result<Json<InvariantsOutput>, ErrorData> {
        Ok(Json(crate::ops::invariants::run(&input)?))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DevtoolServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("xcp_devtool", env!("CARGO_PKG_VERSION")))
            .with_instructions(
            "Developer tools for open_xcp_client. Prefer a2l_inspect / a2l_find_signal over grepping A2L text, sdf_inspect to validate recordings, daq_plan_dryrun after touching src/xcp/daq_planner.rs, and invariants_check + project_check before declaring a change done. mock_server and xcp_probe have side effects (ports, target sessions).",
        )
    }
}

/// Serves the devtool over stdio until the client disconnects.
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let service = DevtoolServer::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
