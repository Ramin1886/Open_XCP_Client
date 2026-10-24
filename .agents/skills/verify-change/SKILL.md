---
name: verify-change
description: Prove a change to open_xcp_client is mergeable - fmt, clippy, the right tests, coverage, invariants and planner/format checks - and report a pass/fail table. Use before declaring any change done or asking for review.
---
# verify-change

Run everything from the repository root. Stop and fix at the first failure; re-run from step 1.

1. `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`
   (or `cargo run -q -p xcp_devtool -- project-check --step fmt --step clippy`).
2. Pick tests by touched area, then run them:
   - `src/xcp/**` → `cargo test -p open_xcp_client --test daq_odt_limits_test --test daq_loss_detection_test --test xcp_protocol_and_clock_test --test end_to_end_xcp_test`
   - `src/recording/**`, `src/replay/**` → `--test sdf_split_and_hierarchy_test --test visualization_video_replay_test`
   - `src/a2l/**` → `--test a2l_hierarchy_test`
   - `src/config_store.rs`, `src/plot_profile.rs`, `src/calibration/**` → `--test calibration_and_config_test --test plot_profile_test`
   - `tools/xcp_devtool/**` → `cargo test -p xcp_devtool`
   - anything else, or more than one area → `cargo test --workspace`
3. If `cargo llvm-cov` is on PATH:
   `cargo llvm-cov --all-targets -p open_xcp_client --ignore-filename-regex 'src/main\.rs' --summary-only`
   and report the line percentage against the 95 % gate.
4. `cargo run -q -p xcp_devtool -- invariants-check` must be `ok: true`.
5. If `src/xcp/daq_planner.rs` changed: for `--max-dto` in 8, 32, 256, 1024 run
   `cargo run -q -p xcp_devtool -- daq-plan-dryrun --builtin adas_sample --max-dto <n>`; `violations` must be `[]`.
6. If `src/recording/**` changed: record a short session (the e2e test or `SdfSessionRecorder` in a scratch test) and run
   `cargo run -q -p xcp_devtool -- sdf-inspect <part.sdf>`; every check must pass.
7. Confirm docs: every changed behaviour has its `README.md` / `docs/*.md` sentence updated and a `docs/TODO.md` item closed or added.
8. Report a table: step, command, pass/fail, one-line detail. If any row fails, the change is not done.
