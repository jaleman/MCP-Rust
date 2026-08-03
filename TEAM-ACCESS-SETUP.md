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

## Claude Desktop

Edit your `claude_desktop_config.json`:
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "kuka": {
      "url": "https://kuka-mcp.whatiskali.dev/mcp",
      "headers": {
        "CF-Access-Client-Id": "<your Client ID>",
        "CF-Access-Client-Secret": "<your Client Secret>"
      }
    }
  }
}
```

Restart Claude Desktop after saving. The server should appear in the MCP
server list (hammer/plug icon in the chat input).

---

## Claude Code (CLI)

Either the one-line command:

```bash
claude mcp add --transport http kuka https://kuka-mcp.whatiskali.dev/mcp \
  --header "CF-Access-Client-Id: <your Client ID>" \
  --header "CF-Access-Client-Secret: <your Client Secret>"
```

...or add it directly to a project's `.mcp.json` (or your user-level MCP
config) if you'd rather have it in every session for that project:

```json
{
  "mcpServers": {
    "kuka": {
      "type": "http",
      "url": "https://kuka-mcp.whatiskali.dev/mcp",
      "headers": {
        "CF-Access-Client-Id": "<your Client ID>",
        "CF-Access-Client-Secret": "<your Client Secret>"
      }
    }
  }
}
```

Run `claude mcp list` to confirm it's registered, and check `--help` on
`claude mcp add` if the flag names above have drifted from what your
installed version expects.

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
that the server shows as connected in your client's MCP server list.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Server shows disconnected / connection refused | Typo in the URL, or the tunnel/server is temporarily down — check with the admin |
| `401`/`403` in logs, or client reports auth failure | Wrong or expired `CF-Access-Client-Id`/`Secret` — re-check the values, or ask the admin if the token was rotated |
| Client doesn't support custom headers on remote MCP servers | Some older client versions only support stdio (locally-spawned) servers — update the client, or ask the admin about alternatives |
| Tool calls succeed but return no KUKA content | Ask the admin to confirm the knowledge bundle synced correctly on the server |

If none of this resolves it, ask the admin to run the verification `curl`
sequence in USER-MANUAL.md §12 to confirm the server itself is healthy
before assuming the problem is client-side.
