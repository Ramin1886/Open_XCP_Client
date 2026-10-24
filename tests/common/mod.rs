// SPDX-License-Identifier: GPL-2.0-only
//! Shared helpers for integration tests. Cargo compiles this file into every test binary that
//! declares `mod common;`, so each binary only uses a subset of what is here.
#![allow(dead_code)] // each test binary uses only some of these helpers

use open_xcp_client::app_state::AppController;
use std::time::{Duration, Instant};

/// Interval between two polls of the worker event queue.
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Polls worker events until `pred` holds or `timeout` elapses. Returns whether it held, so call
/// sites can say why they waited: `assert!(common::wait_until(..), "mock must connect")`.
pub fn wait_until(
    ctrl: &mut AppController,
    timeout: Duration,
    pred: impl Fn(&AppController) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        ctrl.poll_worker_events();
        if pred(ctrl) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Async twin of [`wait_until`] for `#[tokio::test]` bodies; sleeps on the runtime so the mock
/// server and the worker are not starved while we wait.
pub async fn wait_until_async(
    ctrl: &mut AppController,
    timeout: Duration,
    pred: impl Fn(&AppController) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        ctrl.poll_worker_events();
        if pred(ctrl) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
