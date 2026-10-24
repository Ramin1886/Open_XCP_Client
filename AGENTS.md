# AGENTS.md — working on `open_xcp_client`

Stable guidelines for humans and coding agents. This file holds only rules that rarely change.
Findings, decisions and history live elsewhere: backlog in `docs/TODO.md`, decisions in `docs/adr/`
(created with the first ADR), reference docs in `docs/`. Claude Code reads this file through
`CLAUDE.md`; Google Antigravity reads it directly.

## 1. Purpose and non-goals

`open_xcp_client` is an XCP-on-Ethernet (UDP/TCP) measurement and calibration client for Vector
`xcplite` targets: it uploads the A2L, plans DAQ lists, records samples to `.sdf` part files, plots
them live, replays recordings, and writes calibration values. It is a desktop tool (egui) with a
background tokio worker.

It must never, silently:
- lose, reorder or alter a measurement sample, or emit a sample from an incomplete DAQ cycle;
- write a file in a format the documentation does not describe truthfully;
- touch target memory outside an explicit calibration action by the operator;
- report "Connected" or "Recording" when the link or the recorder has failed.

## 2. Repository map (by area)

| Area | Path | Notes |
|---|---|---|
| A2L lexer/parser/model, struct resolver, COMPU methods | `src/a2l/` | Parses text uploaded from the target or read from disk |
| XCP framing, transport, master client, DAQ/ODT planner, CTR link stats, clock sync | `src/xcp/` | Protocol invariants live here (see §4) |
| `.sdf` writer/reader, `.mp4` frame-stream writer, file naming, size splitting | `src/recording/` | On-disk formats (see §4) |
| Multi-part `.sdf` replay | `src/replay/` | |
| Calibration validation and watch-list | `src/calibration/` | |
| Settings (`config/settings.json`) and plot profiles (`config/profiles/*.json`) | `src/config_store.rs`, `src/plot_profile.rs` | Versioned user data |
| Application controller, worker thread, UI↔worker protocol | `src/app_state.rs` | |
| egui tabs and panels | `src/ui/` | |
| In-process mock `xcplite` target and sample A2L | `src/mock_server/` | Used by tests and the devtool |
| Integration tests (the only tests) | `tests/` | No `#[cfg(test)]` in `src/` |
| Developer CLI + MCP server | `tools/xcp_devtool/` | Workspace member, not shipped |
| Reference documentation and backlog | `docs/` | `docs/TODO.md` is the backlog |

The per-file table in `README.md` ("Codebase Architecture") is the detailed map.

## 3. Commands

Linux build prerequisites:
`pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev`.
All runtime paths (`./config`, `./logfile`) are relative to the current directory: run from the repo root.

```bash
cargo build --release                    # product binary: target/release/open_xcp_client
cargo run                                # GUI
cargo run -- --headless-self-check       # renders one UI frame, no network
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo llvm-cov --all-targets -p open_xcp_client --ignore-filename-regex 'src/main\.rs' --fail-under-lines 95
cargo build -p xcp_devtool               # one-time; the MCP launcher rebuilds when sources change
cargo run -q -p xcp_devtool -- <subcommand>   # see §3.1
```

### 3.1 Developer tool (`xcp_devtool`)

Same operations as CLI subcommands (kebab-case) and MCP tools (snake_case); both print one JSON
object with `ok` and `problems`. Exit code 0 = ok, 1 = not ok, 2 = operational error.

| Tool | Use it when | Side effects |
|---|---|---|
| `a2l-inspect` / `a2l_inspect` | You need events, block counts, the structure tree or the list of A2L blocks the parser skips. Takes `--path` or `--builtin adas_sample`. | none |
| `a2l-find-signal` / `a2l_find_signal` | Looking up a measurement/characteristic by substring or `*` glob. | none |
| `sdf-inspect` / `sdf_inspect` | Validating a recorded `.sdf` part: checksums, tail index, dataset ranges, time monotonicity, integrity counters. Run it after any change under `src/recording/`. | none |
| `daq-plan-dryrun` / `daq_plan_dryrun` | After touching `src/xcp/daq_planner.rs`: run with `--max-dto 8 32 256 1024`; `violations` must be empty. | none |
| `mock-server` / `mock_server` | You need a local target. CLI runs in the foreground; MCP keeps servers until stopped. | binds ports |
| `xcp-probe` / `xcp_probe` | Checking that a target answers CONNECT/GET_STATUS. | opens an XCP session on the target; only against hardware when the operator asked |
| `project-check` / `project_check` | Structured fmt/clippy/test result for one package. | runs cargo; rewrites files only with `fix_fmt` |
| `invariants-check` / `invariants_check` | Before declaring any change done, and in CI. | none |

MCP clients: Claude Code via `.mcp.json`, Antigravity via `.agents/mcp_config.json` (both launch
`tools/xcp_devtool/launch.sh`; if Antigravity cannot resolve the relative path, set `cwd` to the
absolute repository path in that file).

## 4. Invariants (never change without a human-approved ADR)

Full rules live in `.agents/rules/protocol-invariants.md` and `.agents/rules/file-formats.md`.
In one line each:

- ODT entries per ODT ≤ `MAX_ODT_ENTRIES`, ODTs per DAQ list ≤ `MAX_ODTS_PER_DAQ_LIST`
  (`src/xcp/daq_planner.rs`); ODT 0 reserves the timestamp bytes; measurements that do not fit are
  dropped and reported, never truncated; usable ODT numbers must stay clear of reserved PIDs 0xFC–0xFF.
- A DAQ cycle is emitted only when complete and in order; CTR gaps, reorders and duplicates are
  counted, never hidden (`src/xcp/link_stats.rs`, `tests/daq_loss_detection_test.rs`).
- `.sdf` parts carry the `sdf_version` string and the `SDFIDX02` tail index; older files stay readable;
  part filenames follow `<Prefix>_<UTC>_PART%03d.{sdf,mp4}` and must stay lexically sortable.
- `settings.json` fields are only added (`#[serde(default)]`), never renamed; plot profiles gate on
  `PROFILE_FORMAT_VERSION`; newer-than-supported files are rejected, not reinterpreted.
- Every source file starts with `// SPDX-License-Identifier: GPL-2.0-only`; dependencies must be
  GPL-2.0 compatible.

## 5. Definition of done (every change)

1. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace` pass; product coverage stays ≥ 95 %.
2. New behaviour has a test in `tests/` that asserts an observable outcome. No test exists only to
   touch lines. Async results are awaited with a deadline loop, never a fixed `sleep` then assert.
   Network tests use `MockXcpServer::start(0)`; file tests use `tempfile`.
3. `cargo run -q -p xcp_devtool -- invariants-check` is ok; `daq-plan-dryrun` has no violations if
   `src/xcp/` changed; `sdf-inspect` passes on a fresh recording if `src/recording/` changed.
4. No new `unwrap()`/`expect()` in `src/`; no `let _ =` on an I/O `Result`; errors reach the UI or a log.
5. Every new `pub` item has a `///` doc comment; no new `#[allow]` without a one-line reason.
6. Documentation that describes the changed behaviour is updated in the same change, and the matching
   `docs/TODO.md` item is closed or a new one is added.
7. The commit message says what changed and why; nothing from §6 below happened without approval.

## 6. Needs explicit human approval

Adding or upgrading a dependency; changing an on-disk format or bumping its version; adding an XCP
command; editing `.github/workflows/*`; editing `[profile.*]` in `Cargo.toml`; deleting or weakening a
test; lowering the coverage gate; `git tag`, pushing tags or anything that triggers a release;
connecting to a real ECU; force-pushing or rewriting history.

## 7. Documentation honesty

A capability may be documented only when a test demonstrates it. Wording must match behaviour: a
container is not a codec, an override is not a fallback, a self-check that renders one frame is not
a recorder test. Numbers come from constants (`MAX_ODT_ENTRIES`, `PROFILE_FORMAT_VERSION`,
`sdf_version`), not memory. Planned work is never written in the present tense.

## 8. Where things live

| What | Where |
|---|---|
| Backlog and known limitations | `docs/TODO.md` (severity 1–10, `T-nn` ids; close items, do not delete) |
| Decisions | `docs/adr/NNNN-title.md` (create the directory with the first ADR) |
| Rules (both harnesses) | `.agents/rules/*.md` (`.claude/rules` is a symlink to it) |
| Skills (both harnesses) | `.agents/skills/<name>/SKILL.md` (`.claude/skills` is a symlink to it) |
| Claude Code hooks and permissions | `.claude/settings.json`, `.claude/hooks/*.sh` |
| MCP client configs | `.mcp.json` (Claude Code), `.agents/mcp_config.json` (Antigravity) |
| Unwrap allowlist | `tools/xcp_devtool/unwrap_allowlist.txt` |

## 9. Git

Branch from `main` per change. Imperative subject ≤ 72 characters; the body explains why. Never
force-push, `reset --hard` a shared branch, or rewrite published history. Never commit runtime
files: `logfile/*.sdf`, `*.mp4`, `config/settings.json`, `config/profiles/*.json`, `lcov.info`.
Windows contributors need `git config core.symlinks true` for the `.claude/rules` and
`.claude/skills` symlinks.
