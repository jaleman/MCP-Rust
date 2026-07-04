# Teaching Notes

## Pick up here (2026-06-29) — after Lesson 23

**What happened this session:**
- Delivered Lesson 23: std::process::Command + pdftotext fallback
- New patterns introduced: `bail!`, `.context()`, `String::from_utf8()`, if-as-expression, `output.status.success()`
- `--force-pdftotext` flag added to the clap Args struct (lesson 22 payoff)
- User applies the changes by following the lesson steps (not pre-applied)

**First thing next session:**
1. Confirm the user applied lesson 22 clap changes AND lesson 23 pdftotext changes
2. Check: did `EmergencyFireAlarm.pdf` extract non-empty text with the fallback?
3. Decide: run batch extraction on all 16 kuka-docs PDFs now that the fallback is in place?
4. Candidate next: production deployment OR tools/list_changed notification

**Candidate next lessons (in rough priority order):**
A. Batch extraction run — now that the fallback is working, extract all 16 PDFs and verify OKF files
B. Production deployment — HTTP/SSE transport, configurable path, auth proxy
   (hold until user signals readiness)
C. tools/list_changed notification — so Claude desktop doesn't need a new conversation when the server binary changes

## User preferences
- Dual goal: learn Rust AND ship something real (not just learn for its own sake)
- Personal use first; external shipping is a future phase
- Workspace: D:\Projects\Learning\MCP-Rust

## Code style in lessons
- All code blocks in lessons must include comments explaining what each block does
- This applies to both the "Before" and "After" code shown in steps, and to any standalone examples

## Quiz design rules
- NEVER put the correct answer at index 0 for every question in a lesson — vary position across questions (e.g. correct at 0, 2, 1, 3 across four questions)
- The pattern "first answer is always right" is detectable and kills retrieval practice

## Context
- KUKA PDFs not yet provided — ingestion pipeline lesson should be held until PDFs are available
- Currently using Claude as the AI agent client

## kuka-docs library inventory (added 2026-06-26)

**PDFs (16 files):**
- Operating manuals (BA_ prefix): BA_KMF_1500P-CB, BA_KMP_1500P, BA_KMP_3000P
- Fleet software manuals: KUKA_AMR_Fleet_2.14, KUKA_AMR_Fleet_2.15
- Technical Notes: Reflector Deployment, Magna Server Upgrade, RedHat Linux, MQTT Config, Adapter Installation, MQTT Payload Definitions, Fleet Ports, Reflective Sticker Specs
- Safety: Safe Minimum Distances, Emergency Fire Alarm
- Other: KMP Calibration WI, InSSIDer WiFi guide

**DOCX (1 file):** building map and extension map.docx — likely important for navigation/site context

**PNG (1 file):** FocusCenter.png — probably a screenshot of KUKA's FocusCenter management UI; text not extractable without OCR or manual annotation

**OKF types identified for this library:**
operating-manual, fleet-manual, technical-note, safety, work-instruction, reference, site-document

## Future lesson: production deployment

The user wants a lesson on deploying the knowledge server to production (external users, beyond personal use). Key things to cover:

- Choosing a transport: HTTP/SSE instead of stdio for remote access (rmcp supports both)
- Packaging: compiling a release binary (`cargo build --release`) vs. shipping a Docker image
- Where to run it: always-on server or container host (not a dev container)
- Making the `knowledge/` path configurable via env var or CLI arg (not hardcoded relative path)
- Auth: MCP has no built-in auth — need a reverse proxy (nginx, Cloudflare Tunnel) or API key middleware
- Working directory gotcha: the stdio/docker-exec cwd problem encountered during personal use is a preview of why production needs explicit path config

Hold until the user signals readiness to productionise (currently personal use first per MISSION.md).

## Future lesson: tools/list_changed notification

The user discovered that updating the server binary requires starting a new Claude conversation for the client to see new tools. The MCP protocol has a `notifications/tools/list_changed` notification that a server can send to tell a connected client to re-fetch the tool list without disconnecting.

Key things to cover:
- What `notifications/tools/list_changed` is and when to send it
- How rmcp exposes this (need to verify against rmcp docs — may require access to the peer/session object)
- Client-side behaviour: Claude desktop app may or may not handle it — verify before teaching
- Production implication: additive tool changes (never remove/rename) so existing sessions don't break
- This naturally fits as part of the production deployment lesson or as a standalone MCP protocol lesson

Hold until the user hits this problem in practice with a real user session, or until production deployment lesson is scheduled.

## Pending: PowerPoint/scan PDF handling

Some PDFs in the kuka-docs library extract empty body text with `pdf-extract`:
- `EmergencyFireAlarm.pdf` — confirmed PowerPoint-to-PDF; text not extracted
- Likely affects other presentation-style or scanned documents

`pdftotext` (from poppler-utils) handles these better. Future lesson should:
- Install poppler-utils in dev container
- Try `pdftotext` on the problem files
- If successful, update `extract.rs` to fall back to `pdftotext` via `std::process::Command` when `pdf-extract` returns empty text
- Also evaluate whether DOCX files (building map.docx) need `docx-rs`

For now: skip problem PDFs and populate their OKF files manually if content is critical.

## Pending
- Evaluate pdf-extract vs lopdf vs pdfium against the actual PDFs (esp. fleet manuals — likely diagram-heavy)
- Decide PNG handling: OCR (tesseract bindings), manual OKF entry, or MCP Resource instead of searchable content
- DOCX parsing: docx-rs crate in Rust

## Lesson 6 design decision (PDF ingestion)
- Use OKF (Google Open Knowledge Format, v0.1, published 2026-06-12) as the intermediate layer
- Architecture: KUKA PDFs → extraction → OKF bundle (/knowledge/*.md) → MCP server searches bundle
- OKF type values to define for KUKA: procedure, specification, safety-note, robot-model, component
- resource field in frontmatter should link back to source PDF + page number for citations
- Spec: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
- This replaces the raw-PDF-to-vector-store approach; search_docs tool queries the bundle instead
