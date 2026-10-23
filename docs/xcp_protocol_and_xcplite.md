# XCP on Ethernet Protocol & Vector `xcplite` 2.2.2 Integration

[← Back to Documentation Index](./index.md)

## XCP on Ethernet Framing

Implemented in [`src/xcp/packet.rs`](../src/xcp/packet.rs), every XCP packet over UDP or TCP uses the standard 4-byte ASAM XCP-on-Ethernet header:

| Byte Offset | Type | Field | Description |
| :--- | :--- | :--- | :--- |
| `0..2` | `u16 LE` | `len` | Length of the XCP payload in bytes |
| `2..4` | `u16 LE` | `ctr` | Monotonically incrementing packet counter |
| `4..4+len` | `[u8]` | `payload` | XCP CTO or DTO payload |

Because `xcplite` 2.2.2 can pack multiple XCP frames into a single UDP datagram or TCP segment, [`unpack_xcp_ethernet_frames`](../src/xcp/packet.rs) iteratively parses concatenated frames from the receive buffer.

---

## Automatic A2L Retrieval from Running Software

When the embedded ADAS software initializes on QNX 7.1, `xcplite` 2.2.2 generates the `.a2l` description and exposes it over XCP via `CC_GET_ID (0xFA)`:

1. [`XcpClient::connect`](../src/xcp/client.rs) sends `CONNECT (0xFF, 0x00)` and negotiates `MAX_CTO` and `MAX_DTO`.
2. [`XcpClient::fetch_a2l`](../src/xcp/client.rs) sends `GET_ID (0xFA, IDT_ASAM_UPLOAD = 4)`.
3. `xcplite` 2.2.2 sets its internal Memory Transfer Address (MTA) to the generated A2L buffer and returns the total byte length in `res[3..7]`.
4. The client streams the entire A2L content using chunked `UPLOAD (0xF5)` commands and feeds it directly to [`parse_a2l`](../src/a2l/parser.rs).
5. If a target is configured to return only a filename (`IDT_ASAM_NAME = 1`) or if the user specifies a **Local A2L Fallback Path** in the **`⚙ Settings`** menu, the client seamlessly loads the fallback `.a2l` file.

---

## Multi-Rate DAQ Configuration

Implemented in [`src/xcp/daq_planner.rs`](../src/xcp/daq_planner.rs) and [`XcpClient::configure_and_start_daq`](../src/xcp/client.rs):

1. All resolved leaf measurements are grouped by their A2L `IF_DATA XCP` `EVENT` channel ID.
2. One dynamic DAQ list is allocated per active event channel via:
   - `FREE_DAQ (0xD6)`
   - `ALLOC_DAQ (0xD5)`
   - `ALLOC_ODT (0xD4)`
   - `ALLOC_ODT_ENTRY (0xD3)`
3. Each measurement's memory address (`ECU_ADDRESS`), extension (`ECU_ADDRESS_EXTENSION`), and byte size are written into ODT entries via `SET_DAQ_PTR (0xE2)` + `WRITE_DAQ (0xE1)`.
4. Each DAQ list is bound to its A2L `EVENT` channel with timestamping enabled (`SET_DAQ_LIST_MODE (0xE0, mode = 0x10)`), selected via `START_STOP_DAQ_LIST (0xDE, mode = 0x02)`, and started synchronously via `START_STOP_SYNCH (0xDD, mode = 0x01)`.

---

## Target Clock Correlation to UTC (`src/xcp/clock_sync.rs`)

Implemented in [`TargetClockSync`](../src/xcp/clock_sync.rs):
- On connect, the client issues `GET_DAQ_CLOCK (0xDC)` to read the target's 64-bit or 32-bit DAQ clock.
- If the target `xcplite` clock is synchronized via PTP / `CLOCK_REALTIME` (reporting nanoseconds since Unix epoch `>= 2020-01-01`), the target's native UTC clock is used directly.
- If the target reports `CLOCK_MONOTONIC` nanoseconds since boot, `TargetClockSync` anchors the target counter against the laptop's UTC clock at session start and tracks 32-bit DTO timestamp rollovers (`~4.29 s` period at `1 ns` resolution) so every recorded sample receives a jitter-free UTC timestamp (`time_utc_s` and `time_utc_ns`).

---

## Next Steps
- [A2L Parsing & Complex C/C++ Structure Mapping →](./a2l_structure_mapping.md)
- [Live Calibration Guide →](./calibration_guide.md)
