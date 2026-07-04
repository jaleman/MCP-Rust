# Copilot Extensions Run Inside the Dev Container

VS Code extensions (including GitHub Copilot) run inside the dev container, not on the Windows host. The Windows side only renders the VS Code UI. This means Copilot can launch the MCP server binary directly using its Linux path — no `docker exec` needed.

Correct `.vscode/mcp.json`:
```json
{ "servers": { "kuka": { "command": "/workspaces/MCP-Rust/mcp-server/target/debug/mcp-server" } } }
```

Also established: Copilot can simulate tool responses by reading source code without invoking the binary. A real MCP call shows an explicit "Ran ping — kuka (MCP Server)" tool-use block. Without that block, the response is fabricated from source.

**Implications**: Any future MCP config for this project uses the direct Linux binary path. Never use `docker exec` from within the container — it would require Docker-in-Docker. The simulation vs real-call distinction should be kept in mind when verifying tool behaviour.
