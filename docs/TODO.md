# Honest Assessment & Prioritised Backlog

[← Back to Documentation Index](./index.md)

**Assessed:** 2026-10-24 at commit `167de78` (v0.2.0), branch `feature/daq-loss-detection`.
**How:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` were run (all
green, 60 integration tests, 0 unit tests in `src/`), followed by a line-by-line read of `src/`,
`tests/`, `.github/`, `docs/` and `README.md`. Every finding below carries a file anchor so it can be
re-verified; line numbers refer to the commit above.

**Severity scale**

| Sev | Meaning |
| :-: | :--- |
| 10 | Ships wrong or unusable data while claiming correctness |
| 8–9 | Loses or corrupts data, or mis-reads the target, under realistic conditions |
| 6–7 | Wrong behaviour on real hardware or a process that lets such bugs in |
| 4–5 | Correctness risk in edge cases, maintainability debt that blocks fixes |
| 1–3 | Hygiene, duplication, cosmetic |

---

## What is solid

Credit where it is due; these should be protected, not refactored away.

- **Zero `unsafe`**, two `unwrap` and two `expect` in all of `src/` (all allow-listed in
  `tools/xcp_devtool/unwrap_allowlist.txt`), one `thiserror` enum per module.
- **DAQ cycle reassembly and CTR loss detection** (`src/xcp/daq_planner.rs`, `src/xcp/link_stats.rs`)
  are careful and pinned by 21 focused tests (`tests/daq_loss_detection_test.rs`,
  `tests/daq_odt_limits_test.rs`). Partial cycles are never emitted; dropped measurements are
  reported, not truncated.
- **Plot profiles** (`src/plot_profile.rs`): atomic tmp+rename writes, `format_version` gating,
  filename sanitising, tolerant of unreadable files.
- **Config forward-compatibility**: every newer `settings.json` field is `#[serde(default)]`.
- SPDX header on every source file; `fmt` and `clippy -D warnings` clean; UDP socket is
  `connect()`ed so only the target's datagrams are accepted.

---

## Ranked backlog

Definition of fixed is one sentence; "Order" lists items that should land first.

### Severity 10

| ID | Area | Problem | Evidence | Definition of fixed | Order |
|---|---|---|---|---|---|
| **T-01** | Video | The `.mp4` output is **not H.264 and not playable**. ffmpeg is never spawned; each frame is a home-made RLE-compressed RGB blob written as its own `mdat` box after a hand-built `ftyp`/`moov` stub. `build_ffmpeg_args` is dead code, the `video_encoder` setting is stored but never used. README, docs and `Cargo.toml` described this as H.264. | `src/recording/video_recorder.rs:522-556,640,685-730`; `src/config_store.rs` `video_encoder` | Either pipe RGB frames into an `ffmpeg` child process (args already exist) and fail loudly when it is absent, or rename the output `.rgbrle` and stop calling it MP4. A test must decode one written frame with an independent reader. | Decide format first (ADR) |

### Severity 9

| ID | Area | Problem | Evidence | Definition of fixed | Order |
|---|---|---|---|---|---|
| **T-02** | Storage | The `.sdf` file is **not readable HDF5**. The superblock and root object header are present, but the `Events`/`Measurements` link addresses are `u64::MAX` (undefined) and no dataset has an object header. All data is found through a JSON index appended at the tail (`SDFIDX02`). h5py, HDFView and MATLAB will see an empty file. There is also a header-flag bug (`0x02` declares a 4-byte chunk-size field, 2 bytes are written). | `src/recording/sdf_writer.rs:603-618,644,665-670` | Either write real HDF5 (the `hdf5` crate, or emit valid v2 object headers for every group and dataset) with a CI test that opens the output in `h5py`, or document the format as "SDF container with HDF5 magic" everywhere. | With T-01 (ADR) |
| **T-03** | Storage | A whole part (default **500 MB**) is accumulated in RAM and written in one shot at split or stop. A crash, power loss or `panic = "abort"` loses the entire part; peak RAM is ~2× part size. Files are written to the final path with no tmp+rename and the directory is never fsynced. | `sdf_writer.rs:317-366,388-425`; `Cargo.toml` `[profile.release]` | Append samples to disk incrementally (or at least every N seconds), finalise via tmp+rename, fsync file and directory; a kill-test shows all but the last few seconds survive. | Before any format work is "done" |

### Severity 8

| ID | Area | Problem | Evidence | Definition of fixed | Order |
|---|---|---|---|---|---|
| **T-04** | XCP | CTO responses are **not matched to requests**. After one timeout the late reply is handed to the next command, so a `SHORT_UPLOAD` readback can return another parameter's bytes. No `SYNCH`, no retry, no response-CTR check. | `src/xcp/transport.rs:166-201` | Drain stale responses after a timeout, send `SYNCH`, verify the response belongs to the request; test with a mock that delays one reply. | — |
| **T-05** | Concurrency | Recording, 1280×720 rasterisation, per-frame video flush and the 500 MB part flush all run **on the same current-thread runtime as the socket readers**. During a flush nothing reads the socket, the kernel UDP buffer overflows, and the client records the very CTR gaps it then reports. | `src/app_state.rs:1316-1848,1714,1757`; `video_recorder.rs:553` | Move disk I/O and rendering to a dedicated thread fed by a bounded channel (or `spawn_blocking`); a test streams at mock rate through a forced split with zero gaps. | After T-03 |
| **T-06** | Reliability | Recording and video **write errors are silently dropped**; the UI keeps showing "Recording". Recorder start failure is not reported; `finish()` errors become an empty file list. | `app_state.rs:248,293,1500,1505,1573,1592,1620` | Every I/O `Result` reaches the UI as a status/event; a test injects an unwritable folder and asserts the UI state flips to error. | — |
| **T-07** | Lifecycle | The worker thread is **never joined**; `Drop` sends `Shutdown` and `main` returns while `finish()` may still be writing. | `app_state.rs:415-418,1283-1287` | Keep the `JoinHandle`, join with a deadline on exit; test that a file is complete after a simulated close during recording. | With T-03 |
| **T-08** | XCP | A configured local A2L **overrides** the target upload (code reads the local path first), while README, settings name and docs call it a fallback. A stale local file silently sends `DOWNLOAD`s to wrong addresses. | `src/xcp/client.rs:99-108` | Try the upload first and fall back only on failure (or rename the setting to `local_a2l_override_path`); test both orders. | Docs corrected 2026-10-24 to say "instead of" |

### Severity 7

| ID | Area | Problem | Evidence | Definition of fixed | Order |
|---|---|---|---|---|---|
| **T-09** | XCP | DAQ parameters are **assumed, not negotiated**: no `GET_DAQ_PROCESSOR_INFO`, `GET_DAQ_RESOLUTION_INFO` or `GET_COMM_MODE_INFO` (the builder exists, unused). DTO ID field type, 4-byte timestamp, nanosecond unit and little-endian byte order are hard-coded; `START_STOP_DAQ_LIST`'s FIRST_PID is ignored. A target with µs ticks is off by 1000×. | `client.rs:60-66,305-310`; `clock_sync.rs:115`; `packet.rs:86-97,154` | Query and honour the three info commands; reject unsupported modes with a clear error; mock gains a configurable timestamp unit. | — |
| **T-10** | XCP | **Silent link death.** The reader task exits on the first socket error (ICMP unreachable, zero-length datagram) and nobody notices; the UI stays "Connected", there is no heartbeat, `close()` only sets a flag so reader tasks and sockets leak on every reconnect. | `transport.rs:83-132,200-202` | Reader exit → `WorkerEvent::Disconnected`; periodic `GET_STATUS` heartbeat; keep and abort `JoinHandle`s on close. | — |
| **T-11** | Security | **ECU-controlled allocation.** `GET_ID` length (u32, up to 4 GB) goes straight into `Vec::with_capacity`; `MATRIX_DIM` products are unchecked and sized into `vec![0u8; …]`. A faulty target aborts the process. | `client.rs:116-134`; `a2l/model.rs:216-227`; `daq_planner.rs:326` | Cap A2L size (e.g. 64 MB), use checked multiplication, reject absurd dims; fuzz the A2L parser and frame unpacker. | — |
| **T-12** | Config | A malformed `settings.json` is **silently replaced by defaults and overwritten** on the next save; settings are written with a plain `fs::write` while profiles use tmp+rename. | `app_state.rs:411`; `config_store.rs:500` | Surface the parse error, keep the broken file as `.bak`, write atomically; test with a truncated file. | — |
| **T-13** | Process | The **95 % line-coverage gate produced coverage-farming tests**: a file exists to "guarantee ≥95 %", blocks with no assertions, `sleep`-then-assert timing (30–80 ms) that is flaky under `llvm-cov`, a test that connects to TCP port 1, and `test_ui_all_states_and_interactions_headless` asserts nothing about the UI. | `tests/comprehensive_coverage_test.rs:2-3,305-354,430-446`; `tests/ui_headless_test.rs:56-60` | Replace sleeps with poll-until-deadline, add assertions or delete dead blocks, keep the gate but add the rules in `.agents/rules/testing.md`. | Use `write-behaviour-tests` skill |
| **T-14** | A2L | Parser **silently ignores** `BYTE_ORDER`, `BIT_MASK`, `TAB_INTP`/`TAB_NOINTP`/`FORM`, `AXIS_DESCR`; data types are inferred from record-layout *names*; non-linear `RAT_FUNC` returns raw; one unmatched `/end` fails the whole file; `MODULE`s are merged. | `a2l/parser.rs:321-346,446,575`; `compu_method.rs:15-24` | Emit warnings for unsupported constructs (surface via `a2l_inspect`), implement `BIT_MASK` and `BYTE_ORDER`, add fixtures for each. | — |

### Severity 6

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-15** | Concurrency | Calibration commands are awaited inline in the DAQ drain loop (a 200 ms timeout overflows the 8192-entry DTO queue); UI↔worker channels are unbounded and grow without limit while the window is minimised; the worker busy-polls with a 2 ms sleep (~500 wake-ups/s idle). | `app_state.rs:412-413,1341,1642,1664,1843`; `transport.rs:63-64` | Bounded channels with drop counters, `tokio::select!` instead of polling, calibration on its own task. |
| **T-16** | Architecture | `AppController` is a god object (57 `pub` fields mixing UI text buffers, domain state, persistence and channels; UI reads/writes them directly); `run_background_worker` is a ~530-line function with all state in local `Option`s. Free recording functions live in the UI state module; `recording` depends on `ui::axis_ticks`. | `app_state.rs:311-395,1316-1848`; `video_recorder.rs:7` | Split into view state / session state / persistence; worker becomes an explicit state machine; move `axis_ticks` out of `ui`. |
| **T-17** | Observability | No logging facade at all (`log`/`tracing` absent). The only diagnostic is one overwritten `status_message` string. Field debugging is impossible. | whole `src/` | Add `tracing` with a file subscriber under `logfile/`; every swallowed error becomes a `warn!`. |
| **T-18** | Calibration | Submitting a value while **disconnected marks the entry Synced** and shows "Sent calibration update" although nothing was written. | `app_state.rs:1094-1098` | Refuse or queue with a visible "not connected" state; test it. |
| **T-19** | CI/Release | Windows `.exe` is shipped but **never tested**; `release.yml` runs on every push to `main` and publishes on tag without depending on CI; floating `dtolnay/rust-toolchain@stable` with `-D warnings` will break on new lints; no `rust-version`/MSRV job (`is_multiple_of` needs ≥ 1.87 while README said 1.80); `contents: write` at workflow level; actions pinned to tags, not SHAs. | `.github/workflows/*.yml`; `mock_server/mod.rs:370` | Windows test job, `needs:` on CI, `rust-version = "1.87"` + MSRV job, per-job permissions, SHA-pinned actions. `--locked` added 2026-10-24. |

### Severity 5

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-20** | XCP | `MAX_ODTS_PER_DAQ_LIST = 255` allows ODT numbers 252–254, which **collide with reserved PIDs 0xFC–0xFF** in the absolute-ODT header format; DAQ list count is not capped at 256. | `daq_planner.rs:19,503`; `packet.rs:74-85` | Limit to 252 usable ODTs, cap lists; `invariants_check reserved_pid_limit` turns from warning to error. |
| **T-21** | Clock | 32-bit timestamp wrap is detected only between consecutive samples (any event slower than ~2.1 s at ns ticks mis-times); the clock is anchored once and never re-synced; a short `GET_DAQ_CLOCK` reply is parsed from reserved bytes. | `clock_sync.rs:78-84,121-124` | Periodic re-sync, wrap handling by elapsed host time, test with a slow event. |
| **T-22** | Replay | Every part is fully loaded into memory, one `BTreeMap` per sample (many × file size); loading happens on the UI thread. | `replay/mod.rs:62-64,138-201`; `top_bar.rs:219` | Stream datasets lazily by time window; load in the worker. |
| **T-23** | Storage | All values are stored as `f64` (`u64`/`i64` above 2⁵³ lose precision); the split estimate ignores the growing JSON index so a part can exceed the configured size; reader `offset + len` is unchecked on data read from the file. | `sdf_writer.rs:239-249,356,816-834` | Native dtypes, index size in the budget, checked arithmetic in the reader. |
| **T-24** | Packaging | The mock server and sample A2L are compiled into the **release binary** (no feature gate); the mock is happy-path only (ignores `MAX_CTO`, no error injection, no byte-order mode, memory map grows unbounded). | `lib.rs:9`; `mock_server/mod.rs:44,191-206` | `mock` cargo feature on by default for dev/test, off for release; add error/loss injection knobs. |
| **T-25** | API | `lib.rs` exports all 10 modules `pub` (359 `pub` items, incl. `cto::*`, controller fields); `AppController` methods return `Result<_, String>` discarding typed errors; 10 `#[allow(too_many_arguments)]`. | `lib.rs`; `app_state.rs` | `pub(crate)` by default, re-export a curated surface, typed `AppError`. |

### Severity 4

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-26** | Hygiene | No `cargo-audit`/`cargo-deny`/dependabot; duplicate `thiserror` 1 and 2 in the lock; `tokio` `rt-multi-thread` and `macros` enabled but unused in the lib; `egui` 0.29 is behind. `*.mp4` was not git-ignored (**fixed 2026-10-24**). | `Cargo.toml`, `Cargo.lock` | Add `deny.toml` + CI job, bump `thiserror`, trim tokio features, plan the egui upgrade. |
| **T-27** | Docs | README/docs claimed H.264 video, standards HDF5, "fallback" A2L order, "no runtime dependencies", a headless self-check that parses A2L and records (it renders one frame), Rust 1.80, and listed 6 of 11 test files. **Corrected 2026-10-24** in `README.md`, `docs/*.md`, `Cargo.toml`, `src/lib.rs`. Keep it that way via `.agents/rules/docs-honesty.md`. | `src/main.rs:14-24` | Done; re-verify with `grep -rn "H.264\|HDF5" README.md docs/`. |

### Severity 3

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-28** | Hygiene | No `CONTRIBUTING.md`, `CHANGELOG.md`, `SECURITY.md`; no `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`; `build.rs` is a no-op; dead code (`cto::get_status` was unused until `xcp_devtool`, `ActiveTab::Logging`, `SizeSplitter::from_max_mb`, `build_ffmpeg_args`). | repo root | Add the files, delete `build.rs` and dead code. |
| **T-29** | Code | The "consume `MATRIX_DIM` numbers" loop is copy-pasted 6× in the parser; magic numbers (`256`/`8192` queue sizes, `128` ODT chunk, `max_dto - 6`, `2 ms`, `32_768`, trail `400`). | `a2l/parser.rs:386,469,528,594,653,711`; `transport.rs:63-64`; `daq_planner.rs:272` | Extract helpers and named constants with a one-line reason each. |

---

## Recommended fix order

1. **Decide the two formats (ADR):** real H.264 via ffmpeg or an honestly named raw stream (T-01); real HDF5 or an honestly named container (T-02). Everything downstream depends on this.
2. **Make recordings durable:** incremental writes, tmp+rename, fsync, join the worker on exit (T-03, T-07).
3. **Make the command channel trustworthy:** request/response matching, SYNCH, retry (T-04).
4. **Negotiate DAQ parameters** and honour byte order and timestamp units (T-09, T-20, T-21).
5. **Move recording and rendering off the socket runtime** (T-05, T-15).
6. **Surface every error** to the UI and to a log (T-06, T-10, T-12, T-17, T-18).
7. **Split `AppController` and the worker** into a view model, a session state machine and persistence (T-16, T-25).
8. **Harden untrusted input** with caps and fuzzing (T-11, T-14, T-23).

## Process debt

- Keep the 95 % gate but enforce the testing rules (`.agents/rules/testing.md`); rewrite
  `comprehensive_coverage_test.rs` into behaviour tests (T-13).
- Windows CI job, `needs:` between release and CI, MSRV job, SHA-pinned actions, per-job permissions (T-19).
- Run `cargo run -q -p xcp_devtool -- invariants-check` and `daq-plan-dryrun` in every change
  (already wired into `ci.yml`).

## How to use this file

Pick the highest open item whose "Order" prerequisites are done. When an item is fixed, move it to a
**Closed** section at the bottom with the commit hash rather than deleting it. New findings get the
next `T-nn` and a severity using the scale above. `AGENTS.md` describes how agents must reference this
file in every change.
