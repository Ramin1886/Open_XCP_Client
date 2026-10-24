---
name: add-xcp-command
description: Add or change an XCP CTO command end to end in open_xcp_client - packet builder, client method, mock-server handler, test, docs. Use for any new GET_/SET_/ALLOC_/DAQ command or response decoder.
---
# add-xcp-command

Needs human approval first (`AGENTS.md` §6). Then:

1. `src/xcp/packet.rs`: add the `XcpCommand` variant and a `cto::<name>()` builder with a `///` doc naming the PID (e.g. `0xFD GET_STATUS`) and the request layout. Little-endian multi-byte fields via `to_le_bytes()`.
2. `src/xcp/client.rs`: add a method on `XcpClient` that calls `transport.send_command(&cto::<name>(..), self.timeout)`, bounds-checks the response (`res.get(i)`), decodes into a small typed struct, and maps failures to `XcpError`. No `unwrap`.
3. `src/mock_server/mod.rs`: handle the PID with a positive response and at least one `ERR_` path (unknown mode, out of range). Keep `MAX_CTO` honoured.
4. Test in `tests/end_to_end_xcp_test.rs` or `tests/xcp_protocol_and_clock_test.rs` against `MockXcpServer::start(0)`: builder bytes, decode of a good response, decode of a short response, the error path. Poll with a deadline.
5. If the command changes DAQ behaviour, re-run `tests/daq_odt_limits_test.rs`, `tests/daq_loss_detection_test.rs` and `cargo run -q -p xcp_devtool -- daq-plan-dryrun --builtin adas_sample --max-dto 8` … `1024`.
6. Docs: add the command row to `docs/xcp_protocol_and_xcplite.md`; if it closes or changes a `docs/TODO.md` item, update it.
7. Sanity: start `cargo run -q -p xcp_devtool -- mock-server --port 0`, then `cargo run -q -p xcp_devtool -- xcp-probe --port <udp port>` must still be `ok: true`.
8. Run the `verify-change` skill.
