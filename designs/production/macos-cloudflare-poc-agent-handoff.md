# Agent Handoff: macOS Cloudflare Tunnel POC

Use this document to start the short-term team-access deployment on the
always-on Mac. The authoritative infrastructure design is
[Cloudflare Tunnel POC](cloudflare-tunnel-poc.md); follow it in full after
the host-preparation phase below. This handoff does not replace that design.

## Outcome

Run the KUKA Knowledge MCP server on the Mac as a loopback-only launchd
service, expose it through a Cloudflare Tunnel, and protect the public
hostname with Cloudflare Access service tokens. The initial server uses the
knowledge bundle committed to this repository; subsequent content updates
can be synced from the development environment and reloaded without a server
restart.

## Guardrails

- This is a short-term POC. Do not present it as the permanent production
  deployment; [windows-vm-deployment.md](windows-vm-deployment.md) remains
  the intended long-term design.
- Do not bind `mcp-server` to `0.0.0.0`, open an inbound firewall port, or
  add port forwarding. It must listen only on `127.0.0.1:8382`.
- Do not commit Cloudflare service-token IDs, secrets, tunnel credentials, or
  private host details. Give each team member a distinct service token.
- Do not change server code merely to deploy it. Record any genuine setup
  problem and make code changes separately with tests and documentation.
- Stop before tunnel creation if the Cloudflare account has no active DNS
  zone. A Cloudflare-managed domain is a prerequisite.

## Phase 1: Prepare the Mac

Work on the Mac under the account that will remain logged in. A
`LaunchAgent` runs only while that user is logged in; use a `LaunchDaemon`
only if the owner explicitly requires operation before user login.

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new terminal after Rust installation, then clone and build the
repository natively:

```bash
git clone https://github.com/jaleman/MCP-Rust.git ~/kuka-mcp/repo
cd ~/kuka-mcp/repo
cargo build --release --manifest-path mcp-server/Cargo.toml --bin mcp-server
mkdir -p ~/kuka-mcp/bin ~/kuka-mcp/logs
install -m 755 mcp-server/target/release/mcp-server ~/kuka-mcp/bin/mcp-server
```

Initialize the served bundle from the checked-out repository:

```bash
rsync -a --delete ~/kuka-mcp/repo/knowledge/ ~/kuka-mcp/knowledge/
test -n "$(find ~/kuka-mcp/knowledge -maxdepth 1 -name '*.md' -print -quit)"
```

The last command must succeed before continuing. Extraction dependencies such
as `pdftotext`, OCR, and LibreOffice are not needed to serve this already
generated bundle. Install them only if this Mac will itself extract source
documents.

## Phase 2: Prove the Local Server

1. Follow component 3 of [Cloudflare Tunnel POC](cloudflare-tunnel-poc.md)
   to create `~/Library/LaunchAgents/com.kuka-mcp.server.plist`.
2. Replace `<you>` in every plist path with the Mac account name. The binary
   path must be `/Users/<you>/kuka-mcp/bin/mcp-server`; the knowledge path
   must be `/Users/<you>/kuka-mcp/knowledge`.
3. Load the service with the command in the POC, then inspect its log files
   under `~/kuka-mcp/logs/` if it does not start.
4. Confirm the listener is local-only before exposing it:

```bash
curl -i http://127.0.0.1:8382/mcp
```

An HTTP response confirms the process is reachable. Do not treat a bare GET
as an MCP protocol test; the authenticated `initialize` request in the POC
is the real end-to-end verification.

## Phase 3: Configure Cloudflare

Proceed through components 4 and 5 of
[Cloudflare Tunnel POC](cloudflare-tunnel-poc.md), in this order:

1. Confirm an active DNS zone in the Cloudflare account.
2. Install `cloudflared`, authenticate in the browser, create `kuka-mcp`,
   and route `kuka-mcp.<your-domain>`.
3. Create `~/.cloudflared/config.yml` exactly as specified in the POC, with
   its upstream set to `http://127.0.0.1:8382`.
4. Register `cloudflared` as its launchd service.
5. In Zero Trust, create the Access application and explicit email allow
   policy, then create one service token for each team member.
6. Store the mapping from team member to Client ID outside the repository.
   Deliver each secret through an approved private channel only.

## Phase 4: Acceptance and Handoff

Run the two exact `curl` commands in the POC's **Verification** section:

1. Without service-token headers, the request must be blocked by Cloudflare
   Access before it reaches MCP.
2. With a valid person's `CF-Access-Client-Id` and
   `CF-Access-Client-Secret`, `initialize` must return `200` and an
   `Mcp-Session-Id`.
3. Complete the `notifications/initialized` and `tools/call` `search_docs`
   sequence linked from the POC to prove a real authenticated search.
4. Kill the server process and verify launchd restarts it. Reboot the Mac and
   repeat the authenticated handshake to confirm both services return.

For each team member, provide the tunnel URL and their own two header values
for their static MCP-client configuration. Do not use an interactive browser
login as the client authentication path.

## Ongoing Content Updates

The Git checkout supplies the initial bundle. When the development source
bundle changes, sync it to the served directory using the POC's `rsync -avz
--delete` pattern, then call `reload_docs` against the running MCP server.
Do not restart the server just to refresh documents. Keep deployment notes
and secrets outside the repository; update `USER-MANUAL.md` and `NOTES.md`
only after the POC is actually live and verified, as required by the POC.