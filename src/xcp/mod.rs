// SPDX-License-Identifier: GPL-2.0-only
//! XCP on Ethernet (UDP/TCP) protocol framing, transport, DAQ planner, target clock
//! correlation, and concurrent CTO/DTO master client for Vector `xcplite` 2.2.2.

pub mod client;
pub mod clock_sync;
pub mod daq_planner;
pub mod packet;
pub mod transport;

pub use client::{XcpClient, XcpError};
pub use clock_sync::TargetClockSync;
pub use daq_planner::{DaqEventSample, DaqListPlan, DaqOdtPlan, DaqPlanner, DecodedValue};
pub use packet::{
    encode_xcp_ethernet_frame, unpack_xcp_ethernet_frames, XcpCommand, XcpFrame, XcpPacketKind,
};
pub use transport::XcpTransport;
