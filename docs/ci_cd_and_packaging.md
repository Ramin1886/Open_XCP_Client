# CI/CD Pipelines, Release Packaging & Test Coverage

[← Back to Documentation Index](./index.md)

## GitHub Actions Workflows

Compiled binaries are generated automatically via GitHub Actions rather than committed to the git repository:

### 1. Continuous Integration & Coverage Gate ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml))
Triggered on every push and pull request:
- Verifies code formatting (`cargo fmt --all -- --check`).
- Enforces zero compiler/Clippy warnings (`cargo clippy --all-targets -- -D warnings`).
- Runs the complete unit, headless UI, and end-to-end integration test suite under `cargo-llvm-cov` and enforces **$\ge 95\%$ line coverage**:
  ```bash
  cargo llvm-cov --all-targets --workspace --lcov --output-path lcov.info --fail-under-lines 95
  ```

### 2. Cross-Platform Binary Release Pipeline ([`.github/workflows/release.yml`](../.github/workflows/release.yml))
Triggered on pushes to `main`, version tags (`v*`), or manual dispatch:
- **`build-linux`**: Compiles a stripped, LTO-optimized Linux x86_64 binary (`open_xcp_client`) and uploads it as a workflow artifact.
- **`build-windows`**: Compiles a self-contained, stripped, LTO-optimized Windows x86_64 executable (`open_xcp_client.exe`) and uploads it as a workflow artifact.
- **`publish-release`**: Automatically attaches both `open_xcp_client` and `open_xcp_client.exe` to a GitHub Release when a `v*` tag is pushed.

---

## Running Tests Locally

Execute the full test suite:
```bash
cargo test
```

Test suite structure in [`./tests/`](../tests/):
- [`tests/a2l_hierarchy_test.rs`](../tests/a2l_hierarchy_test.rs) — Tests A2L lexer, parser, nested `TYPEDEF_STRUCTURE` / `INSTANCE` expansion, dot-separated paths, `MATRIX_DIM` arrays, and `COMPU_METHOD` conversions.
- [`tests/xcp_protocol_and_clock_test.rs`](../tests/xcp_protocol_and_clock_test.rs) — Tests XCP-on-Ethernet framing, CTO commands, DTO unpacking, and `TargetClockSync` UTC correlation & 32-bit rollover.
- [`tests/sdf_split_and_hierarchy_test.rs`](../tests/sdf_split_and_hierarchy_test.rs) — Tests HDF5 v2 `.sdf` file creation, Jenkins `lookup3` checksum verification, nested group paths, UTC time-series datasets, and automatic part splitting (`PART001.sdf`, `PART002.sdf`).
- [`tests/calibration_and_config_test.rs`](../tests/calibration_and_config_test.rs) — Tests `./config/settings.json` persistence (including `max_logfile_size_mb`), strict data-type and range validation, and `CalibrationManager`.
- [`tests/ui_headless_test.rs`](../tests/ui_headless_test.rs) — Runs headless `egui::Context` frames through the Top Bar, `⚙ Settings` modal, Logging Tab, and Calibration Tab.
- [`tests/end_to_end_xcp_test.rs`](../tests/end_to_end_xcp_test.rs) — Spawns `MockXcpServer` over both UDP and TCP and verifies dynamic A2L upload, multi-rate DAQ `.sdf` recording, and concurrent live calibration with readback verification.
