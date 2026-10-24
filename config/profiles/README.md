# Plot Profiles

This directory holds **plot profiles** — named, reusable measurement views for
`Open_XCP_Client`. One profile is one JSON file; the file name is derived from the profile
name (non-alphanumeric characters become `_`).

Profiles are *user data*: `config/profiles/*.json` is gitignored, exactly like
`config/settings.json`. Only this README and `.gitkeep` are tracked, so the client ships with
no project-, ECU- or domain-specific view.

## What a profile stores

| Field | Meaning |
|---|---|
| `format_version` | Schema version (currently `1`). A file declaring a higher version is rejected. |
| `name` | Display name shown in the profile selector. |
| `description` | Free-text note describing what the view is for. |
| `created_utc` | RFC-3339 UTC timestamp written when the profile was saved. |
| `a2l_project` / `a2l_module` | A2L `PROJECT` / `MODULE` the view was captured against (provenance only; never enforced). |
| `ego` | Ego vehicle footprint and coordinate convention (`length_m`, `width_m`, `rear_axle_to_center_m`, `show_ego`, `iso8855_axes`). |
| `xy_layers` | 2D Live Plot `(X, Y)` signal pair layers. |
| `time_plot_signals` | Time Plot signal selection (at most 12 names). |
| `time_plot_window_s` | Time Plot rolling window in seconds (`1`..`120`). |
| `calibration_watch_list` | `CHARACTERISTIC` names re-added to the Calibration tab when the profile is applied. |
| `viewport` | Saved 2D zoom/pan (`center_x_m`, `center_y_m`, `pixels_per_meter`), or `null` for the default framing. |

## Using profiles

In the client, the `Plot Profiles` section appears in `⚙ Settings` and, as a collapsed
`📁 Plot Profiles` header, at the top of the `2D Live Plot` and `Time Plot` sidebars:

- **Save Current View As Profile** — writes the current view to `config/profiles/<Name>.json`
- **Load** — applies a profile to every plot tab
- **Update** — overwrites the selected profile with the current view
- **Import / Export** — reads or writes a profile at an arbitrary path, for sharing

The last profile applied is remembered in `config/settings.json` as `active_profile` and
re-applied on the next start.

## Portability across A2L builds

A profile references signals by A2L name only, so it is not tied to one ECU build. Loading a
profile against an A2L that lacks some of those signals **applies it anyway** and flags the
unresolved names with `⚠` in the UI — they stay in the file, so the profile still works against
the build it was captured from.

## Example

Signal names below are placeholders; use the names your own A2L defines.

```json
{
  "format_version": 1,
  "name": "Example_View",
  "description": "Template showing every field; replace the signal names.",
  "created_utc": "2026-10-24T00:00:00+00:00",
  "a2l_project": "",
  "a2l_module": "",
  "ego": {
    "length_m": 4.85,
    "width_m": 1.9,
    "rear_axle_to_center_m": 1.425,
    "show_ego": true,
    "iso8855_axes": true
  },
  "xy_layers": [
    {
      "name": "Object Set",
      "enabled": true,
      "x_signal": "Signal_Pos_X",
      "y_signal": "Signal_Pos_Y",
      "valid_count_signal": "Signal_Valid_Count",
      "corner_signals": ["", "", "", "", "", "", "", ""],
      "style": "Scatter",
      "color_rgb": [56, 189, 248],
      "point_radius": 5.5
    }
  ],
  "time_plot_signals": ["Signal_Speed", "Signal_State"],
  "time_plot_window_s": 60.0,
  "calibration_watch_list": ["Parameter_Gain"],
  "viewport": {
    "center_x_m": 15.0,
    "center_y_m": 0.0,
    "pixels_per_meter": 14.0
  }
}
```

`style` is one of `Scatter`, `Line`, `TrajectoryTrail`, `Polygon4Corner`. For
`Polygon4Corner`, fill `corner_signals` as `[p0_x, p0_y, p1_x, p1_y, p2_x, p2_y, p3_x, p3_y]`.
