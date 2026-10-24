---
trigger: glob
description: How tests are written and what the 95 percent coverage gate does and does not allow
globs: "tests/**,src/**/*.rs,tools/**/*.rs"
paths:
  - "tests/**"
  - "src/**/*.rs"
  - "tools/**/*.rs"
---
# Testing

- Tests live in `tests/*_test.rs` (integration style). There are no `#[cfg(test)]` modules in `src/`; keep it that way.
- A test is named after the behaviour it proves and asserts an observable outcome: a value, an error variant, a file's content, a state change. Calling a function and asserting nothing is not a test.
- Async outcomes are awaited with `common::wait_until` / `common::wait_until_async` from `tests/common/mod.rs` (poll-until-deadline), never `sleep(ms)` then assert. Add `mod common;` to the test file.
- Network tests use `MockXcpServer::start(0)` (ephemeral ports). Never bind a fixed port, never connect to a port you expect to be closed.
- File tests use `tempfile`; never write into `config/` or `logfile/`.
- Error branches are covered by feeding malformed input (truncated frames, bad A2L, short `.sdf`), not by `#[should_panic]`.
- Every behaviour change ships its test in the same commit.
- The 95 % line-coverage gate on `open_xcp_client` is a floor, not a goal. Never add `--ignore-filename-regex` entries, delete an assertion, or weaken a test to pass it.
- A failing test is fixed by fixing the code or by proving, in the commit message, that the test was wrong.
- Changes under `src/xcp/` run `tests/daq_odt_limits_test.rs`, `tests/daq_loss_detection_test.rs` and `cargo run -q -p xcp_devtool -- daq-plan-dryrun --builtin adas_sample --max-dto {8,32,256,1024}`.
