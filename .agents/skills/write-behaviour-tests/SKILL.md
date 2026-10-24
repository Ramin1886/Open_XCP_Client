---
name: write-behaviour-tests
description: Raise or protect test coverage in open_xcp_client with behaviour tests, not line-touching. Use when adding tests, when the 95 percent coverage gate fails, or when rewriting a weak test.
---
# write-behaviour-tests

1. Find what is uncovered: `cargo llvm-cov --all-targets -p open_xcp_client --ignore-filename-regex 'src/main\.rs' --lcov --output-path /tmp/lcov.info`, then list `DA:<line>,0` entries per `SF:` file (a short `python3 -I` script is fine).
2. For each uncovered region ask "which input reaches this?" The answer is the test: malformed frame, short `.sdf`, unmatched `/end`, closed socket, unwritable folder, duplicate CTR.
3. Put the test in the matching `tests/*_test.rs` file (`src/` has no unit tests by convention). Name it after the behaviour: `odt_zero_loss_discards_cycle_without_touching_staging`, not `test_planner_2`.
4. Assert an outcome: a value, an error variant, a counter, file content, a UI state field. Never only `is_ok()` or `!is_empty()` unless that *is* the behaviour.
5. Async: poll with a deadline (`Instant::now() + Duration::from_secs(5)` loop with `tokio::time::sleep(10ms)` inside), never a single `sleep` then assert.
6. Network: `MockXcpServer::start(0)`; read `udp_addr` / `tcp_addr` from it. Never fixed ports, never "expect refused" ports.
7. Files: `tempfile::tempdir()`; never write under `config/` or `logfile/`.
8. Forbidden ways to pass the gate: deleting assertions, `#[ignore]`, adding `--ignore-filename-regex` entries, moving code into `main.rs`, `#[cfg(not(coverage))]`.
9. One concern per test; a 400-line omnibus test is split, not extended.
10. Run the `verify-change` skill.
