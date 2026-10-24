# Plot Profiles — Predefined, Reusable Measurement Views

[← Back to Documentation Index](./index.md)

## Why

A measurement session is more than a target IP: it is *which* signals you look at, paired how, in
which style, over which time window. Re-picking those signals out of a few hundred A2L
measurements at the start of every test session is wasted work.

A **plot profile** captures one complete measurement view as a single file. Define it once, load
it in the next session, hand it to a colleague.

## Where settings live — JSON, not XML

`Open_XCP_Client` persists everything as **pretty-printed JSON**. There is no XML anywhere, and no
per-user OS configuration directory: both paths below are relative to the process working
directory.

| Path | Contents | Tracked in git |
| :--- | :--- | :--- |
| `./config/settings.json` | All runtime settings — connection, recording, calibration, UI, the current visualization state, and `active_profile` | No (gitignored) |
| `./config/settings.default.json` | Seed values used when `settings.json` does not exist yet | Yes |
| `./config/profiles/<Name>.json` | One plot profile per file | No (gitignored) |
| `./config/profiles/README.md` | Profile schema reference | Yes |

Written by [`ConfigStore::save`](../src/config_store.rs) and
[`ProfileStore::save`](../src/plot_profile.rs). Profile writes are atomic (`<file>.tmp` +
`rename`), so an interrupted save never truncates an existing profile.

Because the paths are working-directory relative, run the binary from the directory holding
`config/` — otherwise a fresh `config/` is created wherever you launched it.

## What a profile stores

Defined by [`PlotProfile`](../src/plot_profile.rs):

| Field | Meaning |
| :--- | :--- |
| `format_version` | Schema version (currently `1`); a file declaring a higher version is refused |
| `name` / `description` | Display name (also the file name) and a free-text note |
| `created_utc` | RFC-3339 UTC timestamp written at save time |
| `a2l_project` / `a2l_module` | A2L `PROJECT` / `MODULE` the view was captured against — provenance only, never enforced |
| `ego` | Ego footprint (`length_m`, `width_m`, `rear_axle_to_center_m`) and `show_ego` / `iso8855_axes` |
| `xy_layers` | 2D Live Plot `(X, Y)` layers — signals, style, color, point radius, valid-count and 4-corner signals |
| `time_plot_signals` | Time Plot selection, at most 12 names |
| `time_plot_window_s` | Time Plot rolling window, `1`..`120` s |
| `calibration_watch_list` | `CHARACTERISTIC` names re-added to the Calibration tab on load |
| `viewport` | Saved 2D zoom/pan, so the exact framing comes back |

Not stored: the target connection. A profile is about *what you measure*, not *where from*, so the
same profile serves a bench target, a vehicle and a mock server.

## Defining, saving and reusing a profile

The `Plot Profiles` section lives in **`⚙ Settings`** and, as a collapsed **`📁 Plot Profiles`**
header, at the top of the **`2D Live Plot`** and **`Time Plot`** sidebars — so switching views
never requires opening the modal.

1. **Build the view.** Pair `(X, Y)` signals into layers in the `2D Live Plot` sidebar, check the
   signals you want in the `Time Plot` picker, set the time window, adjust the Ego geometry, and
   zoom/pan the 2D canvas where you want it.
2. **Name it** in `New Profile Name` (and optionally `Description`).
3. **`💾 Save Current View As Profile`** writes `./config/profiles/<Name>.json` and makes it the
   active profile. Characters that are not alphanumeric, `_` or `-` become `_` in the file name.
4. **Next session:** pick the profile in the selector and click **`📂 Load`**. The active profile is
   remembered in `settings.json` as `active_profile` and re-applied automatically on the next
   start, so the common case needs no clicks at all.
5. **`🔁 Update`** overwrites the selected profile with the current view; **`🗑 Delete`** removes the
   file; **`🔄`** re-scans the directory.

### Sharing

**`⤴ Export`** writes the current view to any path you type; **`⤵ Import`** reads a profile from a
path, applies it, and copies it into `./config/profiles/` so it shows up in the selector. A
profile file is plain JSON with no absolute paths, so copying it between machines is enough.

## Loading a profile against a different A2L

A profile references signals by **A2L name only**, so it is not tied to one ECU build. Loading a
profile whose signals the current A2L does not define **applies it anyway**:

- the unresolved names stay in the profile and in the active view — nothing is silently dropped,
  so the profile still works against the build it was captured from;
- the `Plot Profiles` section reports the count and lists the names;
- the `Time Plot` picker lists them with a `⚠` prefix, and a `2D Live Plot` layer with a missing
  X or Y signal shows its name in amber with a `⚠`;
- the status bar states how many signals did not resolve.

See [`PlotProfile::unresolved_signals`](../src/plot_profile.rs). Measurement references are matched
against the expanded leaf `MEASUREMENT` names produced by the
[structure resolver](./a2l_structure_mapping.md); watch-list references are matched against
`CHARACTERISTIC` names.

## Interaction with the default-seeding heuristic

When an A2L loads and no layers or Time Plot signals are configured,
`init_default_layers_and_plots_for_model` ([`src/app_state.rs`](../src/app_state.rs)) seeds a
heuristic `(X, Y)` pair and the first four measurements. Once a profile has been applied this
seeding is suppressed, so a profile that deliberately starts empty stays empty.

Plot edits made in the sidebars — adding or removing a layer, checking a Time Plot signal,
changing the time window, toggling Ego/axes — are persisted to `settings.json` immediately; they
no longer require a trip through `⚙ Settings`.
