#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
# Launches xcp_devtool in MCP stdio mode, rebuilding first only when sources are newer
# than the binary. All build output goes to stderr so stdout stays clean JSON-RPC.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
BIN="$ROOT/target/debug/xcp_devtool"
needs_build=0
if [ ! -x "$BIN" ]; then
  needs_build=1
elif [ -n "$(find "$ROOT/tools/xcp_devtool/src" "$ROOT/src" "$ROOT/Cargo.toml" "$ROOT/tools/xcp_devtool/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  needs_build=1
fi
if [ "$needs_build" = 1 ]; then
  cargo build -q -p xcp_devtool 1>&2
fi
exec "$BIN" mcp
