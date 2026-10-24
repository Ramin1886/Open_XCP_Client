#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
# PostToolUse hook: rustfmt the edited .rs file. Never blocks (always exit 0).
set -u
input="$(cat)"
file="$(printf '%s' "$input" | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin); print(d.get("tool_input",{}).get("file_path","") or "")
except Exception:
    print("")')"
if [ -n "$file" ] && [ -f "$file" ] && [[ "$file" == *.rs ]]; then
  rustfmt --edition 2021 "$file" >/dev/null 2>&1 || true
fi
exit 0
