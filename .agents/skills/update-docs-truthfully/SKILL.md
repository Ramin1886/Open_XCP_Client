---
name: update-docs-truthfully
description: Edit README.md, docs/ and the Cargo description of open_xcp_client so every claim maps to code or a test. Use when behaviour changed, when a doc claim looks wrong, or when closing a docs/TODO.md item.
---
# update-docs-truthfully

1. For every sentence you add or change, write down (in the commit body if useful) the symbol or test that makes it true. No symbol → soften or delete the claim.
2. Check the known traps: `.mp4` is an RLE RGB frame stream, not H.264 (T-01); `.sdf` is an HDF5-signature container with a JSON tail index, not generic HDF5 (T-02); `local_a2l_fallback_path` is used *instead of* the upload (T-08); `--headless-self-check` renders one frame; only Linux is CI-tested.
3. Numbers from constants: `MAX_ODT_ENTRIES`, `MAX_ODTS_PER_DAQ_LIST`, `PROFILE_FORMAT_VERSION`, `sdf_version`, `Cargo.toml` `version` and `rust-version`.
4. Keep `README.md`, `docs/index.md`, the specific `docs/*.md` page and `Cargo.toml` `description` consistent in the same commit.
5. Planned work is never present tense; point at the `docs/TODO.md` id instead.
6. Verify: `grep -rn -i "h\.264\|hdf5\|fallback" README.md docs/ Cargo.toml src/lib.rs` shows only qualified, honest mentions; `cargo run -q -p xcp_devtool -- invariants-check --check version-consistency` is ok.
7. Update `docs/TODO.md`: close the item (move to Closed with the commit hash) or add a new `T-nn` with severity.
