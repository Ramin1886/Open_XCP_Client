// SPDX-License-Identifier: GPL-2.0-only
//! Error type shared by the CLI and the MCP server.

use open_xcp_client::a2l::A2lParseError;
use open_xcp_client::recording::SdfError;
use open_xcp_client::xcp::XcpError;
use thiserror::Error;

/// Errors produced by devtool operations.
#[derive(Debug, Error)]
pub enum DevtoolError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("A2L parse error: {0}")]
    A2l(#[from] A2lParseError),
    #[error("SDF error: {0}")]
    Sdf(#[from] SdfError),
    #[error("XCP error: {0}")]
    Xcp(#[from] XcpError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("subprocess error: {0}")]
    Process(String),
    #[error("timed out: {0}")]
    Timeout(String),
}

impl From<DevtoolError> for rmcp::ErrorData {
    fn from(err: DevtoolError) -> Self {
        match err {
            DevtoolError::Invalid(msg) => rmcp::ErrorData::invalid_params(msg, None),
            other => rmcp::ErrorData::internal_error(other.to_string(), None),
        }
    }
}

/// Convenience alias used throughout the crate.
pub type DevtoolResult<T> = Result<T, DevtoolError>;
