# UI / UX Operator Manual

[← Back to Documentation Index](./index.md)

## Interface Layout

**Open_XCP_Client** provides a clean, distraction-free engineering layout composed of:
1. **Top Navigation Bar** ([`src/ui/top_bar.rs`](../src/ui/top_bar.rs))
2. **Settings Menu Modal (`⚙ Settings`)** ([`src/ui/settings_menu.rs`](../src/ui/settings_menu.rs))
3. **Logging Tab (`● Logging`)** ([`src/ui/logging_tab.rs`](../src/ui/logging_tab.rs))
4. **Calibration Tab (`◈ Calibration`)** ([`src/ui/calibration_tab.rs`](../src/ui/calibration_tab.rs))
5. **Bottom Status Bar** ([`src/ui/mod.rs`](../src/ui/mod.rs))

---

## 1. Top Navigation Bar & `⚙ Settings` Menu

- **Tab Switcher**: Toggle between **`● Logging`** and **`◈ Calibration`** at any time — switching tabs never interrupts active recording or network communication.
- **Connection Control**: Displays the target IP, Port, Transport (`UDP`/`TCP`), and loaded A2L summary (`[Meas | Events | Cal]`), alongside the **`Connect` / `Disconnect`** button.
- **`⚙ Settings` Button** (Top-Right): Opens the **Settings Menu modal** where you can configure and save to `./config/settings.json`:
  - **Target IP Address & Port** (default `127.0.0.1:5555`)
  - **Transport Protocol** (`UDP` or `TCP`)
  - **Connect Timeout (ms)** (default `2000 ms`)
  - **Local A2L Fallback Path** (optional local `.a2l` file if target upload is disabled)
  - **Built-in Mock XCPlite Server** (check this option to spawn an in-process simulated QNX ADAS target on Connect)
  - **Logfile Split Size (MB)** (default **`500 MB`**)
  - **Default Log Folder** (`./logfile`) and **Default Logfile Prefix** (`Measurement`)
  - **Target UTC Clock Preference**, **Readback Verification**, **Active Calibration Page**, and **Dark Mode**.

---

## 2. Operating the Logging Tab

1. Verify or edit the **Storage Folder** input field (default `./logfile`).
2. Verify or edit the **Logfile Name Prefix** input field (default `Measurement`). A live preview shows the exact filename pattern:
   `./logfile/Measurement_YYYY-MM-DD_HH-MM-SSZ_PART001.sdf`
3. Click **`⏺ Start Recording`**:
   - The button turns red (`⏹ Stop Recording`).
   - All A2L measurements are subscribed to DAQ lists at their native `EVENT` refresh rates.
   - The progress bar tracks the current `.sdf` file size against the configured **Split Limit** (default `500 MB`) and automatically increments `PART001` $\rightarrow$ `PART002` when full.
4. Browse the **Recorded Measurement Variables & Hierarchy** tree to inspect nested C/C++ structures, event rates, units, and live decoded values.
5. Click **`⏹ Stop Recording`** to stop DAQ acquisition and flush the active `.sdf` part to disk.

---

## 3. Operating the Calibration Tab

1. In the left pane (**1. Find Calibration Variable**), type a substring in the **Search** box to locate any A2L `CHARACTERISTIC`.
2. Click **`+ Add`** to place the parameter into the right pane (**2. Active Calibration Content**).
3. Inspect the **Current** value read from the target.
4. Type a new value in the **New Value** input field (or choose an enum state from the combo box) and press **`Enter`**.
5. Confirm that the status badge transitions to **`✓ Synced`** (or displays an inline validation message if the entered value violates the parameter's data type or `[LowerLimit, UpperLimit]`).

---

## Next Steps
- [CI/CD Pipelines, Release Packaging & >=95% Test Coverage →](./ci_cd_and_packaging.md)
