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

### Severity 7

| ID | Area | Problem | Evidence | Definition of fixed | Order |
|---|---|---|---|---|---|
| **T-09** | XCP | DAQ parameters are **assumed, not negotiated**: no `GET_DAQ_PROCESSOR_INFO`, `GET_DAQ_RESOLUTION_INFO` or `GET_COMM_MODE_INFO` (the builder exists, unused). DTO ID field type, 4-byte timestamp, nanosecond unit and little-endian byte order are hard-coded; `START_STOP_DAQ_LIST`'s FIRST_PID is ignored. A target with µs ticks is off by 1000×. | `client.rs:60-66,305-310`; `clock_sync.rs:115`; `packet.rs:86-97,154` | Query and honour the three info commands; reject unsupported modes with a clear error; mock gains a configurable timestamp unit. | — |
| **T-10** | XCP | **Silent link death.** The reader task exits on the first socket error (ICMP unreachable, zero-length datagram) and nobody notices; the UI stays "Connected", there is no heartbeat, `close()` only sets a flag so reader tasks and sockets leak on every reconnect. | `transport.rs:83-132,200-202` | Reader exit → `WorkerEvent::Disconnected`; periodic `GET_STATUS` heartbeat; keep and abort `JoinHandle`s on close. | — |
| **T-11** | Security | *(caps landed in `f3f6f40`: `MAX_UPLOAD_BYTES`, clamped `MATRIX_DIM`, saturating `element_count`)* Remaining: no fuzzing of `unpack_xcp_ethernet_frames`, the A2L parser or the `.sdf` reader. | `src/xcp/packet.rs`, `src/a2l/parser.rs`, `src/recording/sdf_writer.rs` | A `cargo fuzz` target per parser running in CI for a fixed budget. | — |
| **T-14** | A2L | Parser **silently ignores** `BYTE_ORDER`, `BIT_MASK`, `TAB_INTP`/`TAB_NOINTP`/`FORM`, `AXIS_DESCR`; data types are inferred from record-layout *names*; non-linear `RAT_FUNC` returns raw; one unmatched `/end` fails the whole file; `MODULE`s are merged. | `a2l/parser.rs:321-346,446,575`; `compu_method.rs:15-24` | Emit warnings for unsupported constructs (surface via `a2l_inspect`), implement `BIT_MASK` and `BYTE_ORDER`, add fixtures for each. | — |

### Severity 6

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-15** | Concurrency | Calibration commands are awaited inline in the DAQ drain loop (a 200 ms timeout overflows the 8192-entry DTO queue); UI↔worker channels are unbounded and grow without limit while the window is minimised; the worker busy-polls with a 2 ms sleep (~500 wake-ups/s idle). | `app_state.rs:412-413,1341,1642,1664,1843`; `transport.rs:63-64` | Bounded channels with drop counters, `tokio::select!` instead of polling, calibration on its own task. |
| **T-16** | Architecture | `AppController` is a god object (57 `pub` fields mixing UI text buffers, domain state, persistence and channels; UI reads/writes them directly); `run_background_worker` is a ~530-line function with all state in local `Option`s. Free recording functions live in the UI state module; `recording` depends on `ui::axis_ticks`. | `app_state.rs:311-395,1316-1848`; `video_recorder.rs:7` | Split into view state / session state / persistence; worker becomes an explicit state machine; move `axis_ticks` out of `ui`. |
| **T-17** | Observability | No logging facade at all (`log`/`tracing` absent). The only diagnostic is one overwritten `status_message` string. Field debugging is impossible. | whole `src/` | Add `tracing` with a file subscriber under `logfile/`; every swallowed error becomes a `warn!`. |
| **T-19** | CI/Release | Windows `.exe` is shipped but **never tested**; `release.yml` runs on every push to `main` and publishes on tag without depending on CI; floating `dtolnay/rust-toolchain@stable` with `-D warnings` will break on new lints; no `rust-version`/MSRV job (`is_multiple_of` needs ≥ 1.87 while README said 1.80); `contents: write` at workflow level; actions pinned to tags, not SHAs. | `.github/workflows/*.yml`; `mock_server/mod.rs:370` | Windows test job, `needs:` on CI, `rust-version = "1.87"` + MSRV job, per-job permissions, SHA-pinned actions. `--locked` added 2026-10-24. |

### Severity 5

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-21** | Clock | 32-bit timestamp wrap is detected only between consecutive samples (any event slower than ~2.1 s at ns ticks mis-times); the clock is anchored once and never re-synced. *(Short-reply parsing from reserved bytes fixed in `6527c1c`.)* | `clock_sync.rs:121-124` | Periodic re-sync, wrap handling by elapsed host time, test with a slow event. |
| **T-22** | Replay | Every part is fully loaded into memory, one `BTreeMap` per sample (many × file size); loading happens on the UI thread. | `replay/mod.rs:62-64,138-201`; `top_bar.rs:219` | Stream datasets lazily by time window; load in the worker. |
| **T-23** | Storage | All values are stored as `f64` (`u64`/`i64` above 2⁵³ lose precision); the split estimate ignores the growing JSON index so a part can exceed the configured size. *(Reader arithmetic made checked in `6f54385`.)* | `sdf_writer.rs:239-249,356` | Native dtypes, index size in the budget. |
| **T-24** | Packaging | The mock server and sample A2L are compiled into the **release binary** (no feature gate); the mock is happy-path only (ignores `MAX_CTO`, no error injection, no byte-order mode, memory map grows unbounded). | `lib.rs:9`; `mock_server/mod.rs:44,191-206` | `mock` cargo feature on by default for dev/test, off for release; add error/loss injection knobs. |
| **T-25** | API | `lib.rs` exports all 10 modules `pub` (359 `pub` items, incl. `cto::*`, controller fields); `AppController` methods return `Result<_, String>` discarding typed errors; 10 `#[allow(too_many_arguments)]`. | `lib.rs`; `app_state.rs` | `pub(crate)` by default, re-export a curated surface, typed `AppError`. |
| **T-30** | Recording | Two recordings started within the same second get the **same filenames** (`<Prefix>_<UTC-seconds>_PART001.sdf`), so the second silently overwrites the first. Surfaced while writing the T-13 reconnect tests. | `src/recording/file_namer.rs` | Add a millisecond or sequence suffix, or refuse to start when the target file exists; test two starts within one second. |
| **T-31** | Recording | A `StartRecording` command while a recorder is already active **replaces it without `finish()`**, discarding the in-memory part. The UI never sends this, but the command surface allows it. | `src/app_state.rs` `WorkerCommand::StartRecording` arm | Finish-and-report the active recorder first (reuse `finish_recorders_and_report`) or reject the command; test it. |

### Severity 4

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|
| **T-26** | Hygiene | No `cargo-deny` CI job or dependabot (a `deny.toml` exists since `bf977b4`); duplicate `thiserror` 1 and 2 in the lock, pinned by `eframe 0.29`'s `calloop`/`smithay` so our own bump cannot remove it; `egui` 0.29 is behind. *(tokio features trimmed in `4d45c23`.)* | `Cargo.toml`, `Cargo.lock` | `cargo deny check` in CI, dependabot, plan the egui upgrade (which also retires thiserror 1). |
| **T-27** | Docs | README/docs claimed H.264 video, standards HDF5, "fallback" A2L order, "no runtime dependencies", a headless self-check that parses A2L and records (it renders one frame), Rust 1.80, and listed 6 of 11 test files. **Corrected 2026-10-24** in `README.md`, `docs/*.md`, `Cargo.toml`, `src/lib.rs`. Keep it that way via `.agents/rules/docs-honesty.md`. | `src/main.rs:14-24` | Done; re-verify with `grep -rn "H.264\|HDF5" README.md docs/`. |

### Severity 3

| ID | Area | Problem | Evidence | Definition of fixed |
|---|---|---|---|---|

---

## Recommended fix order

1. **Decide the two formats (ADR):** real H.264 via ffmpeg or an honestly named raw stream (T-01); real HDF5 or an honestly named container (T-02). Everything downstream depends on this.
2. **Make recordings durable:** incremental writes, tmp+rename, fsync (T-03; the worker is joined on exit since T-07).
3. **Make the command channel trustworthy:** request/response matching, SYNCH, retry (T-04).
4. **Negotiate DAQ parameters** and honour byte order and timestamp units (T-09, T-20, T-21).
5. **Move recording and rendering off the socket runtime** (T-05, T-15).
6. **Surface every error** to the UI and to a log (T-10, T-17; T-06, T-12 and T-18 are closed).
7. **Split `AppController` and the worker** into a view model, a session state machine and persistence (T-16, T-25).
8. **Harden untrusted input** with caps and fuzzing (T-11, T-14, T-23).

## Process debt

- Keep the 95 % gate and enforce the testing rules (`.agents/rules/testing.md`); the sleep-based and
  assertion-free tests were rewritten in T-13 (`50fa45b`). `test_ui_all_states_and_interactions_headless`
  still asserts little about rendered UI.
- Windows CI job, `needs:` between release and CI, MSRV job, SHA-pinned actions, per-job permissions (T-19).
- Run `cargo run -q -p xcp_devtool -- invariants-check` and `daq-plan-dryrun` in every change
  (already wired into `ci.yml`).

## Closed

Closed 2026-10-24 on `feature/daq-loss-detection`, one commit per item (`git show <hash>`). Partially
closed items (T-11, T-21, T-23, T-26) keep a reduced row above with the landed part in italics.

| ID | Commit | What landed |
|---|---|---|
| T-28 | `bf977b4` | no-op `build.rs` and dead code removed; `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml` added |
| T-29 | `6bbffc9` | `consume_matrix_dims()` replaces six copies; eight named constants with reasons |
| T-20 | `e34c791` | `MAX_ODTS_PER_DAQ_LIST = 252`, `MAX_DAQ_LISTS = 256`; devtool invariant is an error |
| T-12 | `e3d4779` | unreadable `settings.json` is backed up and reported; `write_atomic()` shared by settings and profiles |
| T-07 | `f64ab1e` | `JoinHandle` kept; `Drop` joins with a 3 s deadline; drop-while-recording test |
| T-06 | `6155ca5` | `WorkerEvent::RecordingFailed`, `finish_recorders_and_report()`, `last_recording_error` in the top bar; two failure tests |
| T-13 | `50fa45b` | `tests/common/mod.rs` poll helpers; no fixed sleeps; deterministic timeout endpoint; real assertions |
| T-18 | `1bee787` | offline submit returns an error and marks the entry, instead of Synced |
| T-08 | `324ce23` | upload first, local file only on failure; `A2lSource` in the connect status; mock `start_with_options` |
| T-26 (tokio part) | `4d45c23` | `rt-multi-thread` dropped, `macros` moved to dev-dependencies |
| T-21 (short reply) | `6527c1c` | 4..6-byte `GET_DAQ_CLOCK` replies no longer anchor the clock from reserved bytes |
| T-23 (reader) | `6f54385` | checked offset/length arithmetic in `read_f64_dataset` / `read_i64_dataset` |
| T-11 (caps) | `f3f6f40` | `MAX_UPLOAD_BYTES`, clamped `MATRIX_DIM`, saturating `element_count()` |

## How to use this file

Pick the highest open item whose "Order" prerequisites are done. When an item is fixed, move it to a
**Closed** section at the bottom with the commit hash rather than deleting it. New findings get the
next `T-nn` and a severity using the scale above. `AGENTS.md` describes how agents must reference this
file in every change.
