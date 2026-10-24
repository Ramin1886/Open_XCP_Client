#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
# PreToolUse hook for Bash: block destructive git/cargo commands. Exit 2 blocks the call.
set -u
input="$(cat)"
cmd="$(printf '%s' "$input" | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin); print(d.get("tool_input",{}).get("command","") or "")
except Exception:
    print("")')"
block() { echo "blocked by .claude/hooks/guard-bash.sh: $1 (see AGENTS.md section 9)" >&2; exit 2; }
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+push[^|;&]*([[:space:]]--force([[:space:]]|$)|[[:space:]]-f([[:space:]]|$)|--force-with-lease)'; then block "force push"; fi
if printf '%s' "$cmd" | grep -Eq 'rm[[:space:]]+(-[a-zA-Z]*r[a-zA-Z]*f|-[a-zA-Z]*f[a-zA-Z]*r)[a-zA-Z]*[[:space:]]+("?(/|~|~/|\.|\.\.)"?([[:space:]]|$)|\*|\$|/[a-zA-Z]+([[:space:]]|$))'; then block "recursive delete of a root, home, cwd, glob, variable or top-level path"; fi
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+reset[[:space:]]+--hard'; then block "git reset --hard"; fi
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+checkout[[:space:]]+--[[:space:]]+\.'; then block "git checkout -- . discards all changes"; fi
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+clean[[:space:]]+-[a-zA-Z]*f'; then block "git clean -f"; fi
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+branch[[:space:]]+-D'; then block "git branch -D"; fi
if printf '%s' "$cmd" | grep -Eq 'git[[:space:]]+(tag[[:space:]]+v|push[^|;&]*--tags)'; then block "tagging/pushing tags triggers a release; ask the human"; fi
if printf '%s' "$cmd" | grep -Eq 'cargo[[:space:]]+publish'; then block "cargo publish"; fi
exit 0
