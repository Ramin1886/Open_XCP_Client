# CI/CD Pipelines, Release Packaging & Test Coverage

[← Back to Documentation Index](./index.md)

## GitHub Actions Workflows

Compiled binaries are generated automatically via GitHub Actions rather than committed to the git repository:

### 1. Continuous Integration & Coverage Gate ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml))
Triggered on pushes and pull requests to `main`, `master` and `develop`, plus manual dispatch:
- Verifies code formatting (`cargo fmt --all -- --check`).
- Enforces zero compiler/Clippy warnings across the workspace (`cargo clippy --workspace --all-targets --locked -- -D warnings`).
- Runs the product crate's integration test suite under `cargo-llvm-cov` and enforces **$\ge 95\%$ line coverage** (the `xcp_devtool` crate is tested separately and excluded from the gate):
  ```bash
  cargo llvm-cov --all-targets -p open_xcp_client --locked --ignore-filename-regex 'src/main\.rs' --lcov --output-path lcov.info --fail-under-lines 95
  cargo test -p xcp_devtool --locked
  cargo run -q -p xcp_devtool --locked -- invariants-check
  ```
- The coverage number is a floor, not a goal: tests must assert behaviour (see `.agents/rules/testing.md`). Known coverage-driven tests are tracked as `TODO.md` T-13.

### 2. Cross-Platform Binary Release Pipeline ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
Triggered on pushes to `main`, version tags (`v*`), or manual dispatch:
- **`build-linux`**: Compiles a stripped, LTO-optimized Linux x86_64 binary (`open_xcp_client`) with `--locked` and uploads it as a workflow artifact. The release workflow does not wait for the CI workflow and the Windows build is not tested (`TODO.md` T-19).
- **`build-windows`**: Compiles a self-contained, stripped, LTO-optimized Windows x86_64 executable (`open_xcp_client.exe`) and uploads it as a workflow artifact.
- **`publish-release`**: Automatically attaches both `open_xcp_client` and `open_xcp_client.exe` to a GitHub Release when a `v*` tag is pushed.

---

## Running Tests Locally

Execute the full test suite:
```bash
cargo test --workspace
```

Test suite structure in [`./tests/`](../tests/):
- [`tests/a2l_hierarchy_test.rs`](../tests/a2l_hierarchy_test.rs) — Tests A2L lexer, parser, nested `TYPEDEF_STRUCTURE` / `INSTANCE` expansion, dot-separated paths, `MATRIX_DIM` arrays, and `COMPU_METHOD` conversions.
- [`tests/xcp_protocol_and_clock_test.rs`](../tests/xcp_protocol_and_clock_test.rs) — Tests XCP-on-Ethernet framing, CTO commands, DTO unpacking, and `TargetClockSync` UTC correlation & 32-bit rollover.
- [`tests/sdf_split_and_hierarchy_test.rs`](../tests/sdf_split_and_hierarchy_test.rs) — Tests `.sdf` container creation, Jenkins `lookup3` checksum verification, nested dataset paths, UTC time-series datasets, and automatic part splitting (`PART001.sdf`, `PART002.sdf`).
- [`tests/calibration_and_config_test.rs`](../tests/calibration_and_config_test.rs) — Tests `./config/settings.json` persistence (including `max_logfile_size_mb`), strict data-type and range validation, and `CalibrationManager`.
- [`tests/ui_headless_test.rs`](../tests/ui_headless_test.rs) — Renders three headless `egui::Context` frames through all tabs as a smoke test; it asserts controller calls, not rendered UI.
- [`tests/end_to_end_xcp_test.rs`](../tests/end_to_end_xcp_test.rs) — Spawns `MockXcpServer` over both UDP and TCP and verifies dynamic A2L upload, multi-rate DAQ `.sdf` recording, and live calibration with readback verification.
- [`tests/daq_odt_limits_test.rs`](../tests/daq_odt_limits_test.rs) — Pins the 255-entry / 255-ODT `u8` limits, the ODT-0 timestamp reserve, and drop-not-truncate behaviour.
- [`tests/daq_loss_detection_test.rs`](../tests/daq_loss_detection_test.rs) — CTR gap / reorder / duplicate accounting, cycle discard reasons, and integrity metadata written to `.sdf`.
- [`tests/plot_profile_test.rs`](../tests/plot_profile_test.rs) — Profile round-trip, sanitising, `format_version` gating, import/export and unresolved-signal reporting.
- [`tests/visualization_video_replay_test.rs`](../tests/visualization_video_replay_test.rs) — Axis ticks, XY pairing, rasteriser, `.mp4` frame-stream writer, replay player and worker split.
- [`tests/comprehensive_coverage_test.rs`](../tests/comprehensive_coverage_test.rs) — Error-branch coverage for config, client, validator and worker; scheduled for rewrite into behaviour tests (`TODO.md` T-13).
- [`tools/xcp_devtool/tests/`](../tools/xcp_devtool/tests/) — CLI JSON contract and MCP stdio handshake smoke tests for the devtool.
