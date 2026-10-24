// SPDX-License-Identifier: GPL-2.0-only
//! `mock_server`: start, stop and list in-process mock `xcplite` servers.

use crate::error::{DevtoolError, DevtoolResult};
use open_xcp_client::mock_server::MockXcpServer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

/// Action of `mock_server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MockAction {
    Start,
    Stop,
    List,
}

/// Input of `mock_server`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MockServerInput {
    pub action: MockAction,
    /// Port to bind on 127.0.0.1 for `start`; 0 picks an ephemeral port (default 0).
    #[serde(default)]
    pub port: u16,
    /// Server id for `stop`; omitted stops all.
    #[serde(default)]
    pub id: Option<String>,
}

/// One running mock server.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MockServerInfo {
    pub id: String,
    pub udp_addr: String,
    pub tcp_addr: String,
    pub uptime_s: u64,
}

/// Output of `mock_server`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MockServerOutput {
    pub ok: bool,
    pub action: MockAction,
    pub servers: Vec<MockServerInfo>,
    pub problems: Vec<String>,
}

/// Holds live mock servers for the lifetime of the MCP process.
#[derive(Default)]
pub struct MockRegistry {
    servers: BTreeMap<String, (MockXcpServer, Instant)>,
    next_id: u32,
}

impl MockRegistry {
    fn infos(&self) -> Vec<MockServerInfo> {
        self.servers
            .iter()
            .map(|(id, (s, t))| MockServerInfo {
                id: id.clone(),
                udp_addr: s.udp_addr.to_string(),
                tcp_addr: s.tcp_addr.to_string(),
                uptime_s: t.elapsed().as_secs(),
            })
            .collect()
    }

    /// Applies `input` to the registry.
    pub async fn apply(&mut self, input: &MockServerInput) -> DevtoolResult<MockServerOutput> {
        match input.action {
            MockAction::Start => {
                let server = MockXcpServer::start(input.port).await?;
                self.next_id += 1;
                let id = format!("ms-{}", self.next_id);
                self.servers.insert(id, (server, Instant::now()));
            }
            MockAction::Stop => match &input.id {
                Some(id) => {
                    self.servers
                        .remove(id)
                        .ok_or_else(|| DevtoolError::Invalid(format!("no mock server '{id}'")))?;
                }
                None => self.servers.clear(),
            },
            MockAction::List => {}
        }
        Ok(MockServerOutput {
            ok: true,
            action: input.action,
            servers: self.infos(),
            problems: Vec::new(),
        })
    }
}
