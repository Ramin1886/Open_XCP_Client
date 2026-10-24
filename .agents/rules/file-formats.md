---
trigger: glob
description: Rules for the .sdf container, the .mp4 frame stream, settings.json and plot profile files
globs: "src/recording/**,src/replay/**,src/config_store.rs,src/plot_profile.rs"
paths:
  - "src/recording/**"
  - "src/replay/**"
  - "src/config_store.rs"
  - "src/plot_profile.rs"
---
# On-disk formats

- `.sdf`: HDF5 signature + Superblock v2, data located through the `SDFIDX02` JSON tail index, version in the `sdf_version` string (`src/recording/sdf_writer.rs`). Any layout change bumps the version, keeps `SdfFileReader` able to open the previous version, and adds a golden fixture under `tests/fixtures/` (git-ignore exempts that folder). Document the change in `docs/sdf_file_format.md` in the same commit.
- The `.mp4`-named file is an RLE RGB24 frame stream, not H.264. Do not describe it otherwise; the fix is tracked as `docs/TODO.md` T-01 and needs an ADR.
- Filenames `<Prefix>_<YYYY-MM-DD_HH-MM-SSZ>_PART%03d.{sdf,mp4}` (`src/recording/file_namer.rs`) must stay lexically sortable; replay sorts by name.
- `.sdf` and `.mp4` parts rotate in lockstep; never let one rotate without the other.
- Writes of user data are atomic: write `*.tmp`, then rename (profiles already do; settings must, `TODO.md` T-12).
- `settings.json`: fields are only ever added with `#[serde(default)]`; never rename or remove; a malformed file must be reported, not silently replaced.
- Plot profiles gate on `PROFILE_FORMAT_VERSION`; a newer file is rejected with `ProfileError`, never reinterpreted.
- After any change here: `cargo run -q -p xcp_devtool -- sdf-inspect <fresh part>` must pass all checks.
