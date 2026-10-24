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
5. If the user specifies a **Local A2L Fallback Path** in the **`⚙ Settings`** menu, the client reads that `.a2l` file **instead of** uploading from the target (the local file wins even when the upload would succeed; `TODO.md` T-08 tracks making it a true fallback). The client never requests `IDT_ASAM_NAME`.

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

DAQ is configured once, on connect, and runs until disconnect. Starting or stopping a recording
is purely host-side and never re-allocates DAQ — re-running the allocation would tear down a
working configuration, and some slaves reject the re-allocation outright.

### ODT packing limits

`ALLOC_ODT`, `ALLOC_ODT_ENTRY` and `SET_DAQ_PTR` each carry the ODT number and the ODT entry
number in a **single byte**, so an ODT holds at most `MAX_ODT_ENTRIES = 255` entries and a DAQ
list at most `MAX_ODTS_PER_DAQ_LIST = 255` ODTs
([`src/xcp/daq_planner.rs`](../src/xcp/daq_planner.rs)). An ODT is therefore closed when *either*
the `MAX_DTO` payload is full *or* it already holds 255 entries, and the first ODT of each list
reserves 4 bytes for the DAQ timestamp the slave prepends to it.

This matters on targets with many small measurements: 1000 one-byte signals fit a 1018-byte
`MAX_DTO` by size but need four ODTs by entry count. Packing them into one ODT makes the entry
count wrap in `ALLOC_ODT_ENTRY` and the slave answers `ERR_MEMORY_OVERFLOW (0x22)`, so DAQ never
starts and no measurement data arrives.

Exactly one DAQ list is planned per event channel, which keeps one `.sdf` timestamp row per event
firing. On a target with a small `MAX_DTO` and a very large event, the 255-ODT ceiling can be
reached; the measurements beyond it are then reported through
`DaqPlanner::dropped_measurement_indices` and named in the status bar rather than being folded
into a malformed allocation.

### Data-integrity accounting

A dropped UDP datagram is otherwise invisible: the frame simply never arrives, and nothing
downstream can tell "the target sent nothing" from "the network ate it". Two mechanisms close that.

**Link level — the 16-bit `CTR`.** Every XCP-on-Ethernet frame carries a `CTR` that the slave
increments per transmitted frame. [`LinkStats`](../src/xcp/link_stats.rs) tracks the expected
successor and classifies each arrival as in-order, a gap (with the implied number of lost frames),
a duplicate or a reorder, counting each and bumping a `generation` on any discontinuity. Wrapping
arithmetic makes the `0xFFFF` → `0x0000` rollover an ordinary step, never loss. The counter is
observed in [`transport.rs`](../src/xcp/transport.rs) over **every** received frame, CTO responses
included — the `CTR` is shared across the connection, so sampling it on DTOs alone would score each
command response as a lost datagram. DTOs the bounded DAQ queue cannot accept are counted too,
rather than dropped silently.

**Cycle level — ODT 0 anchors a firing.** One firing of a DAQ list's event arrives as ODTs 0..n.
ODT 0 carries the DAQ timestamp, which makes it the cycle delimiter:
[`process_dto_packet`](../src/xcp/daq_planner.rs) lets **only ODT 0 open a cycle** and emits a
sample only once every planned ODT has arrived exactly once, in increasing order, within that
cycle and without an intervening link discontinuity.

All three conditions are needed. A seen-ODT mask reset on ODT 0 is not enough on its own, and
neither is adding strictly increasing ODT numbers: on a two-ODT list, losing ODT 1 of cycle `N` and
then ODT 0 of cycle `N+1` leaves cycle `N` still open, and ODT 1 of `N+1` follows on with a
perfectly valid ODT number, so a sample stitched from two different firings would be emitted as
complete. What rejects it is that both lost datagrams consumed `CTR` values, so the link generation
moved between the cycle's header and the arriving ODT — non-header ODTs carry no cycle identity of
their own, so an ODT arriving after a discontinuity cannot be proven to belong to the open cycle.

**Incomplete cycles are discarded, never flagged.** A torn sample has no valid physical
interpretation, and a flag only protects consumers that check it. Nothing incomplete reaches the
`.sdf` datasets; what is preserved is the *fact* of the loss, as per-event counters
(`cycles_discarded`, `cycles_lost_no_header`, `odt_sequence_errors`) and gap records in
`SdfFileMetadata.integrity` / `.gaps`. A discarded cycle has no trustworthy target timestamp —
that is exactly what is lost when ODT 0 goes missing — so a gap record carries the host arrival
time. The effect is that a lossy link lowers the effective sample rate and leaves an auditable
hole, instead of silently corrupting physical values.

The `XCP Info` tab shows all of it: a **Link & DAQ Integrity** panel for the link counters and
recent discards, three per-event columns (`Discarded`, `No ODT 0`, `ODT Seq Err`), and a measured
cycle time flagged when it deviates from the A2L value by more than 25%.

### Diagnosing "connected but no data"

A DAQ setup that the slave rejects is reported in the status bar immediately after connect (and
after the `Connected` event, so the connect message cannot overwrite it), so
`Connected — Live DAQ streaming` never stands alone when the allocation actually failed. If DAQ
starts cleanly and still no DTO arrives, the target application is not triggering its XCP events
— check that the target's `xcplite` integration actually registers and fires them (a target that
logs `No xcp_evts section found` at startup serves its A2L and answers every CTO command, but
never produces DAQ data).

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
