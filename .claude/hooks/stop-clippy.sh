#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
# Stop hook: when Rust sources are dirty, run clippy with warnings denied. Exit 2 asks Claude to fix.
set -u
input="$(cat)"
active="$(printf '%s' "$input" | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin); print("1" if d.get("stop_hook_active") else "0")
except Exception:
    print("0")')"
[ "$active" = "1" ] && exit 0
root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
cd "$root" || exit 0
dirty="$(git status --porcelain -- '*.rs' Cargo.toml tools/ 2>/dev/null)"
[ -z "$dirty" ] && exit 0
out="$(timeout 150 cargo clippy -p open_xcp_client --all-targets -- -D warnings 2>&1)"
code=$?
if [ "$code" -eq 124 ]; then
  echo "stop-clippy: clippy exceeded its 150 s budget; run 'cargo clippy --workspace --all-targets -- -D warnings' manually." >&2
  exit 0
fi
if [ "$code" -ne 0 ]; then
  printf '%s\n' "$out" | tail -40 >&2
  echo "stop-clippy: clippy reported errors (above). Fix them before finishing." >&2
  exit 2
fi
exit 0
