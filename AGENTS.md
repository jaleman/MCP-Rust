# Agent instructions — MCP-Rust (KUKA Knowledge Server)

This file is read by coding agents (Codex, and other harnesses that honor
the AGENTS.md convention). Claude Code reads the equivalent CLAUDE.md —
keep the two files in sync when policies change.

A Rust learning project: an MCP server that answers KUKA AMR robot questions
grounded in official KUKA documentation. Key docs: MISSION.md (goals),
USER-MANUAL.md (install/use), REFACTOR-PLAN.md (status dashboard + progress
log — read it before doing any planned work, and keep it updated).

## Answering KUKA content questions

- Use the `kuka` MCP server tools. **Always `search_docs` first.**
- If the excerpts don't fully answer: retry `search_docs` with different
  terms, or read the `kuka://docs/{name}` resource named in the hit — bundle
  documents are chunked to ~8 KB, so reading one whole is always safe.
- Do **not** answer KUKA content questions by reading `kuka-docs/*` (source
  PDFs) or `knowledge/*.md` from the filesystem — those are the server's
  inputs, not a knowledge interface. (Reading them while developing or
  debugging the server itself is fine.)
- After re-extracting documentation, call `reload_docs`.
- **Diagrams**: search hits may carry `Diagrams: kuka://images/{name}` lines.
  Read the resource to view/interpret the image yourself. When the USER asks
  to *see* a diagram, reading it is not enough — also open the image on their
  screen: `code -r knowledge/images/{name}` (the same PNGs live in the
  bundle), which displays it in an editor image tab.

## Build & test

- cargo is **not** installed on the Windows host. Everything runs in the
  devcontainer:
  `docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server cargo test`
- Definition of verified: `cargo clippy --all-targets` clean and all tests
  passing in the devcontainer.
- `.mcp.json` launches `mcp-server/target/debug/mcp-server` via `docker exec`
  — rebuild the **debug** profile after code changes or clients keep running
  the old binary.
- Re-extract the bundle:
  `cargo run --bin extract -- --force-pdftotext kuka-docs knowledge`
  (from `/workspaces/MCP-Rust`; `--force-pdftotext` gives page-accurate
  chunking).

## Workflow conventions

- One PR per work step (branch `refactor/step-N-<name>` or `docs/<name>`);
  the **user** reviews and merges — never merge or push to master directly
  except trivial plan-bookkeeping commits.
- Ask the user for approval before starting a new plan step; finishing one
  step is not authorization for the next.
- Significant steps are documented as lessons in `lessons/refactor-NN-*.html`
  (Java-vs-Rust side-by-side style — the user is a Java developer learning
  Rust; follow the existing template).
- Line endings are pinned to LF via `.gitattributes`; if phantom modified
  files appear with an empty `git diff`, run `git restore .`.
