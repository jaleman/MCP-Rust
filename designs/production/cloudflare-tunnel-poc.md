# Design: Cloudflare Tunnel POC — short-term team access

Short-term proof-of-concept for getting `mcp-server` in front of ~11 team
members without depending on the office VM (currently unreachable — see
Task "Resolve VM access for production deployment pilot", and
`designs/production/windows-vm-deployment.md`, which this POC does not
replace, only precedes). Host: the user's other computer (macOS,
always-on), which already has a Cloudflare account.

## Orientation (read first)

This is explicitly a **POC, not the hardened production deployment** —
scope stays lean (no reverse-proxy config to hand-roll, no cert management,
no firewall rules) because Cloudflare Tunnel/Access absorb that work. The
Windows-VM design doc remains the eventual target if/when VM access is
unblocked; this doc is what unblocks team access *now*.

## Goal / acceptance criteria

1. `mcp-server` runs on the Mac as a background service (launchd), survives
   reboots and restarts on crash — same bar as the VM plan, different OS
   mechanism.
2. A Cloudflare Tunnel exposes it at a Cloudflare-managed HTTPS hostname —
   no inbound firewall/port-forwarding changes on the Mac's network needed
   (tunnel is outbound-only).
3. Cloudflare Access sits in front of the tunnel hostname; requests without
   valid credentials never reach `mcp-server`.
4. Team members configure their MCP client (static `.mcp.json`-style
   config, not an interactive OAuth flow) with the tunnel URL plus a
   Cloudflare Access **Service Token** (`CF-Access-Client-Id` /
   `CF-Access-Client-Secret` headers) — no login screen, no Cloudflare
   account needed per team member.
5. Acceptance: a `curl` request without the service-token headers gets
   blocked by Cloudflare Access (never reaches `mcp-server`); the same
   request with valid headers completes the full MCP `initialize` →
   `search_docs` handshake.
6. Existing stdio/devcontainer workflow is unaffected (unchanged from the
   VM design doc — this is additive, not a replacement of local dev).

## Architecture

```
Team member's MCP client (static config: URL + service-token headers)
        │  HTTPS
        ▼
   Cloudflare Edge (Access policy check, then Tunnel)
        │  outbound-only connection, no inbound ports on the Mac
        ▼
   cloudflared (launchd service on the Mac)
        │  proxies to
        ▼
   mcp-server --http 127.0.0.1:8382     (launchd service on the Mac)
        │  reads
        ▼
   ~/kuka-mcp/knowledge/*.md            (synced from devcontainer)
```

## Components

### 1. Binary build — build natively ON the Mac, not cross-compiled

Unlike the Windows target (`x86_64-pc-windows-gnu`, well-supported by the
devcontainer's Rust toolchain via mingw), cross-compiling Rust to macOS
from Linux needs Apple's proprietary SDK (osxcross-style toolchains) — not
worth the setup for a POC. Instead:

1. Install Rust on the Mac: `curl --proto '=https' --tlsv1.2 -sSf
   https://sh.rustup.rs | sh` (same official installer either OS).
2. Clone the repo (or copy just `mcp-server/`) onto the Mac.
3. `cargo build --release --manifest-path mcp-server/Cargo.toml --bin
   mcp-server` — native build, no cross-compilation. `mcp-server` itself
   has no PDF/OCR/Office runtime dependencies (those are extract-time
   only), so nothing else needs installing on the Mac just to serve.
4. Binary lands at `mcp-server/target/release/mcp-server`.

### 2. Knowledge bundle sync (unchanged pattern, different transport)

Extraction stays in the devcontainer exactly as today. To get the bundle
onto the Mac:

```bash
rsync -avz --delete <devcontainer-or-Windows-host>:/path/to/knowledge/ user@mac-host:~/kuka-mcp/knowledge/
```

(`--delete` mirrors robocopy's `/MIR` — keeps the Mac's copy from
accumulating stale chunks after a source doc is removed/renamed.) After
syncing, call `reload_docs` against the running server instead of
restarting it — same as every other reload in this project.

### 3. `mcp-server` as a launchd service

Create `~/Library/LaunchAgents/com.kuka-mcp.server.plist` (LaunchAgent, not
LaunchDaemon, is enough if the Mac stays logged in as this user; escalate
to a LaunchDaemon only if it needs to run before any user logs in):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.kuka-mcp.server</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/<you>/kuka-mcp/bin/mcp-server</string>
    <string>--http</string>
    <string>127.0.0.1:8382</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>KUKA_KNOWLEDGE_DIR</key><string>/Users/<you>/kuka-mcp/knowledge</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/<you>/kuka-mcp/logs/mcp-server.out.log</string>
  <key>StandardErrorPath</key><string>/Users/<you>/kuka-mcp/logs/mcp-server.err.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.kuka-mcp.server.plist
```

`KeepAlive: true` restarts it on crash; `RunAtLoad` starts it at login.

### 4. Cloudflare Tunnel

```bash
brew install cloudflared
cloudflared tunnel login                       # browser auth against the Cloudflare account
cloudflared tunnel create kuka-mcp
cloudflared tunnel route dns kuka-mcp kuka-mcp.<your-domain>
```

`~/.cloudflared/config.yml`:

```yaml
tunnel: kuka-mcp
credentials-file: /Users/<you>/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: kuka-mcp.<your-domain>
    service: http://127.0.0.1:8382
  - service: http_status:404
```

```bash
cloudflared service install    # registers cloudflared itself as a launchd service
```

Requires the Cloudflare account to manage a real DNS zone (a domain) —
flagged as an open question below if that's not already the case.

### 5. Cloudflare Access

In the Cloudflare Zero Trust dashboard:

1. Add an Access application for the `kuka-mcp.<your-domain>` hostname.
2. Add a policy allowing the 11 team members — by specific email list to
   start (simplest for 11 known people); can move to a company email-domain
   rule later if that's cleaner.
3. Under **Service Auth**, create one **Service Token** per team member
   (per-person from day one, per user decision — Cloudflare supports
   multiple service tokens per Access application, so this is just N
   repetitions of the same dashboard action, not extra architecture).
   Each token is a separate `Client ID` / `Client Secret` pair, individually
   revocable from the dashboard without affecting anyone else's access —
   the real payoff of doing this from the start rather than a shared token.
4. Each team member's MCP client config gets their own two headers:
   `CF-Access-Client-Id: <their id>` and
   `CF-Access-Client-Secret: <their secret>`. Track who has which token
   (name → Client ID mapping) somewhere outside the repo — needed for
   revocation later; do not commit tokens anywhere in this project.

## Verification (exact commands)

```bash
# No service-token headers → blocked by Access before reaching mcp-server
curl -si https://kuka-mcp.<your-domain>/mcp -X POST \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
# expect: Cloudflare Access block page / 403, not an MCP response

# With service-token headers → full handshake succeeds
curl -si https://kuka-mcp.<your-domain>/mcp -X POST \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -H "CF-Access-Client-Id: <id>" -H "CF-Access-Client-Secret: <secret>" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
# expect: 200 + Mcp-Session-Id, then repeat the notifications/initialized +
# tools/call search_docs sequence from designs/step-10-streamable-http-transport.md
```

Resilience checks:
- Kill the `mcp-server` process → launchd (`KeepAlive`) restarts it;
  re-verify with the curl sequence above.
- Reboot the Mac → both launchd services (mcp-server, cloudflared) come
  back automatically (confirm `RunAtLoad`/`cloudflared service install`
  actually registered).

## Known POC-scope limitations (acceptable for now, revisit if this graduates)

- Runs on a personal Mac, not managed infrastructure — acceptable per the
  user's explicit "short-term POC" framing; the Windows-VM design remains
  the target for anything longer-lived.
- No monitoring/alerting on the launchd services beyond `KeepAlive` —
  fine for a POC; would want real observability before calling this
  production.

## Documentation (same commit/PR as any code change — house rule)

- No `mcp-server` code changes required for this doc — pure infra/config.
  If anything in `mcp-server`/`extract` needs adjusting during setup, that
  gets its own commit + doc update per the usual house rules.
- **USER-MANUAL.md**: short section noting the Cloudflare-Tunnel POC access
  path exists, once it's actually stood up and verified.
- **NOTES.md**: note the POC under "Still genuinely open" → production
  deployment, pointing at this doc, until/unless it's promoted.

## Decisions (resolved)

1. **DNS zone**: check the Cloudflare dashboard's "Websites"/"Domains" list
   — a domain with active/green status and Cloudflare nameservers is a
   usable zone. If none exists, registering or delegating one to Cloudflare
   is a prerequisite step before `cloudflared tunnel route dns` will work.
   User to confirm before the tunnel-creation step.
2. **Access policy**: explicit email allow-list, not a domain rule; list may
   exceed 11 entries — no practical limit on allow-list size for a team
   this size.
3. **Service tokens**: per-person from day one (not shared). Track the
   name → Client ID mapping outside the repo for future revocation.
