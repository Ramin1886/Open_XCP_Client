---
name: add-a2l-construct
description: Teach the open_xcp_client A2L parser a new keyword or block (BIT_MASK, BYTE_ORDER, AXIS_DESCR, TAB_INTP, ...) without breaking structure resolution. Use for any change under src/a2l.
---
# add-a2l-construct

1. Check current handling: `cargo run -q -p xcp_devtool -- a2l-inspect --builtin adas_sample` lists `unrecognized_blocks`; `src/a2l/parser.rs` is a flat token scan, so new blocks are matched by keyword wherever they appear.
2. `src/a2l/lexer.rs` only if a new token shape is needed. `src/a2l/model.rs`: add fields with `Default`-friendly types; keep `A2lModel` `Default + Clone`.
3. `src/a2l/parser.rs`: parse the block; reuse (and if needed extract) the repeated "consume `MATRIX_DIM` numbers" loop instead of copying it a seventh time. Unknown sub-blocks are skipped with `find_matching_end`, never cause a parse failure.
4. If the construct affects addresses, sizes or hierarchy, update `src/a2l/struct_resolver.rs` and keep the depth limit.
5. If the construct affects decoding (`BYTE_ORDER`, `BIT_MASK`), update `A2lDataType::decode_raw_le` callers and `compu_method.rs`; add the inverse for calibration encoding.
6. Add the construct to `src/mock_server/sample_adas_a2l.rs` so the mock, the tests and the devtool stay aligned, and to `KNOWN_BLOCKS` in `tools/xcp_devtool/src/ops/a2l.rs`.
7. Test in `tests/a2l_hierarchy_test.rs`: parse a minimal snippet, assert the model field; parse the sample and assert nothing else changed.
8. Docs: `docs/a2l_structure_mapping.md` supported-constructs list; close the `docs/TODO.md` T-14 sub-item if applicable.
9. Run the `verify-change` skill.
