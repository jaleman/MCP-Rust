# Team Access Setup — Connecting to the KUKA MCP Server

This is for team members connecting to the already-deployed KUKA Knowledge
MCP server — not for building or running the server itself (see
[USER-MANUAL.md](USER-MANUAL.md) for that). If you just need to *ask
questions* about KUKA AMR documentation from your own AI assistant, this is
the doc you want.

Background on the deployment: [`designs/production/cloudflare-tunnel-poc.md`](designs/production/cloudflare-tunnel-poc.md)
and [USER-MANUAL.md §12](USER-MANUAL.md#12-cloudflare-tunnel-poc-access-short-term-team-deployment).

## What you need before starting

Get these three values from whoever administers the Cloudflare account, via
your team's private channel (password manager share, not email/Slack in
plaintext):

1. **Server URL**: `https://kuka-mcp.whatiskali.dev/mcp`
2. **`CF-Access-Client-Id`** header value
3. **`CF-Access-Client-Secret`** header value

These three values are the same for every team member (a shared credential,
not personal). **Never commit them to any repository, paste them into a
public chat, or check them into a config file that gets version-controlled.**
If you ever suspect they've leaked, tell the admin immediately so the token
can be rotated.

---

## Claude Desktop, claude.ai (web), and Claude mobile — currently NOT supported

**Do not spend time trying to add this server as a Custom Connector in
Claude Desktop, claude.ai web, or the Claude mobile app.** We built and
tested an OAuth-fronted path for exactly this (a Cloudflare Access "Managed
OAuth" MCP Server Portal in front of the same origin) and it fails at the
Connect step with *"Authorization with the MCP server failed"* — a known,
already-reported bug in Anthropic's `claude-ai-mcp` tracker
([issue #410](https://github.com/anthropics/claude-ai-mcp/issues/410)),
closed **"not planned."** It affects the Connectors feature across Desktop,
web, and mobile equally (they share the same connector implementation) —
this isn't a per-platform quirk.

Editing `claude_desktop_config.json` directly to add a remote `url` +
`headers` entry also does not work: that file's `mcpServers` section is for
**local (stdio) servers only** in this app; a remote entry there gets
silently rejected at startup ("not valid MCP server configurations... were
skipped").

**If you need to use this server from Claude Desktop today, install Claude
Code instead** (see below) — Claude Code ships as part of, or alongside,
Claude Desktop and uses a completely different, working configuration path.

---

## Claude Code (CLI, and the Claude Code surface inside Claude Desktop)

This is the **verified-working** path — confirmed both from a standalone
terminal and from the Claude Code panel inside the Claude Desktop app.

If the `claude` command isn't already on your machine:
```bash
npm install -g @anthropic-ai/claude-code
```

Then register the server **at user scope**, so it's available no matter
which directory you start a session from (`--scope user` is the key flag —
without it, the server is only available in the one project directory you
ran the command from):

```bash
claude mcp add --transport http kuka https://kuka-mcp.whatiskali.dev/mcp \
  --scope user \
  --header "CF-Access-Client-Id: <your Client ID>" \
  --header "CF-Access-Client-Secret: <your Client Secret>"
```

Confirm it's registered:
```bash
claude mcp list
```
Expect: `kuka: https://kuka-mcp.whatiskali.dev/mcp (HTTP) - ✔ Connected`

If you previously added it without `--scope user` (or with a placeholder
instead of your real secret), remove and re-add:
```bash
claude mcp remove kuka
```
then re-run the `add` command above.

Check `--help` on `claude mcp add` if the flag names above have drifted
from what your installed version expects.

---

## Codex CLI

Codex's MCP support and its exact config syntax for **remote/HTTP** servers
(as opposed to locally-spawned stdio servers) has been evolving quickly, so
treat the shape below as a starting point rather than gospel — confirm
against `codex mcp --help` or the current Codex docs for your installed
version before assuming it's exact.

`~/.codex/config.toml`:

```toml
[mcp_servers.kuka]
url = "https://kuka-mcp.whatiskali.dev/mcp"

[mcp_servers.kuka.headers]
CF-Access-Client-Id = "<your Client ID>"
CF-Access-Client-Secret = "<your Client Secret>"
```

If your Codex CLI version doesn't yet support HTTP-transport MCP servers
with custom headers, that's a real limitation to flag to the team rather
than something to work around with a hack.

---

## GitHub Copilot (VS Code)

VS Code's MCP support uses a `mcp.json` file — either workspace-level
(`.vscode/mcp.json`, shareable with the team since it doesn't need to
contain the actual secret — see the "inputs" note below) or in your user
settings.

```json
{
  "servers": {
    "kuka": {
      "type": "http",
      "url": "https://kuka-mcp.whatiskali.dev/mcp",
      "headers": {
        "CF-Access-Client-Id": "${input:cf-access-client-id}",
        "CF-Access-Client-Secret": "${input:cf-access-client-secret}"
      }
    }
  },
  "inputs": [
    {
      "id": "cf-access-client-id",
      "type": "promptString",
      "description": "KUKA MCP Cloudflare Access Client ID"
    },
    {
      "id": "cf-access-client-secret",
      "type": "promptString",
      "description": "KUKA MCP Cloudflare Access Client Secret",
      "password": true
    }
  ]
}
```

Using `inputs` with `"password": true` means VS Code prompts you once and
stores the secret in its own secret storage — not in the checked-in
`mcp.json` — which is why this file is safe to commit to a shared repo even
though the plain hardcoded-header version above is not. Prefer this form if
your team keeps `.vscode/` in version control.

---

## Verifying it works

Ask your assistant something like *"What are the safe minimum distances for
a KUKA KMP 1500P?"* — it should call `search_docs` and cite real KUKA
documentation back at you. If it doesn't seem to be using the tool, check
that the server shows as connected in your client's MCP server list
(`claude mcp list` for Claude Code).

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Server shows disconnected / connection refused | Typo in the URL, or the tunnel/server is temporarily down — check with the admin |
| `401`/`403` in logs, or client reports auth failure | Wrong or expired `CF-Access-Client-Id`/`Secret` — re-check the values, or ask the admin if the token was rotated |
| Client doesn't support custom headers on remote MCP servers | Some older client versions only support stdio (locally-spawned) servers — update the client, or ask the admin about alternatives |
| Tool calls succeed but return no KUKA content | Ask the admin to confirm the knowledge bundle synced correctly on the server |
| Claude Desktop/web/mobile Custom Connector fails instantly at "Connect" with "Authorization with the MCP server failed" | Known unsupported path — see the Claude Desktop section above. Use Claude Code instead |

If none of this resolves it, ask the admin to run the verification `curl`
sequence in USER-MANUAL.md §12 to confirm the server itself is healthy
before assuming the problem is client-side.
