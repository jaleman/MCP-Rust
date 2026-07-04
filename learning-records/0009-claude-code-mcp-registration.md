# Claude Code Uses `claude mcp add`, Not `.mcp.json`

`.mcp.json` is project-scoped and only loads when its folder is part of the active session. In the Claude Code desktop app this requires manually adding the folder each session — fragile and easily lost.

The correct approach for Claude Code is `claude mcp add`, which registers the server at the user level:

```
claude mcp add kuka -- docker exec -i kuka-mcp-server /workspaces/MCP-Rust/mcp-server/target/debug/mcp-server
```

This persists across all sessions automatically. Verify with `claude mcp list`. Remove with `claude mcp remove kuka`.

**Implications**: Never rely on `.mcp.json` for Claude Code session availability. Use `claude mcp add` for any server that should be globally accessible. The `.mcp.json` file in the project is kept as documentation but is not the active registration mechanism.
