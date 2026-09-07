# Migration & Deployment Runbook: Windows VM / PC Host

This document provides complete instructions for exporting, packaging, and deploying the **KUKA Knowledge MCP Server** to a Windows 11 PC or Windows Server VM on the internal corporate network/VPN, as well as configuring AI agent harnesses to connect to it.

---

## 1. Overview & Architecture

When deployed on a corporate Windows host, the MCP server runs as an always-on background Windows Service listening for HTTP/HTTPS requests on the corporate network or VPN.

```
Client Agent Harnesses (Claude Code, Claude Desktop, VS Code, Custom Agents)
        │
        │  Streamable HTTP over Corporate VPN / Intranet
        ▼
   http://<windows-host-ip-or-fqdn>:8382/mcp
        │
        ▼
   mcp-server.exe --http 0.0.0.0:8382         (Windows Service via NSSM)
        │
        ├── reads  ▶  C:\kuka-mcp\knowledge\*.md
        ├── reads  ▶  C:\kuka-mcp\kuka-movies\index.json
        └── reads  ▶  C:\kuka-mcp\kuka-prints\*
```

### Key Deployment Characteristics

- **Zero Runtime Dependencies on Windows**: `mcp-server.exe` is a standalone native executable. It does not require Rust, Node.js, `pdftotext`, `ocrmypdf`, or `soffice` on the Windows host.
- **Fast Startup & Flat Memory**: Indexes documents in milliseconds and holds metadata in memory.
- **Hot Reload**: Content changes (manuals or media indices) can be synced and reloaded on the fly via `reload_docs` without restarting the Windows Service.

---

## 2. Package Artifacts for Export

The deployment package requires the compiled Windows binary and the three knowledge/media folders.

### Package Contents

1. **`mcp-server.exe`** — Native Windows release binary (compiled via `cargo build --release --target x86_64-pc-windows-gnu`).
2. **`knowledge/`** — Markdown document chunks and `knowledge/images/` diagram PNGs.
3. **`kuka-movies/`** — `index.json` metadata index file (large video binaries are not needed).
4. **`kuka-prints/`** — `index.json` metadata index file and print PDF files.

---

## 3. Host Directory Structure

On the target Windows host machine, create the following directory structure (e.g. `C:\kuka-mcp\`):

```
C:\kuka-mcp\
├── bin\
│   └── mcp-server.exe
├── knowledge\
│   ├── *.md
│   └── images\
│       └── *.png
├── kuka-movies\
│   └── index.json
├── kuka-prints\
│   ├── index.json
│   └── *.pdf
└── logs\
```

---

## 4. Step-by-Step Installation on Windows

### Step 4.1: Copy Package Files

Copy the packaged folders and executable to `C:\kuka-mcp\` using SMB share, USB, or `robocopy`:

```powershell
robocopy <source_path>\knowledge C:\kuka-mcp\knowledge /MIR
robocopy <source_path>\kuka-movies C:\kuka-mcp\kuka-movies /MIR
robocopy <source_path>\kuka-prints C:\kuka-mcp\kuka-prints /MIR
copy <source_path>\mcp-server.exe C:\kuka-mcp\bin\mcp-server.exe
```

### Step 4.2: Test Direct Execution (Sanity Check)

Open PowerShell on the Windows host:

```powershell
# Set environment variable pointing to knowledge bundle
$env:KUKA_KNOWLEDGE_DIR="C:\kuka-mcp\knowledge"

# Launch mcp-server in HTTP mode
C:\kuka-mcp\bin\mcp-server.exe --http 0.0.0.0:8382
```

Verify in PowerShell or browser:

```powershell
curl.exe http://127.0.0.1:8382/mcp
```

*(Press `Ctrl+C` to terminate test execution once confirmed.)*

### Step 4.3: Register as an Always-On Windows Service (NSSM)

Use **NSSM** ([Non-Sucking Service Manager](https://nssm.cc/)) to ensure `mcp-server.exe` runs unattended, starts on boot, and auto-restarts on crash.

Run PowerShell as **Administrator**:

```powershell
# Download / place nssm.exe in C:\kuka-mcp\bin\
nssm install kuka-mcp-server "C:\kuka-mcp\bin\mcp-server.exe" --http 0.0.0.0:8382
nssm set kuka-mcp-server AppEnvironmentExtra KUKA_KNOWLEDGE_DIR=C:\kuka-mcp\knowledge
nssm set kuka-mcp-server AppStdout C:\kuka-mcp\logs\mcp-server.out.log
nssm set kuka-mcp-server AppStderr C:\kuka-mcp\logs\mcp-server.err.log
nssm set kuka-mcp-server Start SERVICE_AUTO_START
nssm set kuka-mcp-server AppExit Default Restart
nssm start kuka-mcp-server
```

Check service status:

```powershell
Get-Service kuka-mcp-server
```

### Step 4.4: Configure Windows Firewall

Allow inbound TCP traffic on port `8382` for internal corporate network / VPN clients:

```powershell
New-NetFirewallRule -Name "KUKA_MCP_Server_8382" `
                    -DisplayName "KUKA MCP Server (HTTP 8382)" `
                    -Direction Inbound `
                    -Protocol TCP `
                    -LocalPort 8382 `
                    -Action Allow
```

---

## 5. Client Configuration & Harness Directives

Once the server is running on the Windows host (e.g., IP `10.20.30.50` or hostname `kuka-mcp.internal.company.com`), team members connecting via corporate VPN configure their AI harnesses as follows:

### 5.1 Claude Code Configuration (`.mcp.json` or `~/.claude.json`)

```json
{
  "mcpServers": {
    "kuka": {
      "url": "http://10.20.30.50:8382/mcp"
    }
  }
}
```

### 5.2 Claude Desktop Configuration (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "kuka": {
      "url": "http://10.20.30.50:8382/mcp"
    }
  }
}
```

### 5.3 VS Code MCP Configuration (`.vscode/mcp.json`)

```json
{
  "servers": {
    "kuka": {
      "url": "http://10.20.30.50:8382/mcp"
    }
  }
}
```

### 5.4 Reverse Proxy / TLS Auth Configuration (Optional Security Layer)

If a reverse proxy (such as Caddy or IIS) is placed in front of `mcp-server` with an `X-Api-Key` header check:

```json
{
  "mcpServers": {
    "kuka": {
      "url": "https://kuka-mcp.internal.company.com/mcp",
      "headers": {
        "X-Api-Key": "<shared-corporate-api-key>"
      }
    }
  }
}
```

---

## 6. Prompting & Harness Directives for Corporate Users

When instructing AI agents or custom harnesses on the corporate VPN, provide the following prompt directive:

> **System Prompt / Harness Directive:**
> *"You are connected to the KUKA Knowledge MCP Server over the corporate network at `http://<windows-host-ip>:8382/mcp`.*
>
> **Workflow rules for KUKA questions:**
>
> 1. **Text Documentation**: Always call `search_docs` first to query manuals and technical notes.
> 2. **Videos, Prints & Schematics**: Call `list_media` or `get_media` by topic (`electrical`, `mechanical`, `safety`, `vision`, `localization`, etc.) or keyword to discover schematics and video assets.
> 3. **Deep Reading**: Read the `kuka://docs/{name}` resource returned in search hits for complete sections.
> 4. **Diagrams**: Search hits include `Diagrams: kuka://images/{name}` URIs — read these resources to inspect embedded schematics and figures."*

---

## 7. Ongoing Maintenance & Hot Reloading

When documents or media registries are updated in the development environment:

1. **Mirror Updated Folders to Windows**:

   ```powershell
   robocopy <dev_workspace>\knowledge C:\kuka-mcp\knowledge /MIR
   robocopy <dev_workspace>\kuka-movies C:\kuka-mcp\kuka-movies /MIR
   robocopy <dev_workspace>\kuka-prints C:\kuka-mcp\kuka-prints /MIR
   ```

2. **Hot Reload Server Index**:
   Send a `reload_docs` call over MCP (or tell any connected AI client *"reload the KUKA docs"*). The server will rebuild the search index and reload the media registries in memory **without restarting the Windows Service**.
