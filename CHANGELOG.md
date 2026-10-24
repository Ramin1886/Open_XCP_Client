# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow Semantic Versioning.

## [Unreleased]

### Added
- `docs/TODO.md`: severity-ranked audit of the code base with file anchors and a fix order.
- `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`; shared agent rules and skills under `.agents/`;
  Claude Code hooks and permissions; MCP client configuration.
- `tools/xcp_devtool`: developer CLI and MCP stdio server (A2L/SDF inspection, DAQ plan dry-run,
  mock server, target probe, project and invariant checks). CI runs its tests and the invariants check.
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`.

### Changed
- README and docs now describe the recording outputs truthfully: the `.mp4` file is an RLE RGB frame
  stream (not H.264) and the `.sdf` file is an HDF5-signature container with a JSON tail index (not
  readable by generic HDF5 tools). Minimum supported Rust version is 1.87.
- CI: workspace-wide clippy, coverage gate scoped to the product crate, `--locked` builds.

### Removed
- No-op `build.rs`; unused `ActiveTab::Logging`, `SizeSplitter::from_max_mb`, `build_ffmpeg_args`.

## [0.2.0] - 2026-10-24

### Added
- 2D live plot with `(X, Y)` signal pairing, ego-vehicle footprint and ISO 8855 axes.
- Synchronized `.sdf` + `.mp4` part recording with lockstep size splitting.
- Multi-part `.sdf` replay with transport bar and time slider.
- Time Plot, Numerical and XCP Info tabs; named, reusable plot profiles.
- Detection and accounting of dropped DAQ data (CTR gaps, incomplete cycles) instead of corrupting samples.

### Fixed
- DAQ allocation no longer exceeds the 255-entry ODT limit.

## [0.1.0] - 2026-10-23

### Added
- Initial release: XCP-on-Ethernet (UDP/TCP) client for xcplite targets with A2L upload, DAQ
  logging to `.sdf`, live calibration, and an egui desktop UI.
