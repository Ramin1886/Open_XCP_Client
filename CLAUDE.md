@AGENTS.md

Claude Code specifics for this repository:
- Rules load from `.claude/rules/` (a symlink to `.agents/rules/`), skills from `.claude/skills/`.
- The `xcp_devtool` MCP server is declared in `.mcp.json`; its first start builds the tool (one to two minutes cold).
- Prefer the MCP tools over grepping: `a2l_inspect`, `a2l_find_signal`, `sdf_inspect`, `daq_plan_dryrun`, `invariants_check`, `project_check`. `mock_server` and `xcp_probe` ask for permission because they bind ports or open a target session.
- Hooks: every edited `.rs` file is formatted; destructive git and `cargo publish` commands are blocked; the Stop hook runs clippy when `.rs` files are dirty. Fix the warning rather than bypassing the hook.
- Use the `verify-change` skill before saying a change is done.
