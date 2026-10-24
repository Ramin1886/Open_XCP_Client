# Contributing to Open_XCP_Client

Thanks for helping. The rules that matter most are in [`AGENTS.md`](./AGENTS.md); they apply to
humans and coding agents alike. This page is the short version.

## Before you start

- Linux build packages: `pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev`.
- Rust `1.87+` (`rust-toolchain.toml` pins `stable` with `rustfmt` and `clippy`).
- Run everything from the repository root; runtime paths (`./config`, `./logfile`) are relative.
- One-time: `cargo build -p xcp_devtool` (developer CLI / MCP server used by the checks below).

## Making a change

1. Branch from `main`; one branch per change.
2. Write the test first or with the code, in `tests/*_test.rs`. A test asserts an observable
   outcome and waits for async results with a deadline loop, never a fixed sleep.
3. Keep the invariants in `.agents/rules/protocol-invariants.md` and `.agents/rules/file-formats.md`.
4. Update `README.md` / `docs/*.md` in the same commit when behaviour changes, and close or add the
   matching item in [`docs/TODO.md`](./docs/TODO.md).

## Definition of done

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo llvm-cov --all-targets -p open_xcp_client --ignore-filename-regex 'src/main\.rs' --fail-under-lines 95
cargo run -q -p xcp_devtool -- invariants-check
cargo run -q -p xcp_devtool -- daq-plan-dryrun --builtin adas_sample --max-dto 256   # if src/xcp changed
```

`cargo deny check` (configuration in `deny.toml`) is recommended locally when you touch
`Cargo.toml`; it is not yet part of CI.

## Needs explicit maintainer approval

Adding or upgrading a dependency, changing an on-disk format or its version, adding an XCP command,
editing `.github/workflows/*` or `[profile.*]`, deleting or weakening a test, lowering the coverage
gate, tagging a release, connecting to real hardware.

## Commits and pull requests

Imperative subject line of at most 72 characters, a body that explains why, and a reference to the
`docs/TODO.md` id when one applies. Never force-push shared branches. Never commit `logfile/*.sdf`,
`*.mp4`, `config/settings.json`, `config/profiles/*.json` or `lcov.info`.

## Security

See [`SECURITY.md`](./SECURITY.md). The client parses untrusted input from the target (XCP frames,
uploaded A2L text) and from disk (`.sdf`, JSON); treat every length and size field as hostile.
