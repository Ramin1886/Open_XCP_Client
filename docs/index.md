# Open_XCP_Client — Documentation Portal

Welcome to the official technical documentation for **Open_XCP_Client**. Every module in the codebase has a narrowly defined scope and corresponding documentation page below.

## Table of Contents

1. [**Architecture & Performance Optimization**](./architecture.md)
   - Multi-threaded separation of UI (`egui`), async XCP I/O (`tokio`), synchronized `.sdf` + `.mp4` recording, and `.sdf` replay.
   - CPU and memory footprint optimizations for low-spec developer laptops.
2. [**XCP Protocol & Vector `xcplite` 2.2.2 Integration**](./xcp_protocol_and_xcplite.md)
   - XCP-on-Ethernet (UDP & TCP) 4-byte framing (`len`, `ctr`).
   - Dynamic A2L upload via `GET_ID (IDT_ASAM_UPLOAD = 4)` + `UPLOAD`.
   - Immediate live DAQ streaming on connect and concurrent CTO calibration demultiplexing.
   - Target clock correlation (`GET_DAQ_CLOCK` + 32-bit DTO timestamps $\rightarrow$ UTC).
3. [**A2L Parsing & Complex C/C++ Structure Mapping**](./a2l_structure_mapping.md)
   - Resolving `TYPEDEF_STRUCTURE`, `STRUCTURE_COMPONENT`, `TYPEDEF_MEASUREMENT`, `TYPEDEF_CHARACTERISTIC`, and `INSTANCE`.
   - Normalizing dot-separated and array-indexed `MEASUREMENT` names into hierarchical trees.
   - `COMPU_METHOD` conversions (`IDENTICAL`, `LINEAR`, `RAT_FUNC`, `TAB_VERB`).
4. [**`.sdf` Container & `.mp4` Frame-Stream Recording with Configurable Size Splitting**](./sdf_file_format.md)
   - HDF5 signature and superblock plus an embedded JSON index (`/Events` and `/Measurements` dataset names); not readable by generic HDF5 tools yet.
   - Synchronized `.sdf` and `.mp4`-named RLE frame-stream recording (`PART001.sdf` + `PART001.mp4`); the video is not H.264.
   - Configurable split size (`max_logfile_size_mb`, default `500 MB` in `settings.json`).
5. [**Calibration Guide & Type-Safe Validation**](./calibration_guide.md)
   - Finding and adding `CHARACTERISTIC` parameters to the active Calibration Content list.
   - Strict data-type and A2L `[LowerLimit, UpperLimit]` validation on **Enter**.
   - Concurrent `SHORT_DOWNLOAD` / `DOWNLOAD` with `SHORT_UPLOAD` readback verification.
6. [**UI / UX Operator Manual (5 Tabs, 2D Plot & Replay)**](./ui_ux_manual.md)
   - Top navigation bar (`Live` / `Replay` switch), Recording Bar, Replay Transport Bar, and **`⚙ Settings`** modal.
   - Operating the **2D Live Plot**, **Numerical**, **Time Plot**, **Calibration**, and **XCP Info** tabs.
   - Configurable Ego vehicle footprint and interactive `(X, Y)` scalar/vector signal pairing.
7. [**Plot Profiles — Predefined, Reusable Measurement Views**](./plot_profiles.md)
   - JSON (not XML) settings layout: `./config/settings.json` and `./config/profiles/<Name>.json`.
   - Defining, saving, loading, updating, importing and exporting a named measurement view.
   - Behavior when a profile is loaded against an A2L that lacks some of its signals.
8. [**CI/CD Pipelines, Release Packaging & Test Coverage**](./ci_cd_and_packaging.md)
   - GitHub Actions workflows (`ci.yml` and `release.yml`).
   - Running tests and verifying $\ge 95\%$ source code coverage.
9. [**Honest Assessment & Prioritised Backlog (`TODO.md`)**](./TODO.md)
   - Severity-ranked (1–10) findings with file anchors, definition of fixed, and recommended order.
   - Known limitations of the `.sdf` and `.mp4` outputs, protocol negotiation gaps, and process debt.
10. [**Agent Development Guide (`AGENTS.md`)**](../AGENTS.md)
   - Stable guidelines, commands, invariants and definition of done for AI-assisted changes; developer tool `tools/xcp_devtool`.

---

## Source Module Cross-Reference

- [`src/lib.rs`](../src/lib.rs) — Library root
- [`src/main.rs`](../src/main.rs) — Native GUI & headless self-check binary entry point
- [`src/config_store.rs`](../src/config_store.rs) — Persistent JSON settings store
- [`src/plot_profile.rs`](../src/plot_profile.rs) — Named, reusable plot profiles (`./config/profiles/`)
- [`src/app_state.rs`](../src/app_state.rs) — UI controller & background worker orchestration
- [`src/a2l/`](../src/a2l/mod.rs) — A2L lexer, parser, structure resolver, and conversion rules
- [`src/xcp/`](../src/xcp/mod.rs) — XCP packet framing, UDP/TCP transport, DAQ planner, and clock sync
- [`src/recording/`](../src/recording/mod.rs) — `.sdf` container writer/reader, `.mp4`-named frame-stream recorder, file namer, and size splitter
- [`src/replay/`](../src/replay/mod.rs) — Multi-part `.sdf` recording replay engine
- [`src/calibration/`](../src/calibration/mod.rs) — Calibration validator and active watch-list manager
- [`src/ui/`](../src/ui/mod.rs) — `egui`/`eframe` user interface components (5 tabs + top bar + settings + plot profile selector)
- [`src/mock_server/`](../src/mock_server/mod.rs) — Mock `xcplite` 2.2.2 server & sample ADAS A2L
