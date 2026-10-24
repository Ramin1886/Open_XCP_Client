---
trigger: glob
description: Rust style and error-handling rules for every .rs file in this repository
globs: "**/*.rs"
paths:
  - "**/*.rs"
---
# Rust style

- First line of every `.rs`, `.toml`, `.yml`, `.sh` file: `// SPDX-License-Identifier: GPL-2.0-only` (or `#` for TOML/YAML/shell).
- Edition 2021, `cargo fmt` always; `cargo clippy --workspace --all-targets -- -D warnings` is the bar.
- No new `#[allow(...)]` without a one-line reason on the same line.
- No `unwrap()` or `expect()` in `src/`. Use `?`, `unwrap_or`, or return the module's typed error. The only tolerated sites are listed in `tools/xcp_devtool/unwrap_allowlist.txt`; shrink that list, never grow it without a reason comment.
- Never `let _ =` a `Result` from I/O, networking, or a channel send. Propagate it or turn it into a `WorkerEvent`/status the UI shows.
- Errors are one `thiserror` enum per module (`ConfigError`, `ProfileError`, `SdfError`, `XcpError`, `A2lParseError`, `CalibrationValidationError`). New APIs return those, not `String`.
- Use `BTreeMap`/`BTreeSet` where iteration order reaches the UI, a file, or a test.
- `pub` only what `tests/`, the binary, or `tools/xcp_devtool` need; every `pub` item gets a `///` doc comment.
- No new dependency without saying why in the change description and getting approval (see `AGENTS.md` §6).
- Magic numbers become named constants with a one-line reason (`const CTO_QUEUE_DEPTH: usize = 256; // ...`).
