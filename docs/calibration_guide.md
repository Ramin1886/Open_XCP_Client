# Live Calibration Guide & Data-Type Validation

[← Back to Documentation Index](./index.md)

## Overview

Implemented in [`src/calibration/`](../src/calibration/mod.rs) and [`src/ui/calibration_tab.rs`](../src/ui/calibration_tab.rs), **Open_XCP_Client** enables engineers to inspect and modify target calibration parameters (`CHARACTERISTIC`) in real time — including while multi-rate DAQ recording is actively running.

---

## Calibration Workflow

1. **Find Calibration Parameter**:
   - Switch to the **◈ Calibration** tab.
   - Use the search box in the **1. Find Calibration Variable** pane to filter A2L `CHARACTERISTIC` parameters by name, struct hierarchy, or description.
   - Click **`+ Add`** next to any parameter to add it to the **2. Active Calibration Content** pane (and persist it in `./config/settings.json` watch-list).
2. **Automatic Initial Read**:
   - Upon adding a parameter (or clicking **`↻ Refresh from Target`**), the client issues an XCP `SHORT_UPLOAD` (or `SET_MTA` + `UPLOAD` for arrays $> 7$ bytes), converts the raw ECU bytes using the parameter's `COMPU_METHOD`, and displays the current target value in the input field.
3. **Modify & Press Enter to Send**:
   - Edit the value in the input field (or select an enum option from the dropdown for `TAB_VERB` parameters).
   - Press **`Enter`** (or click **`↵ Enter`**) to validate and transmit the new value to the target.

---

## Strict Data-Type & Range Validation (`src/calibration/validator.rs`)

Before any XCP download packet is transmitted, [`validate_and_encode_characteristic`](../src/calibration/validator.rs) enforces four validation gates:

1. **Array Dimension Check**:
   - For scalar (`VALUE`) parameters, expects 1 value.
   - For array (`VAL_BLK`, `CURVE`, `MAP`) parameters with `MATRIX_DIM`, verifies that the bracket/comma-separated input contains the exact `element_count()`.
2. **Strict Data-Type Syntax Check**:
   - Integer parameters (`UBYTE`, `SBYTE`, `UWORD`, `SWORD`, `ULONG`, `SLONG`, `A_UINT64`, `A_INT64`) with `IDENTICAL` conversion reject floating-point strings (such as `"12.34"`) and accept decimal or hexadecimal (`0x...`) integers.
   - Boolean `UBYTE` parameters (`[0 .. 1]`) accept `"0"`, `"1"`, `"true"`, or `"false"`.
   - Verbal enumeration (`TAB_VERB`) parameters accept either a valid enum label (e.g., `"ACTIVE_ACC"`) or a valid integer key defined in the A2L `COMPU_VTAB`.
3. **A2L Physical Range Check**:
   - Verifies that every element satisfies `lower_limit <= phys_value <= upper_limit`.
4. **Raw Type Overflow Check**:
   - Applies the inverse `COMPU_METHOD` ($\text{phys} \rightarrow \text{raw}$) and verifies that the resulting raw value fits within the target integer/float type's hardware bounds.

If any check fails, the parameter's status badge immediately displays `⚠ <validation error>` and **no packet is sent to the target**.

---

## Concurrent XCP Download & Readback Verification

When validation succeeds:
1. For payloads $\le \text{MAX\_CTO} - 8$ bytes, [`XcpClient::write_memory`](../src/xcp/client.rs) sends a single atomic `SHORT_DOWNLOAD (0xED)`.
2. For larger array payloads, it sends `SET_MTA (0xF6)` followed by chunked `DOWNLOAD (0xF0)` packets.
3. When **Readback Verification** is enabled in **`⚙ Settings`** (on by default), the client immediately reads the memory back from the target and confirms byte-for-byte equality before marking the parameter `✓ Synced`.

---

## Next Steps
- [UI / UX Operator Manual →](./ui_ux_manual.md)
