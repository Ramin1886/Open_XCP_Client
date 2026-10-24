---
name: change-file-format
description: Change the .sdf container, the .mp4 frame stream, settings.json or plot profile layout safely in open_xcp_client - versioning, golden fixtures, backward-compatible readers, docs. Use for any change under src/recording, src/replay, src/config_store.rs or src/plot_profile.rs that alters bytes on disk.
---
# change-file-format

Needs human approval first (`AGENTS.md` §6).

1. Classify: **additive** (old readers ignore new data, new readers read old files) or **breaking**.
2. `.sdf`: breaking changes bump the `sdf_version` string in `src/recording/sdf_writer.rs`; change the `SDFIDX02` magic only if the tail-index layout itself changes. `SdfFileReader::from_bytes` must still open the previous version; if it cannot, that is a different format and needs an ADR.
3. Golden fixture: before changing the writer, record a small part (the e2e test or `SdfSessionRecorder` with a handful of samples) and save it as `tests/fixtures/sdf/v<old>_minimal.sdf` (the `.gitignore` exempts `tests/fixtures/**`). Add a test that opens the fixture and reads one dataset. Repeat for the new version after the change.
4. `settings.json`: add fields with `#[serde(default = ...)]` only; never rename. Profiles: bump `PROFILE_FORMAT_VERSION` and keep the newer-version rejection test.
5. Writes stay atomic (`*.tmp` then rename). Lockstep `.sdf`/`.mp4` rotation stays intact; `PART%03d` naming untouched.
6. Verify: `cargo run -q -p xcp_devtool -- sdf-inspect <fresh part>` and `... sdf-inspect tests/fixtures/sdf/<fixture>` both `ok: true`.
7. Docs: `docs/sdf_file_format.md` (layout, version, honest limitations block), `README.md` dataset-path section, `docs/TODO.md` items T-01/T-02/T-03/T-23 as applicable.
8. Run the `verify-change` skill.
