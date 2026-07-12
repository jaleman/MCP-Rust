# KUKA Knowledge Server — User Manual

An MCP (Model Context Protocol) server, written in Rust, that lets AI
assistants like Claude answer questions about KUKA AMR robots using your
official KUKA documentation as the source of truth.

---

## Contents

1. [Introduction](#1-introduction)
2. [How it works](#2-how-it-works)
3. [Requirements](#3-requirements)
4. [Installation](#4-installation)
5. [Building the knowledge bundle (extracting documents)](#5-building-the-knowledge-bundle-extracting-documents)
6. [Connecting an AI client](#6-connecting-an-ai-client)
7. [Using the server](#7-using-the-server)
8. [Keeping the bundle up to date](#8-keeping-the-bundle-up-to-date)
9. [Configuration reference](#9-configuration-reference)
10. [Troubleshooting](#10-troubleshooting)
11. [Appendix: file formats](#11-appendix-file-formats)

---

## 1. Introduction

### What it is

The KUKA Knowledge Server turns a folder of KUKA PDF documentation (fleet
manuals, technical notes, safety and deployment guides) into a searchable
knowledge base that AI agents query over MCP. Instead of relying on an AI
model's general knowledge — which may be outdated or wrong about KUKA
specifics — answers are grounded in excerpts from the actual documents,
with page-level references back to the source PDFs.

### Capabilities

- **Full-text search** with typo tolerance ("reflecter" finds "reflector"),
  partial-word matching ("amr" matches "KUKA.AMR"), and stop-word filtering
  ("what is the reflector height" searches for "reflector height").
- **Relevance ranking** that favors documents *dense* in your query terms,
  with excerpts anchored where multiple query terms appear close together.
- **Page-level provenance**: results from large documents are titled with
  their page range, e.g. *"MQTT Payload Definitions (pages 9–16)"*, so you
  can find the passage in the original PDF.
- **Clean extraction**: running page headers/footers (identified by
  repeating *at page edges*), page-number markers, and table-of-contents
  lines are stripped at extraction time, and tables keep their layout — so
  search results show content, not navigation noise. Content that repeats
  *inside* pages (a lookup table printed under several sections) is
  deliberately preserved.
- **OCR fallback for image-only PDFs**: when normal extraction finds no text
  layer, the extractor runs `ocrmypdf`, then feeds the searchable PDF back
  through the same page-preserving pipeline. OCR-derived bundle files are
  tagged with `ocr` for auditability.
- **Native Office and text ingestion**: PDFs, Word documents (`.docx`/`.doc`),
  PowerPoint decks (`.pptx`/`.ppt`), and plain text files (`.txt`) can all be
  passed directly to `extract`. Office files are converted through LibreOffice
  to preserve page structure; text files go straight through cleaning and
  chunking.
- **Document listing and full-document reading** via MCP resources.
- **Diagrams**: images embedded in the source documents are extracted at
  ingestion (page furniture like logos is filtered out) and served as
  `kuka://images/…` resources. Search hits list the diagrams belonging to
  the matched section, and multimodal assistants can open and interpret
  them alongside the text.
- **Live reload**: add or re-extract documents and refresh the index without
  restarting the server.
- **Fast and lightweight**: the search index is built at startup in
  milliseconds (12 documents ≈ 23 ms) and document bodies are never held in
  memory — memory use stays flat as the document library grows.

### The two programs

| Binary | Purpose |
|--------|---------|
| `extract` | One-time / occasional: converts PDF, Office, and text files into the markdown "knowledge bundle" the server reads |
| `mcp-server` | Long-running: serves the bundle to AI clients over MCP (stdio) |

---

## 2. How it works

```
Source files       extract binary         knowledge bundle        mcp-server
kuka-docs/* ─────────────────────▶  knowledge/*.md      ──────────────────▶  AI client
PDF/Office/TXT     text extraction +    markdown files with      inverted index      (Claude)
            ~8 KB chunking      OKF frontmatter           + MCP tools
```

1. **Extraction**: `extract` pulls text from PDF, Office, or plain-text
  source files. Office files are first converted to a temporary PDF with
  LibreOffice, then processed exactly like PDFs so page provenance still
  works. If a PDF or converted Office file has no text layer, `extract`
  runs `ocrmypdf` once to create a searchable temporary PDF, then continues
  through the same extraction path. The resulting text is cleaned (running
  headers/footers, page-number markers, and table-of-contents lines are
  stripped — they are navigation, not knowledge) and written as markdown
  files with a metadata header (the *OKF frontmatter*: title, type, source
  filename, timestamp). Documents larger than ~8 KB are split into chunks,
  one file per chunk, so every unit is small enough for an AI agent to read
  comfortably.
2. **Indexing**: at startup, `mcp-server` reads every bundle file once and
   builds an inverted index (term → documents and positions). The bundle is
   trusted as already clean — everything in it is searchable.
3. **Serving**: the AI client launches `mcp-server` and talks to it over
   stdin/stdout. When you ask a KUKA question, the client calls the server's
   search tool, receives ranked excerpts, and composes an answer grounded in
   them.

---

## 3. Requirements

- **Rust toolchain** (cargo) — already provided if you use the project's
  devcontainer.
- **poppler-utils** (`pdftotext`) — used for PDF extraction. Required for
  page-accurate chunking; already installed in the devcontainer.
- **ocrmypdf** — used only as a fallback when a PDF has no extractable text
  layer. It wraps Tesseract and produces a searchable PDF that preserves
  pages, so the existing `pdftotext -layout` pipeline still provides page
  provenance. Already installed in newly rebuilt devcontainers.
- **LibreOffice Writer and Impress** (`soffice`) — used for Word and
  PowerPoint ingestion (`.docx`, `.doc`, `.pptx`, `.ppt`). Already installed
  in newly rebuilt devcontainers.
- **An MCP client** — Claude Code, Claude Desktop, VS Code with MCP support,
  or any other MCP-capable agent.

---

## 4. Installation

### Option A — devcontainer (recommended, used by this project)

Open the project in VS Code and reopen in the devcontainer
(`.devcontainer/devcontainer.json`). Everything needed is preinstalled.
Build both binaries:

```bash
cd /workspaces/MCP-Rust/mcp-server
cargo build --release
```

The binaries land in `mcp-server/target/release/` (`mcp-server` and
`extract`). Debug builds (`cargo build`) work too and are what the sample
client configs in this repo point at — just remember which one your client
config launches when you rebuild.

### Option B — native build

With Rust installed locally (and `pdftotext` on your PATH for extraction):

```bash
cd mcp-server
cargo build --release
```

### Option C — Windows binaries cross-compiled from the devcontainer

The devcontainer includes the `x86_64-pc-windows-gnu` target, so you can
produce Windows executables without installing Rust on Windows:

```bash
cd /workspaces/MCP-Rust/mcp-server
cargo build --release --target x86_64-pc-windows-gnu
```

Binaries appear under `target/x86_64-pc-windows-gnu/release/` (`.exe`).

### Verifying the build

```bash
cargo test          # 42 tests should pass
./target/release/extract --help
```

---

## 5. Building the knowledge bundle (extracting documents)

The server reads markdown files from a bundle directory (by default
`knowledge/`). You create and update that bundle with the `extract` binary.

### Extract a whole folder of documents

```bash
# from the project root, inside the devcontainer
cargo run --release --manifest-path mcp-server/Cargo.toml --bin extract -- \
    --force-pdftotext kuka-docs knowledge
```

- First argument: a single supported document file **or** a directory of documents
  (unsupported files are skipped; extensions are matched case-insensitively).
  Supported inputs are `.pdf`, `.docx`, `.doc`, `.pptx`, `.ppt`, and `.txt`.
- Second argument: the output bundle directory — it must already exist.
- `--force-pdftotext` (recommended): skips the built-in extractor and uses
  `pdftotext` directly. This is what enables **page-accurate chunking**,
  because `pdftotext` marks page boundaries. Without the flag, the built-in
  extractor is tried first and `pdftotext` is only a fallback. In either
  mode, OCR runs automatically only if the normal extraction result is empty.
  Office files are converted to temporary PDFs first; text files ignore this
  flag because they do not need PDF extraction.

### Command reference

The command above is the day-to-day one: extract everything, from inside the
devcontainer. Other situations call for a slightly different invocation:

| Task | Command |
|------|---------|
| Extract **one file** (not a whole folder) | `cargo run --manifest-path mcp-server/Cargo.toml --bin extract -- --force-pdftotext "kuka-docs/My Document.pdf" knowledge` |
| Extract the **whole folder** | `cargo run --manifest-path mcp-server/Cargo.toml --bin extract -- --force-pdftotext kuka-docs knowledge` |
| Run from the **Windows host** instead of a container terminal | prefix either command above with `docker exec -w /workspaces/MCP-Rust kuka-mcp-server` |
| Quieter output (suppress cargo's own build messages) | insert `--quiet` right after `cargo run` |
| Optimized build (only worth it for very large batches) | insert `--release` right after `cargo run` |

Notes:

- Quote any path containing spaces, as in `"My Document.pdf"` above.
- The **debug build** (no `--release`) is what you'll use for nearly
  everything — it's what's already cached in the devcontainer and compiles
  in a couple of seconds. Add `--release` only if extraction speed itself
  becomes the bottleneck (large batches of large PDFs).
- Every command above assumes the current directory is the project root
  (`/workspaces/MCP-Rust`). If you're not already there, `cd` into it first
  or run from the Windows host with the `docker exec -w` prefix shown above.
- After extracting, tell your assistant *"reload the KUKA docs"* so the
  running server picks up the new bundle (see Section 8).

### What you'll see

```
Extracting: kuka-docs/KUKA Technical Note-MQTT Payload Definitions_Ver1.1.9.pdf
  3 chunk(s):
  → knowledge/kuka-technical-note-mqtt-payload-definitions_ver119-p001-008.md
  → knowledge/kuka-technical-note-mqtt-payload-definitions_ver119-p009-016.md
  → knowledge/kuka-technical-note-mqtt-payload-definitions_ver119-p017-024.md
Extracting: kuka-docs/EmergencyFireAlarm.pdf
  no text layer — running OCR…
  2 chunk(s):
  → knowledge/emergencyfirealarm-p001-006.md
  → knowledge/emergencyfirealarm-p007-009.md
Extracting: kuka-docs/building map and extension map.docx
  → knowledge/building-map-and-extension-map.md
...
Done: 11 extracted, 0 failed.
```

- Small documents produce **one file**; documents over ~8 KB of text are
  split into page-ranged chunks (`-p009-016` = pages 9–16). Each chunk knows
  its parent document and page range, which is how search results get their
  page provenance.
- The text is **cleaned automatically**: running headers/footers (lines that
  repeat at the top/bottom of pages), "Page N of M" markers, and
  table-of-contents dot-leader lines are stripped, and tables are extracted
  with layout preserved (`pdftotext -layout`) so their rows stay readable.
  Content that repeats *within* pages — like a lookup table printed under
  several sections — is kept in full.
- **Diagrams are extracted too**: embedded images larger than ~10 KB (small
  logos and header graphics are skipped, byte-identical duplicates removed,
  capped at 20 per document) land in `knowledge/images/` and are linked to
  their chunk's page range via `images:` frontmatter.
- If OCR was needed, the output frontmatter includes
  `tags: [extracted, ocr, technical-note]`. That tag is a useful reminder
  that the text came from recognition rather than an original text layer.
- Office files keep their original identity in frontmatter even though
  LibreOffice uses a temporary PDF internally: `resource:` points to the
  `.docx`, `.doc`, `.pptx`, or `.ppt` you provided, never to a temp file.
- Text files are read directly, cleaned, chunked, and written as OKF
  markdown. They do not go through LibreOffice or OCR.
- A failure like `no text could be extracted, even after OCR` means normal
  extraction was empty and OCR also produced no usable text. If `ocrmypdf`
  is missing, the error tells you to install it. One bad PDF never aborts
  the batch.
- Re-running extraction over the same folder simply regenerates the files —
  it is safe and idempotent. A warning is printed only if two *different*
  source files would collide on the same output name.

### Hand-written markdown

The extractor now accepts PDF, Office, and text inputs directly, but the
server still does not care where bundle files come from — **anything in OKF
markdown is indexed and served** (see the Appendix for the format). You can
write bundle files by hand for content that never existed as a source
document: FAQs, procedures, tribal knowledge, or short internal notes.

Run `reload_docs` (Section 8) after adding files either way.

---

## 6. Connecting an AI client

The server speaks MCP over stdin/stdout: the client launches the binary and
pipes messages to it. The one thing every setup must get right is **where
the knowledge bundle is**: the server looks for a `knowledge/` directory
relative to its working directory, or wherever `KUKA_KNOWLEDGE_DIR` points.
If the bundle cannot be found, the server exits at startup with an error
naming the path it tried — check your client's MCP logs.

### Claude Code (project-level)

This repo ships a working `.mcp.json`. It launches the server *inside the
running devcontainer* from a Windows host — a useful pattern when the
binary is built in the container:

```json
{
  "mcpServers": {
    "kuka": {
      "command": "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe",
      "args": [
        "exec", "-i", "-w", "/workspaces/MCP-Rust", "kuka-mcp-server",
        "/workspaces/MCP-Rust/mcp-server/target/debug/mcp-server"
      ]
    }
  }
}
```

(`-w` sets the working directory so the relative `knowledge/` default
resolves; the devcontainer must be running.)

If you run the binary natively instead:

```json
{
  "mcpServers": {
    "kuka": {
      "command": "/path/to/mcp-server/target/release/mcp-server",
      "env": { "KUKA_KNOWLEDGE_DIR": "/path/to/MCP-Rust/knowledge" }
    }
  }
}
```

### Claude Desktop

Add the same entry to `claude_desktop_config.json`
(Settings → Developer → Edit Config), using an **absolute**
`KUKA_KNOWLEDGE_DIR` — Claude Desktop's working directory is not your
project folder. On Windows, point `command` at a cross-compiled
`mcp-server.exe` (see Installation, Option C), or reuse the `docker exec`
pattern above. Restart Claude Desktop after editing.

### VS Code

The repo's `.vscode/mcp.json` registers the server for VS Code's MCP
support when working inside the devcontainer:

```json
{
  "servers": {
    "kuka": { "command": "/workspaces/MCP-Rust/mcp-server/target/debug/mcp-server" }
  }
}
```

### Verifying the connection

Ask your client: *"ping the KUKA knowledge server"*. You should get back
**"KUKA Knowledge server is online and ready."** Then try
*"list the KUKA documents"* and confirm your bundle appears.

---

## 7. Using the server

### Just ask questions

In day-to-day use you don't call tools yourself — you ask your AI assistant
normal questions, and it decides to consult the server (the server
announces its capabilities to the client during the handshake). Examples:

> *"What's the minimum safe distance for a KMP 1500P?"*
> *"Which MQTT topic is the mission status published on?"*
> *"How high should reflectors be mounted, and what's the maximum spacing?"*
> *"What ports does the KUKA AMR Fleet need open?"*

The assistant searches, reads the returned excerpts, and answers with the
document (and page range) it drew from. Typos are fine — the fuzzy matcher
absorbs one or two letter errors in words of four letters or more.

### The tools, for reference

| Tool | What it does | Example phrasing |
|------|--------------|------------------|
| `search_docs` | Ranked full-text search; returns up to 3 excerpts per matching document, each hit with a `kuka://docs/…` resource URI for reading the full section | "search the KUKA docs for battery charging" |
| `list_docs` | Lists every document in the bundle, grouped by type | "what KUKA documents do you have?" |
| `reload_docs` | Rebuilds the search index from the bundle directory | "reload the KUKA docs" |
| `ping` | Health check | "is the KUKA server running?" |

### Reading whole documents (MCP resources)

Every bundle file is also exposed as an MCP resource with URI
`kuka://docs/<filename-without-.md>` — e.g.
`kuka://docs/kuka-technical-note-mqtt-payload-definitions_ver119-p009-016`.
Clients that support resources (Claude Desktop's attachment picker, for
example) can pull a full chunk into context when excerpts aren't enough.
Chunks are ~8 KB by design, so a whole one always fits comfortably.

### How to read search results

```
Found 2 result(s) for 'mission status':

• KUKA Technical Note-MQTT Payload Definitions_Ver1.1.9 (pages 9-16)
  Resource: kuka://docs/kuka-technical-note-mqtt-payload-definitions_ver119-p009-016

  ...Mission Status Payload
  Message to inform the customer of mission status. ...
```

- Results are ordered by relevance: how *densely* a document mentions your
  terms, not just how often. A focused 2-page note beats a long manual with
  scattered mentions.
- **All terms must match** (after stop-word removal). No results usually
  means one of your words appears nowhere in the bundle — drop or replace
  the rarest word and retry.
- The `(pages N–M)` suffix in the title points you to the exact place in
  the original PDF (the source PDF's name is recorded in the bundle file's
  frontmatter).
- The `Resource:` line is the follow-up step: if the excerpts don't contain
  the full answer, that URI reads the whole section. Tool output never
  contains source-file paths — agents work entirely through the server.
- A `Diagrams:` line (when present) lists `kuka://images/…` resources — the
  images extracted from that section's pages. Ask the assistant to open one
  ("show me that diagram") and it can view and explain the picture.

---

## 8. Keeping the bundle up to date

When documentation changes or you add new source files:

1. Drop the new/updated files into your source folder (e.g. `kuka-docs/`).
2. Re-run the extractor (Section 5).
3. Tell your assistant: *"reload the KUKA docs"* — it calls `reload_docs`
   and reports the new document/term counts. No restart needed.

If a reload fails (for instance the bundle directory was moved), the server
says so and **keeps serving the previous index**, so a bad reload never
takes the server down. Note that a reload is also the fix if excerpts ever
look garbled: excerpt text is read live from the bundle files, so editing
files on disk without reloading leaves the index pointing at stale offsets.

---

## 9. Configuration reference

| Setting | Default | Meaning |
|---------|---------|---------|
| `KUKA_KNOWLEDGE_DIR` (env var) | `knowledge` (relative to the server's working directory) | Where the knowledge bundle lives. Read once at startup. |
| `RUST_LOG` (env var) | off | Logging level, written to **stderr** (never stdout — that would corrupt the MCP stream). `RUST_LOG=info` logs the startup summary: documents, unique terms, index build time. |

Startup log example (`RUST_LOG=info`):

```
Starting KUKA MCP server: indexed 12 document(s), 1051 unique term(s) in 23.0ms (knowledge dir: knowledge)
```

---

## 10. Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| Server won't start; log says `knowledge directory not found: ... — set KUKA_KNOWLEDGE_DIR or run from the project root` | The bundle path doesn't resolve from the client's working directory | Set `KUKA_KNOWLEDGE_DIR` to an absolute path in the client config, or set the working directory (e.g. `docker exec -w …`) |
| `no text could be extracted, even after OCR` during extraction | The PDF has no text layer and OCR produced no usable text | Confirm `ocrmypdf` is installed and that the scan is readable; then re-run extraction |
| `ocrmypdf not found — install ocrmypdf` during extraction | The running environment was not rebuilt after OCR support was added, or `ocrmypdf` is not on `PATH` | Install `ocrmypdf` (`sudo apt-get install -y ocrmypdf`) or rebuild the devcontainer |
| `soffice not found — install libreoffice-writer libreoffice-impress` during Office extraction | LibreOffice is not installed in the running environment | Install `libreoffice-writer libreoffice-impress` or rebuild the devcontainer |
| `soffice produced no output for ...` during Office extraction | LibreOffice accepted the command but did not write the converted PDF | Check that the source Office file opens correctly; try saving it again and rerun extraction |
| "No results found" for a query that should match | AND semantics: one of your terms appears nowhere in the bundle | Remove the rarest word; check `list_docs` to confirm the document was extracted |
| Search results answer from old content after re-extracting | Index still reflects the previous bundle | Ask the assistant to *"reload the KUKA docs"* |
| Excerpts look truncated or garbled | Bundle files changed on disk after the index was built | Same fix: `reload_docs` |
| Client shows the server as failed immediately | Wrong binary path in the client config, or (for the docker exec setup) the devcontainer isn't running | Check the client's MCP log; start the container; rebuild the binary the config points at |
| Rebuilt the code but behavior didn't change | Client config points at `target/debug` but you rebuilt `--release` (or vice versa) | Rebuild the profile your config launches, or update the config |

---

## 11. Appendix: file formats

### Bundle files (OKF markdown)

Every file in the knowledge bundle is markdown with a frontmatter header.
Written by `extract`; parsed by the server. Example (a chunk):

```markdown
---
type: technical-note
title: KUKA Technical Note-MQTT Payload Definitions_Ver1.1.9 (pages 9-16)
description: Extracted from KUKA documentation.
resource: kuka-docs/KUKA Technical Note-MQTT Payload Definitions_Ver1.1.9.pdf
parent: kuka-technical-note-mqtt-payload-definitions_ver119
pages: 9-16
tags: [extracted, technical-note]
timestamp: 2026-07-04T21:37:13.595435146Z
---

...document text...
```

| Field | Meaning |
|-------|---------|
| `type` | Grouping key used by `list_docs` |
| `title` | Display title in listings and search results |
| `resource` | The original source file this text came from (`.pdf`, `.docx`, `.pptx`, `.txt`, etc.) |
| `parent` / `pages` | Only on chunks: the parent document's slug and the source page range |
| `tags` | Provenance tags. OCR-derived files include `ocr`: `[extracted, ocr, technical-note]` |
| `timestamp` | When the extraction ran |

You can also write bundle files **by hand** (meeting notes, FAQs, internal
procedures) — anything with this frontmatter and a text body is indexed and
searchable like any extracted document. Run `reload_docs` after adding.

### Chunk file naming

`<document-slug>-p<first>-<last>.md`, zero-padded to three digits:
`...-p001-008.md` covers pages 1–8. Documents that fit in a single chunk
use the plain slug with no page suffix.

---

*Developed as a Rust learning project — the design history and Java-vs-Rust
lessons behind every component live in [`lessons/`](lessons/) and
[`REFACTOR-PLAN.md`](REFACTOR-PLAN.md).*
