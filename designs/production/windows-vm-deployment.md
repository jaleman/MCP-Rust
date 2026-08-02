# Design: Production deployment — internal team pilot on a Windows VM

> **STATUS: paused — VM unreachable.** Neither RDP nor SSH from the user's
> home computer currently reaches the target VM (see the "Resolve VM access
> for production deployment pilot" task). Short-term team access is
> proceeding instead via `designs/production/cloudflare-tunnel-poc.md`
> (Cloudflare Tunnel + Access on a different, reachable machine). This doc
> remains the target design to return to once VM access is unblocked — not
> abandoned, just not the active path right now.

Turns the server from "something Claude runs inside a devcontainer for one
person" into an always-on service an internal team reaches over the office
network / VPN. Scope is deliberately a **pilot for a trusted internal team**,
not a public claude.ai connector — see "Explicitly out of scope" below.

## Orientation (read first)

Read `REFACTOR-PLAN.md` (all 14 numbered steps are complete — this is a new,
separate track, not a numbered plan step) and `NOTES.md` ("Still genuinely
open" → "Production deployment / public exposure", which this doc resolves)
and `designs/step-10-streamable-http-transport.md` (the HTTP transport this
deployment builds on — read it for the existing `--http` flag's behavior and
its explicit "no auth, loopback-only" security note, which this doc replaces
with real auth).

**Preconditions:** none blocking — step 10 (streamable HTTP) is on master.
Ask the user before starting; this touches a new `designs/production/`
tree rather than the numbered dashboard, so use ordinary commit/PR hygiene
but there is no dashboard row to flip.

## Goal / acceptance criteria

1. `mcp-server` runs unattended on the Windows VM, survives reboots, and
   restarts itself if it crashes.
2. Team members on the office network or VPN reach it over HTTPS with a
   shared API key; the server is not reachable without one.
3. The knowledge bundle on the VM can be refreshed from a fresh extraction
   without redeploying the binary or touching the service config.
4. A team member's MCP client (Claude Desktop / Code, or any streamable-HTTP
   MCP client) can complete the full `initialize` → `search_docs` handshake
   against the VM's public URL, and a request with a missing/wrong API key
   is rejected before it reaches the MCP server.
5. Existing stdio/devcontainer workflow (extraction, local dev) is
   completely unaffected — this is additive.

## Architecture

```
Team member's MCP client
        │  HTTPS + X-Api-Key header
        ▼
   Caddy (reverse proxy, TLS termination, API-key auth)
        │  HTTP, loopback only (127.0.0.1:8382)
        ▼
   mcp-server.exe --http 127.0.0.1:8382     (Windows Service, NSSM)
        │  reads
        ▼
   C:\kuka-mcp\knowledge\*.md               (synced from devcontainer)
```

Two Windows Services, both auto-start/auto-restart:
- `kuka-mcp-server` — wraps `mcp-server.exe --http 127.0.0.1:8382`.
- Caddy running as a service (Caddy ships its own Windows service support;
  no NSSM needed for Caddy itself) fronting it on `:443`.

## Components

### 1. Binary + knowledge bundle placement

- `C:\kuka-mcp\bin\mcp-server.exe` — native Windows build. Confirmed
  buildable natively (the user already produced `extract.exe` this way);
  build with `cargo build --release --manifest-path mcp-server\Cargo.toml
  --bin mcp-server` from a machine with the Rust toolchain, or copy the
  `x86_64-pc-windows-gnu` release artifact used for `extract.exe`.
  **mcp-server itself needs no runtime dependencies** (no pdftotext/
  ocrmypdf/soffice — those are extract-time only), so the VM does not need
  the toolchain installed just to serve.
- `C:\kuka-mcp\knowledge\` — the bundle directory. Set
  `KUKA_KNOWLEDGE_DIR=C:\kuka-mcp\knowledge` as a system environment
  variable (or in the NSSM service definition) so it doesn't depend on the
  service's working directory.

### 2. Knowledge sync (devcontainer → VM)

Extraction stays exactly where it is today (devcontainer, `extract` binary,
full OCR/LibreOffice support) — do not duplicate that toolchain on the VM.
After a re-extraction:

1. Copy `knowledge/` from the dev machine to `C:\kuka-mcp\knowledge\` on the
   VM (robocopy over the VPN, or any existing file-sync path already used
   to reach the VM — a `/mirror` robocopy so deleted/renamed source docs
   don't leave orphaned stale chunks: `robocopy <src>\knowledge
   C:\kuka-mcp\knowledge /MIR`).
2. Call `reload_docs` against the VM's running server (through the same
   auth as any other client) instead of restarting the service — this is
   exactly what `reload_docs` was built for in step 5b.

This keeps a clean separation: devcontainer = build/extract, VM = serve
only. No new code needed for this step, just a documented runbook (below).

### 3. Windows Service wrapper (NSSM)

`mcp-server.exe` has no built-in Windows-service mode, so wrap it with
[NSSM](https://nssm.cc/) (or `sc.exe create` with `srvany`, but NSSM is far
less fiddly for restart-on-crash and log redirection):

```powershell
nssm install kuka-mcp-server "C:\kuka-mcp\bin\mcp-server.exe" --http 127.0.0.1:8382
nssm set kuka-mcp-server AppEnvironmentExtra KUKA_KNOWLEDGE_DIR=C:\kuka-mcp\knowledge
nssm set kuka-mcp-server AppStdout C:\kuka-mcp\logs\mcp-server.out.log
nssm set kuka-mcp-server AppStderr C:\kuka-mcp\logs\mcp-server.err.log
nssm set kuka-mcp-server Start SERVICE_AUTO_START
nssm set kuka-mcp-server AppExit Default Restart
nssm start kuka-mcp-server
```

### 4. Reverse proxy + TLS (Caddy)

Caddy handles both TLS termination and the API-key check in one config
block — no custom code needed for auth if Caddy's own directives suffice
(see below for the fallback if they don't).

`Caddyfile`:

```
kuka-mcp.internal.example.com {
    @missing_key not header X-Api-Key {$KUKA_MCP_API_KEY}
    respond @missing_key "unauthorized" 401

    reverse_proxy 127.0.0.1:8382
}
```

- Cert: if the office already has an internal CA, get a cert issued for the
  hostname and point Caddy at it (`tls cert.pem key.pem`); otherwise Caddy
  can self-sign (`tls internal`) and you distribute/trust that root once per
  team machine. Either way, avoid Caddy's automatic public-ACME issuance —
  this hostname is not publicly resolvable, so it can't complete an HTTP-01
  challenge.
- `X-Api-Key` compared via Caddy's `header` matcher against an env var
  (`$KUKA_MCP_API_KEY`, set as a system environment variable, not
  hardcoded in the Caddyfile — keeps the key out of version control if the
  Caddyfile is ever checked in).
- Run Caddy as a Windows service (`caddy.exe run --config Caddyfile
  --environ` registered via NSSM the same way, or Caddy's own
  `caddy windows-service` helper if using the installer package).

**Fallback if Caddy's header-matcher auth proves too coarse** (e.g. you
later want per-user keys instead of one shared key): move the check into
`mcp-server` itself as a small axum middleware layer guarding the `/mcp`
route, reading acceptable keys from `KUKA_MCP_API_KEYS` (comma-separated).
This is a real code change (unlike the Caddy-only path) — do it as a
follow-up design doc if/when per-user keys become a real requirement, not
speculatively now.

### 5. Firewall

- Windows Firewall: allow inbound `443` (Caddy) only. Do **not** open
  `8382` — it's loopback-only by design (`mcp-server --http 127.0.0.1:...`
  never listens on `0.0.0.0`; the existing step-10 code already
  `tracing::warn`s if it's ever bound non-loopback, which should never
  happen in this deployment).

## Explicitly out of scope (per NOTES.md, unchanged)

- Public internet / claude.ai connector exposure — that needs public HTTPS
  from a real public hostname plus OAuth per Anthropic's connector spec, a
  materially bigger scope than an internal pilot. Revisit only if this pilot
  succeeds and broader/public rollout is actually requested.
- Per-user authentication/identity or audit trail — the shared API key gives
  the team a single trust boundary, not per-user accountability. Acceptable
  for a small trusted internal team; flag to the user if requirements
  change.
- `tools/list_changed` notifications (separate NOTES.md item, unrelated to
  deployment).

## Verification (exact commands)

From a machine outside the VM, on the office network/VPN:

```powershell
# Missing key → 401
curl.exe -si https://kuka-mcp.internal.example.com/mcp -X POST `
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" `
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
# expect: HTTP/1.1 401

# Valid key → real MCP handshake, mirrors the step-10 curl sequence
curl.exe -si https://kuka-mcp.internal.example.com/mcp -X POST `
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" `
  -H "X-Api-Key: <the real key>" `
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
# expect: 200 + Mcp-Session-Id header, then repeat the notifications/initialized
# + tools/call search_docs sequence from designs/step-10-streamable-http-transport.md
```

Service resilience checks:
- `Restart-Service kuka-mcp-server` (or kill the process) → NSSM restarts it
  automatically; confirm via the handshake above.
- Reboot the VM → both services come back without manual intervention.

Knowledge-sync check:
- Re-extract a doc on the devcontainer, robocopy to the VM, call
  `reload_docs` over HTTPS with the API key, confirm the doc count/term
  count in the response changed and a query for new content succeeds —
  without touching either Windows service.

## Documentation (same commit/PR as any code change — house rule)

- **USER-MANUAL.md**: new section "Production deployment (internal team
  pilot)" summarizing this doc's runbook — VM layout, service names, sync
  process, how to rotate the API key.
- **NOTES.md**: move "Production deployment / public exposure" from "Still
  genuinely open" to resolved, with a pointer to this design doc.
- No REFACTOR-PLAN.md dashboard row — this is infrastructure/ops, not a
  numbered code-refactor step; log it in NOTES.md instead.

## Open questions for the user before execution

1. Real hostname for the VM (internal DNS entry, or is this IP-only for
   now)?
2. Does the office have an internal CA to issue a proper cert, or is a
   self-signed/Caddy-internal cert (trusted manually on team machines)
   acceptable for the pilot?
3. Who holds/rotates the shared API key, and how is it distributed to the
   team (password manager, etc.) — out of scope for the doc itself but
   needs an answer before go-live.
