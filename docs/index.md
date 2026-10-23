# Open_XCP_Client — Documentation Portal

Welcome to the official technical documentation for **Open_XCP_Client**. Every module in the codebase has a narrowly defined scope and corresponding documentation page below.

## Table of Contents

1. [**Architecture & Performance Optimization**](./architecture.md)
   - Multi-threaded separation of UI (`egui`), async XCP I/O (`tokio`), and `.sdf` disk writing.
   - CPU and memory footprint optimizations for low-spec developer laptops.
2. [**XCP Protocol & Vector `xcplite` 2.2.2 Integration**](./xcp_protocol_and_xcplite.md)
   - XCP-on-Ethernet (UDP & TCP) 4-byte framing (`len`, `ctr`).
   - Dynamic A2L upload via `GET_ID (IDT_ASAM_UPLOAD = 4)` + `UPLOAD`.
   - Concurrent CTO calibration and DTO DAQ packet demultiplexing.
   - Target clock correlation (`GET_DAQ_CLOCK` + 32-bit DTO timestamps $\rightarrow$ UTC).
3. [**A2L Parsing & Complex C/C++ Structure Mapping**](./a2l_structure_mapping.md)
   - Resolving `TYPEDEF_STRUCTURE`, `STRUCTURE_COMPONENT`, `TYPEDEF_MEASUREMENT`, `TYPEDEF_CHARACTERISTIC`, and `INSTANCE`.
   - Normalizing dot-separated and array-indexed `MEASUREMENT` names into hierarchical trees.
   - `COMPU_METHOD` conversions (`IDENTICAL`, `LINEAR`, `RAT_FUNC`, `TAB_VERB`).
4. [**HDF5-Based `.sdf` File Format & Configurable Size Splitting**](./sdf_file_format.md)
   - HDF5 Superblock v2 / Object Header v2 layout (`/Events` and `/Measurements` hierarchies).
   - File naming specification: `<Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART001.sdf`.
   - Configurable split size (`max_logfile_size_mb`, default `500 MB` in `settings.json`).
5. [**Calibration Guide & Type-Safe Validation**](./calibration_guide.md)
   - Finding and adding `CHARACTERISTIC` parameters to the active Calibration Content list.
   - Strict data-type and A2L `[LowerLimit, UpperLimit]` validation on **Enter**.
   - Concurrent `SHORT_DOWNLOAD` / `DOWNLOAD` with `SHORT_UPLOAD` readback verification.
6. [**UI / UX Operator Manual**](./ui_ux_manual.md)
   - Top navigation bar and the **`⚙ Settings`** menu modal.
   - Operating the **Logging Tab** and **Calibration Tab**.
   - Using the built-in Mock `xcplite` 2.2.2 ADAS target for offline validation.
7. [**CI/CD Pipelines, Release Packaging & Test Coverage**](./ci_cd_and_packaging.md)
   - GitHub Actions workflows (`ci.yml` and `release.yml`).
   - Running tests and verifying $\ge 95\%$ source code coverage.

---

## Source Module Cross-Reference

- [`src/lib.rs`](../src/lib.rs) — Library root
- [`src/main.rs`](../src/main.rs) — Native GUI & headless self-check binary entry point
- [`src/config_store.rs`](../src/config_store.rs) — Persistent JSON settings store
- [`src/app_state.rs`](../src/app_state.rs) — UI controller & background worker orchestration
- [`src/a2l/`](../src/a2l/mod.rs) — A2L lexer, parser, structure resolver, and conversion rules
- [`src/xcp/`](../src/xcp/mod.rs) — XCP packet framing, UDP/TCP transport, DAQ planner, and clock sync
- [`src/recording/`](../src/recording/mod.rs) — `.sdf` (HDF5 v2) writer/reader, file namer, and size splitter
- [`src/calibration/`](../src/calibration/mod.rs) — Calibration validator and active watch-list manager
- [`src/ui/`](../src/ui/mod.rs) — `egui`/`eframe` user interface components
- [`src/mock_server/`](../src/mock_server/mod.rs) — Mock `xcplite` 2.2.2 server & sample ADAS A2L
