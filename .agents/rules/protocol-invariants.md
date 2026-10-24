---
trigger: glob
description: XCP protocol and DAQ planner invariants that must survive every change under src/xcp
globs: "src/xcp/**"
paths:
  - "src/xcp/**"
---
# XCP protocol invariants

- `MAX_ODT_ENTRIES` and `MAX_ODTS_PER_DAQ_LIST` (`src/xcp/daq_planner.rs`) exist because ODT and entry numbers are single bytes in `ALLOC_ODT`, `ALLOC_ODT_ENTRY` and `SET_DAQ_PTR`. Never exceed them; measurements that do not fit are dropped and reported through `dropped_measurement_indices`, never truncated.
- Usable ODT numbers must stay clear of reserved packet IDs 0xFC–0xFF (`invariants-check reserved_pid_limit` reports the current state).
- ODT 0 of every DAQ list reserves the 4 timestamp bytes; the payload budget accounts for it.
- A `DaqEventSample` is emitted only for a complete, in-order cycle. A lost ODT 0 discards the cycle without touching staging. Discards are counted with a `CycleDiscardReason`.
- CTR accounting (`src/xcp/link_stats.rs`): 16-bit wraparound, gap/reorder/duplicate classification, a duplicate never rewinds the expectation. Transport queue overflows are counted, not hidden.
- Every new CTO command: builder in `packet.rs` with a doc comment naming the XCP PID, client method with the configured timeout, mock-server handler including one error path, a test against `MockXcpServer::start(0)`, and a row in `docs/xcp_protocol_and_xcplite.md`.
- Hard-coded assumptions (little-endian, 4-byte ns timestamps, absolute ODT numbering, DTO ID = `[odt, daq]`) are known debt (`docs/TODO.md` T-09). Do not add new ones; when you touch one, make it a negotiated or validated value.
- `xcplite` 2.2.2 is the reference target. Deviations from the XCP spec are documented, never silent.
