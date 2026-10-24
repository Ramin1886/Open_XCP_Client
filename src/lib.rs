// SPDX-License-Identifier: GPL-2.0-only
//! `open_xcp_client` — Lightweight XCP on Ethernet (UDP/TCP) measurement logging (`.sdf`
//! container) and live calibration client for Vector `xcplite` 2.2.2 targets.
//!
//! Known limitations and the prioritised backlog are tracked in `docs/TODO.md`.

pub mod a2l;
pub mod app_state;
pub mod calibration;
pub mod config_store;
pub mod mock_server;
pub mod plot_profile;
pub mod recording;
pub mod replay;
pub mod ui;
pub mod xcp;
