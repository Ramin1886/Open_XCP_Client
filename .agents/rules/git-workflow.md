---
trigger: always_on
description: Branching, commits, and the actions that need explicit human approval
---
# Git workflow

- One branch per change, cut from `main`. Never commit directly to `main`.
- Subject line: imperative, ≤ 72 characters; body says why. Reference `docs/TODO.md` ids when relevant.
- Never `git push --force`, `git reset --hard` on a shared branch, `git branch -D`, `git clean -f`, or rewrite published history.
- Never `git tag` or push tags without explicit approval: tags trigger the release workflow.
- Never commit runtime artefacts: `logfile/*.sdf`, `*.mp4`, `config/settings.json`, `config/profiles/*.json`, `lcov.info`, `target/`.
- Ask before: adding a dependency, editing `.github/workflows/*`, editing `[profile.*]` in `Cargo.toml`, deleting or weakening a test, lowering the coverage gate, connecting to a real ECU.
- Run the `verify-change` skill before asking for review.
