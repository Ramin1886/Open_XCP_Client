---
name: release
description: Cut a versioned release of open_xcp_client - version bump, consistency checks, release-prep commit, then stop for human approval before tagging. Use when asked to release or bump the version.
---
# release

1. On an up-to-date `main` (or the release branch the human named), run the `verify-change` skill with `cargo test --workspace`; everything green.
2. Bump `version` in the root `Cargo.toml` only (`tools/xcp_devtool` has its own, unrelated version). `cargo build` refreshes `Cargo.lock`.
3. `cargo run -q -p xcp_devtool -- invariants-check --check version-consistency` must be ok.
4. Update version mentions in `README.md` and `docs/` if any; add or update `CHANGELOG.md` with a short list of user-visible changes and the `docs/TODO.md` items closed.
5. Commit: `Release prep: bump version to X.Y.Z`.
6. **STOP.** Tagging (`git tag vX.Y.Z`) and `git push --tags` trigger `release.yml`, which publishes binaries. Ask the human to confirm, or to do it themselves. Never tag or push tags on your own.
7. After the human tags: confirm the release workflow produced both artifacts and note in `docs/TODO.md` T-19 that Windows is still untested unless that changed.
