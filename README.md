# Open_XCP_Client

[![License: GPL v2](https://img.shields.io/badge/License-GPL_v2-blue.svg)](./LICENSE)
[![CI & Coverage](https://github.com/Ramin1886/Open_XCP_Client/actions/workflows/ci.yml/badge.svg)](./.github/workflows/ci.yml)
[![Build & Release](https://github.com/Ramin1886/Open_XCP_Client/actions/workflows/release.yml/badge.svg)](./.github/workflows/release.yml)

**Open_XCP_Client** is a lightweight, high-performance, open-source (**GPL-2.0-only**, same license as the Linux kernel) **XCP on Ethernet (UDP/TCP) logging and live calibration client** engineered in Rust (`egui`/`eframe` + `tokio`).

It is specifically designed for embedded ADAS and automotive software running **Vector `xcplite` 2.2.2** (such as Qualcomm SoC targets running QNX 7.1) where the A2L description is generated dynamically at target startup and served over the network.

---

## Key Features

- **Dynamic A2L Upload (`GET_ID` + `UPLOAD`)**: Automatically retrieves and parses the runtime-generated A2L file directly from the running `xcplite` 2.2.2 XCP server over UDP or TCP (with an optional local `.a2l` file fallback).
- **Complex C/C++ Structure Preservation (`.sdf` / HDF5 v2)**:
  - Expands both ASAM MCD-2 MC v1.7 `TYPEDEF_STRUCTURE` / `INSTANCE` blocks and dot/bracket-delimited measurement paths (e.g., `Perception.RadarFront.targets[0].distance_m`) into hierarchical HDF5 groups (`/Measurements/Perception/RadarFront/targets_0/distance_m`).
  - Preserves scalar, 1D, and multi-dimensional (`MATRIX_DIM`) array shapes.
- **Native Refresh-Rate DAQ & Target UTC Clock Sync**:
  - Automatically groups measurements by their A2L `IF_DATA XCP` `EVENT` channel so each variable is logged at its native refresh rate.
  - Synchronizes target `GET_DAQ_CLOCK` (`0xDC`) and 32-bit DTO packet timestamps to UTC (`time_utc_s` in `float64` seconds and `time_utc_ns` in `int64` nanoseconds).
- **Configurable File-Size Splitting (Default 500 MB)**:
  - Stores recordings by default in `./logfile/<Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART001.sdf`.
  - Automatically rotates to `PART002.sdf`, `PART003.sdf`, etc. (preserving the recording-start UTC timestamp) so no split file ever exceeds the configured `max_logfile_size_mb` setting (default **500 MB** in [`config/settings.default.json`](./config/settings.default.json)).
- **Concurrent Live Calibration**:
  - Dedicated **Calibration Tab** to search `CHARACTERISTIC` parameters, add them to an active watch-list, inspect current target values converted via `COMPU_METHOD`, edit values in type-checked inputs, and press **Enter** to send (`SHORT_DOWNLOAD` / `DOWNLOAD` with readback verification) **while DAQ recording continues uninterrupted**.
- **Dedicated Settings Menu (`⚙ Settings`)**:
  - All connection, splitting size (`max_logfile_size_mb`), folder/prefix, and calibration settings are managed behind a clean **`⚙ Settings`** menu button in the top bar and persisted in `./config/settings.json` (never polluting `.sdf` files intended for cloud upload).

---

## Documentation Hub

Detailed modular documentation is available in [`./docs/`](./docs/index.md):

1. [**Documentation Index & Overview**](./docs/index.md)
2. [**System Architecture & Threading Model**](./docs/architecture.md)
3. [**XCP on Ethernet Protocol & XCPlite 2.2.2 Integration**](./docs/xcp_protocol_and_xcplite.md)
4. [**A2L Parsing & Complex C/C++ Structure Mapping**](./docs/a2l_structure_mapping.md)
5. [**HDF5-Based `.sdf` File Format & Configurable Splitting**](./docs/sdf_file_format.md)
6. [**Live Calibration Guide & Data-Type Validation**](./docs/calibration_guide.md)
7. [**UI / UX Operator Manual (Settings Menu, Logging & Calibration Tabs)**](./docs/ui_ux_manual.md)
8. [**CI/CD Pipelines, Release Packaging & >=95% Test Coverage**](./docs/ci_cd_and_packaging.md)

---

## Quick Start

### 1. Download Prebuilt Binaries (Windows `.exe` & Linux)
Self-contained native executables for Windows (`open_xcp_client.exe`) and Linux (`open_xcp_client`) are built automatically by the [GitHub Actions Release Pipeline](./.github/workflows/release.yml).

### 2. Build from Source
```bash
cargo build --release
```

### 3. Run the Test Suite
```bash
cargo test
```

---

## Repository Structure

| Path | Responsibility |
| :--- | :--- |
| [`Cargo.toml`](./Cargo.toml) | Rust crate manifest and release optimization profile |
| [`LICENSE`](./LICENSE) | GNU General Public License v2.0 (`GPL-2.0-only`, Linux kernel license) |
| [`config/settings.default.json`](./config/settings.default.json) | Default JSON configuration (including `max_logfile_size_mb: 500`) |
| [`src/config_store.rs`](./src/config_store.rs) | JSON configuration loader, validator, and saver |
| [`src/a2l/`](./src/a2l/mod.rs) | Zero-copy A2L lexer, parser, `TYPEDEF_STRUCTURE`/`INSTANCE` resolver, and `COMPU_METHOD` engine |
| [`src/xcp/`](./src/xcp/mod.rs) | XCP-on-Ethernet framing, async UDP/TCP transport, DAQ planner, and target-to-UTC clock sync |
| [`src/recording/`](./src/recording/mod.rs) | HDF5 v2 `.sdf` writer/reader, UTC file namer, and configurable size splitter |
| [`src/calibration/`](./src/calibration/mod.rs) | Strict data-type & A2L limit validator and concurrent calibration state manager |
| [`src/ui/`](./src/ui/mod.rs) | `egui`/`eframe` Top Bar, `⚙ Settings` modal, Logging Tab, and Calibration Tab |
| [`src/mock_server/`](./src/mock_server/mod.rs) | Built-in Mock `xcplite` 2.2.2 UDP/TCP server with realistic ADAS structures |
| [`tests/`](./tests/) | Comprehensive unit, headless UI, and end-to-end integration test suite |
| [`.github/workflows/`](./.github/workflows/) | Automated CI (`>=95%` coverage check) and Windows/Linux release build pipelines |

---

## License

This project is licensed under the **GNU General Public License v2.0 only (`GPL-2.0-only`)** — the same license as the Linux kernel. See [`LICENSE`](./LICENSE) for full terms.
