# UI / UX Operator Manual (5 Tabs, 2D Plot & Replay)

[← Back to Documentation Index](./index.md)

## Interface Layout

**Open_XCP_Client** provides a clean, responsive 5-tab engineering workspace composed of:
1. **Top Navigation & Mode Bar** ([`src/ui/top_bar.rs`](../src/ui/top_bar.rs)):
   - **`🟢 Live` / `🎞 Replay`** mode switch
   - **5-Tab Switcher**: **`📐 2D Live Plot`** (`Ctrl+1`), **`🔢 Numerical`** (`Ctrl+2`), **`📈 Time Plot`** (`Ctrl+3`), **`🎛 Calibration`** (`Ctrl+4`), and **`📡 XCP Info`** (`Ctrl+5`)
   - **Context Bar**: Live Recording Bar (Folder, Prefix, MP4 toggle, FPS, `⏺ Start / ⏹ Stop Recording`) in `Live` mode, or `.sdf` Replay Transport Bar (`.sdf` loader, `▶ Play / ⏸ Pause`, `■ Stop`, Speed `0.25×..10× / max`, continuous Time Slider) in `Replay` mode
   - **`⚙ Settings` Button** ([`src/ui/settings_menu.rs`](../src/ui/settings_menu.rs))
2. **Tab 1 — `📐 2D Live Plot`** ([`src/ui/plane_plot_tab.rs`](../src/ui/plane_plot_tab.rs))
3. **Tab 2 — `🔢 Numerical`** ([`src/ui/numerical_tab.rs`](../src/ui/numerical_tab.rs))
4. **Tab 3 — `📈 Time Plot`** ([`src/ui/time_plot_tab.rs`](../src/ui/time_plot_tab.rs))
5. **Tab 4 — `🎛 Calibration`** ([`src/ui/calibration_tab.rs`](../src/ui/calibration_tab.rs))
6. **Tab 5 — `📡 XCP Info`** ([`src/ui/xcp_info_tab.rs`](../src/ui/xcp_info_tab.rs))

---

## 1. Operating the `2D Live Plot` Tab (`Ctrl+1`)

- **Coordinate Convention**: Defaults to **ISO 8855 vehicle body coordinates** (`+x` forward/up, `+y` left, origin `(0, 0)` at the rear-axle center). Toggle **ISO 8855 (+X Up, +Y Left)** in the left sidebar or in `⚙ Settings` to switch between ISO 8855 and standard Cartesian (`+x` right, `+y` up) axes.
- **Interactive Navigation**:
  - **Mouse Wheel**: Zoom in/out anchored at the cursor position (`1..500 px/m`) with a zoom-adaptive `1-2-5` metric grid (`0.1 m`..`50 m`).
  - **Left-Drag**: Pan the 2D ground plane.
  - **Double-Click** (or **`↺ Reset View`**): Reset the viewport to the default origin view.
- **Configurable Ego Vehicle Footprint**:
  - In **`⚙ Settings`**, configure **Ego Length (m)** (default `4.85`), **Ego Width (m)** (default `1.90`), and **Rear-Axle to Center (m)** (default `1.425`).
- **Pairing `(X, Y)` Signals (Scalar, Vector, Broadcast & Polygons)**:
  - Use the **Filter A2L Signals** box in the left sidebar to search all measurements.
  - Click **`Set X`** and **`Set Y`** next to any measurement (or type the signal names directly), choose a **Style** (`Points / Scatter`, `Connected Line`, `History Trail`, or `4-Corner Polygon (P0..P3)`), pick a color, and click **`➕ Add (X, Y) Layer to 2D Plot`**.
  - Works seamlessly whether `X` and `Y` are both scalars, both 1D/N-D arrays (`MATRIX_DIM`), or one scalar and one vector (automatic broadcast), with optional **Valid Count** truncation.

---

## 2. Operating the `Numerical` Tab (`Ctrl+2`)

- Displays all measurements organized by their C/C++ structure hierarchy.
- Type in **🔍 Filter** to narrow down signals or structure groups immediately.
- Use **`➕ Expand All`** / **`➖ Collapse All`** to expand or collapse all nested structure groups.
- Shows each signal's physical value (or verbal `COMPU_VTAB` enum string / array summary), unit, raw value, and XCP event rate, with a **1.0-second visual highlight** whenever a signal's value changes.

---

## 3. Operating the `Time Plot` Tab (`Ctrl+3`)

- In the left panel, filter and check up to 12 signals (`☑`) to plot in the 2-column strip-chart grid.
- Select a time window preset (**`10 s`**, **`20 s`**, **`60 s`**, **`120 s`**) or hover any chart and scroll the mouse wheel to zoom the time window (`1..120 s`) anchored at the newest sample on the right edge.
- Double-click any chart to reset the time window to `20 s`, or click **`❄ Freeze`** to pause trace updates for inspection.

---

## 4. Operating the `Calibration` Tab (`Ctrl+4`) & `XCP Info` Tab (`Ctrl+5`)

- **Calibration Tab (`Ctrl+4`)**: Search A2L `CHARACTERISTIC` parameters on the left, click **`+ Add`**, edit the value in the **New Value** field, and press **`Enter`** to validate and transmit to target memory concurrently during live streaming or recording.
- **XCP Info Tab (`Ctrl+5`)**: Inspect target connection metadata, A2L summary, active/finalized `.sdf` and `.mp4` part files, and the live per-`EVENT` DAQ telemetry table (A2L Cycle `ms`, Measured Mean/Max Cycle `ms`, Sample Count, Latest UTC Timestamp, Age `ms`).

---

## 5. Multi-Part `.sdf` Replay (`🎞 Replay` Mode)

1. Click **`🎞 Replay`** in the top bar.
2. Enter a path to a single `.sdf` file, multiple comma-separated `.sdf` files, or a directory containing `PART001.sdf`, `PART002.sdf`, ..., and click **`📂 Load .sdf`**.
3. Use **`▶ Play` / `⏸ Pause`**, **`■ Stop`**, the **Speed** selector (`0.25×`..`10×`, `max`), and the **continuous Time Slider** to scrub across the entire recording while inspecting any of the 5 tabs.
