// SPDX-License-Identifier: GPL-2.0-only
//! `open_xcp_client` — Lightweight, high-performance XCP on Ethernet (UDP/TCP)
//! logging (`.sdf`/HDF5) and live calibration client for Vector `xcplite` 2.2.2 targets.

pub mod a2l;
pub mod app_state;
pub mod calibration;
pub mod config_store;
pub mod mock_server;
pub mod recording;
pub mod ui;
pub mod xcp;
