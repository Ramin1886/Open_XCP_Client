// SPDX-License-Identifier: GPL-2.0-only
//! `xcp_probe`: CONNECT, GET_STATUS, GET_COMM_MODE_INFO, DISCONNECT against a target.
//! No DAQ is allocated and no memory is written.

use crate::error::{DevtoolError, DevtoolResult};
use open_xcp_client::config_store::TransportProtocol;
use open_xcp_client::xcp::packet::cto;
use open_xcp_client::xcp::XcpTransport;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// Transport selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeProtocol {
    #[default]
    Udp,
    Tcp,
}

/// Input of `xcp_probe`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct XcpProbeInput {
    /// Target IP address (default 127.0.0.1).
    #[serde(default = "default_host")]
    pub host: String,
    /// Target XCP port (default 5555).
    #[serde(default = "default_port")]
    pub port: u16,
    /// `udp` or `tcp` (default udp).
    #[serde(default)]
    pub protocol: ProbeProtocol,
    /// Per-command timeout in milliseconds (default 1000).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    5555
}
fn default_timeout_ms() -> u64 {
    1000
}

/// Decoded CONNECT response.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ConnectInfo {
    pub resource: u8,
    pub comm_mode_basic: u8,
    pub byte_order_motorola: bool,
    pub address_granularity: u8,
    pub slave_block_mode: bool,
    pub max_cto: u8,
    pub max_dto: u16,
    pub protocol_layer_version: u8,
    pub transport_layer_version: u8,
}

/// Decoded GET_STATUS response.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatusInfo {
    pub session_status: u8,
    pub daq_running: bool,
    pub resume_mode: bool,
    pub resource_protection_status: u8,
    pub cal_pag_protected: bool,
    pub daq_protected: bool,
    pub session_configuration_id: u16,
}

/// Decoded GET_COMM_MODE_INFO response.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommModeInfo {
    pub comm_mode_optional: u8,
    pub max_bs: u8,
    pub min_st: u8,
    pub queue_size: u8,
    pub driver_version: u8,
}

/// Output of `xcp_probe`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct XcpProbeOutput {
    pub ok: bool,
    pub target: String,
    pub protocol: ProbeProtocol,
    pub connect: Option<ConnectInfo>,
    pub status: Option<StatusInfo>,
    pub comm_mode_info: Option<CommModeInfo>,
    pub connect_round_trip_ms: f64,
    pub problems: Vec<String>,
}

/// Runs `xcp_probe`.
pub async fn probe(input: &XcpProbeInput) -> DevtoolResult<XcpProbeOutput> {
    let ip: IpAddr = input
        .host
        .parse()
        .map_err(|_| DevtoolError::Invalid(format!("invalid IP address '{}'", input.host)))?;
    let addr = SocketAddr::new(ip, input.port);
    let timeout = Duration::from_millis(input.timeout_ms.max(50));
    let protocol = match input.protocol {
        ProbeProtocol::Udp => TransportProtocol::Udp,
        ProbeProtocol::Tcp => TransportProtocol::Tcp,
    };
    let mut problems = Vec::new();
    let (transport, _daq_rx) = match XcpTransport::connect(addr, protocol).await {
        Ok(t) => t,
        Err(e) => {
            return Ok(XcpProbeOutput {
                ok: false,
                target: addr.to_string(),
                protocol: input.protocol,
                connect: None,
                status: None,
                comm_mode_info: None,
                connect_round_trip_ms: 0.0,
                problems: vec![format!("socket connect failed: {e}")],
            });
        }
    };

    let t0 = Instant::now();
    let res = match transport.send_command(&cto::connect(0x00), timeout).await {
        Ok(r) => r,
        Err(e) => {
            transport.close();
            return Ok(XcpProbeOutput {
                ok: false,
                target: addr.to_string(),
                protocol: input.protocol,
                connect: None,
                status: None,
                comm_mode_info: None,
                connect_round_trip_ms: t0.elapsed().as_secs_f64() * 1000.0,
                problems: vec![format!("CONNECT failed: {e}")],
            });
        }
    };
    let rtt = t0.elapsed().as_secs_f64() * 1000.0;
    if res.len() < 7 {
        problems.push(format!(
            "CONNECT response is {} bytes, expected 7; fields may be defaults",
            res.len()
        ));
    }
    let b = |i: usize| res.get(i).copied().unwrap_or(0);
    let connect = ConnectInfo {
        resource: b(0),
        comm_mode_basic: b(1),
        byte_order_motorola: b(1) & 0x01 != 0,
        address_granularity: (b(1) >> 1) & 0x03,
        slave_block_mode: b(1) & 0x40 != 0,
        max_cto: b(2),
        max_dto: u16::from_le_bytes([b(3), b(4)]),
        protocol_layer_version: b(5),
        transport_layer_version: b(6),
    };

    let status = match transport.send_command(&cto::get_status(), timeout).await {
        Ok(s) => {
            let g = |i: usize| s.get(i).copied().unwrap_or(0);
            Some(StatusInfo {
                session_status: g(0),
                daq_running: g(0) & 0x40 != 0,
                resume_mode: g(0) & 0x80 != 0,
                resource_protection_status: g(1),
                cal_pag_protected: g(1) & 0x01 != 0,
                daq_protected: g(1) & 0x04 != 0,
                session_configuration_id: u16::from_le_bytes([g(3), g(4)]),
            })
        }
        Err(e) => {
            problems.push(format!("GET_STATUS failed: {e}"));
            None
        }
    };

    let comm_mode_info = match transport
        .send_command(&cto::get_comm_mode_info(), timeout)
        .await
    {
        Ok(c) => {
            let g = |i: usize| c.get(i).copied().unwrap_or(0);
            Some(CommModeInfo {
                comm_mode_optional: g(1),
                max_bs: g(3),
                min_st: g(4),
                queue_size: g(5),
                driver_version: g(6),
            })
        }
        Err(e) => {
            problems.push(format!("GET_COMM_MODE_INFO not available: {e}"));
            None
        }
    };

    if let Err(e) = transport.send_command(&cto::disconnect(), timeout).await {
        problems.push(format!("DISCONNECT failed: {e}"));
    }
    transport.close();

    Ok(XcpProbeOutput {
        ok: status.is_some(),
        target: addr.to_string(),
        protocol: input.protocol,
        connect: Some(connect),
        status,
        comm_mode_info,
        connect_round_trip_ms: rtt,
        problems,
    })
}
