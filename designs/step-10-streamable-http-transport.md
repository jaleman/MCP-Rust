# Design: Step 10 — Streamable-HTTP transport (Codex-ready)

Turns the stdio-only server into one that browser-based clients (claude.ai
connectors, web frontends) can reach, completing the MISSION.md "future
phase". Written by Claude for implementation by Codex (or any agent).

## Orientation (read first)

Read `AGENTS.md` and `REFACTOR-PLAN.md` (dashboard row 10) before starting.
**Preconditions:** PR #13 (step 9b) merged; confirm dashboard shows 9b
complete. Branch `refactor/step-10-http-transport` off master; PR to master;
the user merges. Ask the user before starting; flip row 10 to `in progress`
+ log entries per protocol. cargo runs ONLY in the devcontainer
(`docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server …`).

## Goal / acceptance criteria

1. `mcp-server` gains an optional HTTP mode: `mcp-server --http 127.0.0.1:8382`
   serves MCP over streamable HTTP; **no flag = stdio exactly as today**
   (existing `.mcp.json` clients must be unaffected — regression-test this).
2. One shared index across transports and sessions (the existing
   `Arc<RwLock<Index>>` — build once, share with every HTTP session).
3. Acceptance: the curl handshake below succeeds against the running
   container, and `search_docs` over HTTP returns the same hits as stdio.
4. 53+ tests pass, clippy clean, debug binary rebuilt, docs + lesson
   refactor-16 in the same commit.

## Design

### Dependencies (`mcp-server/Cargo.toml`)

- Add rmcp feature: `"transport-streamable-http-server"` (alongside the
  existing `"macros", "transport-io"`).
- Add `axum` (the rmcp streamable-http service is a tower service mounted in
  an axum router). **Match the axum major version rmcp 1.8 uses** — check
  `/usr/local/cargo/registry/src/*/rmcp-1.8.0/Cargo.toml` in the container.
- tokio already has `rt-multi-thread`; add `"net"` feature if the build asks.

### Verify the rmcp API before writing code

Consult the shipped source/examples in the container:
`ls /usr/local/cargo/registry/src/*/rmcp-1.8.0/src/transport/streamable_http_server*`
Expected shape (verify names, do not trust this doc blindly):
`StreamableHttpService::new(factory, LocalSessionManager::default().into(), StreamableHttpServerConfig::default())`
where `factory` is a closure returning a new server instance per session.

### CLI + main() (`mcp-server/src/main.rs`)

- clap is already a dependency for the extract bin; give `mcp-server` its own
  minimal `Args` with `#[arg(long)] http: Option<String>` ("listen address,
  e.g. 127.0.0.1:8382; omit for stdio"). Keep `KUKA_KNOWLEDGE_DIR` handling
  unchanged.
- Build the index ONCE (unchanged), construct one `KukaServer`, then:
  - `None` → existing `serve(stdio())` path, byte-for-byte behavior.
  - `Some(addr)` → mount the streamable-http service at `/mcp`, factory
    clones the `KukaServer` (it derives Clone; the Arc makes all clones share
    the index — `reload_docs` from any session updates all). Bind with
    `tokio::net::TcpListener`, `axum::serve`, log
    `"KUKA MCP server listening on http://{addr}/mcp"`.
- SECURITY (document in code + manual): no authentication in this step —
  bind loopback `127.0.0.1` by default and warn (tracing::warn) if the user
  binds `0.0.0.0`. Real internet exposure (claude.ai connectors need public
  HTTPS + OAuth) is explicitly out of scope; note that a tunnel
  (`cloudflared`/`ngrok`) is the interim path for trying claude.ai.

### Devcontainer

- Add `"forwardPorts": [8382]` to `.devcontainer/devcontainer.json`.

## Verification (exact commands)

Start: `docker exec -d -w /workspaces/MCP-Rust kuka-mcp-server ./mcp-server/target/debug/mcp-server --http 127.0.0.1:8382`

Handshake (run inside the container; note the session id returned in the
`Mcp-Session-Id` response header of the initialize call — pass it back on
every subsequent request):

```bash
curl -si -X POST http://127.0.0.1:8382/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
# → grab Mcp-Session-Id: <SID> from the response headers, then:
curl -s -X POST http://127.0.0.1:8382/mcp -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' -H 'Mcp-Session-Id: <SID>' \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
curl -s -X POST http://127.0.0.1:8382/mcp -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' -H 'Mcp-Session-Id: <SID>' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_docs","arguments":{"query":"mission status payload"}}}'
```

Expect the same "Found N result(s)" text as stdio. Then the stdio
regression: the existing printf-pipe smoke test (see progress log entries
for the exact lines) must still work with no flag.

## Tests

- Unit: args parsing (http flag optional, default None).
- The HTTP loop itself is verified by the curl sequence (manual); don't
  build a heavyweight integration harness for this step.

## Documentation (same commit — house rule)

- **USER-MANUAL.md**: new section "Serving over HTTP (browser and remote
  clients)" — when to use it, the `--http` flag, the security caveats above,
  claude.ai/tunnel note; update §1 capabilities and §6 (connection methods).
- **REFACTOR-PLAN.md**: row 10 + progress-log entries.
- **Lesson `lessons/refactor-16-streamable-http.html`** per the standing
  template (Java-vs-Rust: axum vs embedded Jetty/Spring Boot, tower services
  vs servlet filters, one binary serving two transports, session managers).
