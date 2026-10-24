---
trigger: always_on
description: Documentation must describe tested behaviour only, with wording that matches the code
---
# Documentation honesty

- A capability is documented only when a test demonstrates it. Each sentence you add or change maps to a symbol or test you can name.
- Wording matches behaviour: a container is not a codec; a path that is used instead of the upload is an override, not a fallback; a self-check that renders one frame is not a recorder test.
- Numbers come from constants (`MAX_ODT_ENTRIES`, `PROFILE_FORMAT_VERSION`, `sdf_version`, `Cargo.toml` version), never from memory.
- Planned or partial work is never written in the present tense. Known gaps point at their `docs/TODO.md` id.
- When behaviour changes, `README.md`, `docs/*.md` and `Cargo.toml` `description` change in the same commit; when a feature is removed, its docs go too.
- Close or add the matching `docs/TODO.md` item with every change. Closed items move to a Closed section with the commit hash; they are not deleted.
