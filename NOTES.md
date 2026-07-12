# Teaching Notes

> **Status note (2026-07-12):** the numbered-lesson era (0001-0025) ended and
> the refactor era took over — see REFACTOR-PLAN.md for the live status
> dashboard and progress log (steps 1-12 complete, 11 in review, 13-14
> designed). Many "future lesson" items originally parked here were since
> shipped as refactor steps and documented as refactor-NN lessons instead.
> This file keeps the evergreen teaching preferences plus what genuinely
> remains open.

## User preferences
- Dual goal: learn Rust AND ship something real (not just learn for its own sake)
- Personal use first; external shipping is a future phase
- Workspace: D:\Projects\Learning\MCP-Rust
- Approve each refactor step before it starts — finishing one step is not
  authorization for the next (also encoded in REFACTOR-PLAN.md / CLAUDE.md / AGENTS.md)

## Code style in lessons
- All code blocks in lessons must include comments explaining what each block does
- This applies to both the "Before" and "After" code shown in steps, and to any standalone examples
- Refactor lessons additionally require Java equivalents side-by-side with the
  Rust (the user reads Rust through a Java lens) — see REFACTOR-PLAN.md's
  lesson template rules

## Quiz design rules
- NEVER put the correct answer at index 0 for every question in a lesson — vary position across questions (e.g. correct at 0, 2, 1, 3 across four questions)
- The pattern "first answer is always right" is detectable and kills retrieval practice

## kuka-docs library inventory (added 2026-06-26)

**PDFs (16 files):**
- Operating manuals (BA_ prefix): BA_KMF_1500P-CB, BA_KMP_1500P, BA_KMP_3000P
- Fleet software manuals: KUKA_AMR_Fleet_2.14, KUKA_AMR_Fleet_2.15
- Technical Notes: Reflector Deployment, Magna Server Upgrade, RedHat Linux, MQTT Config, Adapter Installation, MQTT Payload Definitions, Fleet Ports, Reflective Sticker Specs
- Safety: Safe Minimum Distances, Emergency Fire Alarm
- Other: KMP Calibration WI, InSSIDer WiFi guide

**DOCX (1 file):** building map and extension map.docx — ingested since
refactor step 9a (LibreOffice headless → PDF → existing pipeline)

**PNG (1 file):** FocusCenter.png — still not ingested; standalone images are
not part of the extraction pipeline (only images extracted FROM documents are
served, via step 9b)

**OKF types identified for this library:**
operating-manual, fleet-manual, technical-note, safety, work-instruction, reference, site-document

## Formerly-pending items now shipped as refactor steps

Kept as a map from old plans to what actually happened:

- **pdftotext fallback for PowerPoint-style PDFs** → lesson 23, then step 8
  added an ocrmypdf/Tesseract OCR fallback for fully image-based PDFs
  (EmergencyFireAlarm.pdf is searchable now); lesson refactor-13.
- **DOCX parsing** → step 9a, via LibreOffice headless conversion rather than
  the docx-rs crate originally anticipated; lesson refactor-14.
- **Configurable knowledge path** → KUKA_KNOWLEDGE_DIR env var (step 4
  territory); lesson refactor-07.
- **HTTP transport for remote access** → step 10 shipped `--http`
  (streamable HTTP, loopback-only, no auth); lesson refactor-16. Public
  HTTPS/OAuth for claude.ai connectors remains a future phase (see
  designs/step-10-streamable-http-transport.md "out of scope" notes).
- **Diagram-heavy fleet manuals** → step 9b extracts per-page images and
  serves them as kuka://images/ resources; lesson refactor-15.

## Still genuinely open

- **tools/list_changed notification** — updating the server binary still
  requires a new client conversation to see new tools. The MCP protocol has
  `notifications/tools/list_changed`; rmcp support and Claude-client handling
  both need verification before teaching/implementing. Fits naturally with a
  future production-deployment step.
- **Production deployment / public exposure** — auth (reverse proxy or API
  key), release packaging, always-on hosting. Hold until the user signals
  readiness to productionise (personal use first per MISSION.md).
- **FocusCenter.png** — decide: OCR it, describe it manually in a hand-written
  OKF file, or leave it out of the bundle.

## Lesson 6 design decision (PDF ingestion) — historical record
- Use OKF (Google Open Knowledge Format, v0.1, published 2026-06-12) as the intermediate layer
- Architecture: KUKA PDFs → extraction → OKF bundle (/knowledge/*.md) → MCP server searches bundle
- resource field in frontmatter links back to the source document for citations
- Spec: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
- This replaced the raw-PDF-to-vector-store approach; the search_docs tool
  queries the bundle (since step 5b, via an inverted index)
