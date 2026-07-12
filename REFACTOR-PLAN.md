# Refactoring Plan: mcp-server

Agreed plan from code review + architecture discussion (2026-07-04).
Each step compiles and passes tests independently — commit after each.
Verification happens in the devcontainer (`cargo clippy --all-targets && cargo test`);
cargo is not installed on the Windows host.

## Status dashboard

> **This table is the single source of truth for progress.** Whoever works on
> this plan (human or agent) must update it when starting or finishing a step,
> and add a dated entry to the Progress Log at the bottom of this file.
> Statuses: `not started` | `in progress` | `complete` | `blocked`.
> A step is `complete` only after `cargo clippy --all-targets && cargo test`
> passes in the devcontainer AND the step is committed.
>
> **Do not start a step without the user's explicit approval.** The user has
> asked to be consulted before each step begins — finishing one step is NOT
> authorization to start the next. Ask, wait for a yes, then flip the status
> to `in progress`.
>
> **Every step must be documented as lessons before it counts as complete.**
> After the implementation is verified and committed, write one or more lesson
> files in `lessons/` under the refactor appendix series:
> `refactor-01-<topic>.html`, `refactor-02-<topic>.html`, … (numbering is
> sequential across the whole series, not per step; a step may produce more
> than one lesson). Follow the existing lesson template exactly:
> `../assets/style.css`, `lesson-header` block (lesson-number "Refactor
> Appendix N"), prose descriptions of WHY the change was made, before/after
> Rust code samples from the actual commit, and **equivalent Java samples**
> alongside the Rust (the user reads Rust through a Java lens — see the
> existing appendix-*.html files for the side-by-side style and
> comparison-table usage). Cross-link related lessons and learning-records.
> So the full completion criteria are: tests pass in devcontainer → committed
> → refactor lessons written → dashboard flipped to `complete` + log entry.
>
> **Git workflow (since 2026-07-04): pull-request per step.** The repo root is
> `D:\projects\Learning\MCP-Rust` with remote `jaleman/MCP-Rust` on GitHub
> (the old nested mcp-server repo was promoted to the root in 06a1b02; its
> history is preserved). For each step: create a branch
> `refactor/step-N-<short-name>` off master, commit the code AND its refactor
> lessons AND the plan/dashboard updates on that branch, push, and open a PR
> with `gh pr create`. The USER reviews and merges the PR — do not merge it
> yourself. A step is `complete` when its PR is merged. Always push; nothing
> stays local-only. Trivial bookkeeping-only edits (progress-log lines,
> workflow notes) may be committed directly to master and pushed.

| Step | Description | Status | Notes |
|------|-------------|--------|-------|
| 1 | lib/bin split, shared frontmatter module | complete | commit 17967a8; lessons refactor-01, refactor-02 |
| 2 | Document + load_bundle, error/traversal/extension fixes | complete | PR #1 merged (948a356); lessons refactor-03, refactor-04 |
| 3 | SearchHit API, consts, LazyLock stop words, test rewrite | complete | PR #2 merged (1ace0b2); lessons refactor-05, refactor-06 |
| 4 | config into KukaServer | complete | PR #4 merged (e221860); lesson refactor-07 |
| 5a | chunking in extract | complete | PR #5 merged (a6a813e); lesson refactor-08 |
| 5b | inverted index, seek excerpts, reload_docs | complete | PR #6 merged (4732155); lessons refactor-09, refactor-10 |
| 5c | tantivy/hybrid escape hatch | deferred | trigger conditions in §5c |
| 6 | clean extraction + agent steering (post-plan) | complete | PR #9 merged (e601cb3, after the #8 stacked-merge mishap); lesson refactor-11 |
| 7 | zone-based boilerplate detection (data-loss fix) | complete | PR #10 merged (b15f39e); lesson refactor-12 |
| 8 | OCR ingestion for image-based PDFs | complete | PR #11 merged (b6cbf23), implemented by Codex, reviewed + verified by Claude; lesson refactor-13 |
| 9a | Office (.docx/.pptx) + plain-text (.txt) ingestion | complete | PR #12 merged (0a035ab), implemented by Codex, reviewed + verified by Claude; lesson refactor-14 |
| 9b | diagram/image extraction + serving as MCP resources | complete | PR #13 merged (995c399); lesson refactor-15 |
| 10 | streamable-HTTP transport (browser/remote clients) | in progress | implemented + verified locally 2026-07-12; PR still needed |

## Resuming mid-step (handoff protocol)

If a step is `in progress`, the Progress Log entry for it must say:
- which files were already changed and which remain,
- whether the code currently compiles / tests pass,
- the exact next action.

When picking up: read this file top to bottom, read the Progress Log last
entry, run `cargo test` in the devcontainer to confirm the recorded state
matches reality, then continue.

## Context

- Current state: `src/main.rs` (~740 lines: server, search engine, frontmatter
  parsing, all tests) + `src/extract.rs` (PDF → OKF markdown CLI).
- Drivers: corpus will grow from 9 docs to hundreds; some source documents are
  very large. Search cost and memory must become independent of corpus size.
  Everything the server returns must fit in an AI agent's context.
- Decision: lexical search first (hand-rolled inverted index → tantivy/FTS5 if
  needed). Vector search deferred until real queries show vocabulary-mismatch
  failures; would be added as hybrid alongside BM25, not a replacement.

## Step 1 — Split into library + two binaries

```
src/
  lib.rs           pub mod frontmatter; pub mod bundle; pub mod search;
  frontmatter.rs   parse fields + generate OKF block (shared by both bins)
  bundle.rs        knowledge dir walking (grows in step 2)
  search.rs        fuzzy match, normalize/repeated lines, char-boundary
                   helpers, search logic + unit tests
  main.rs          MCP server binary (KukaServer, tools, main)
  bin/extract.rs   moved from src/extract.rs
```

- Delete both `[[bin]]` blocks in Cargo.toml (auto-discovery handles it).
- Key design act: extract.rs and main.rs share the frontmatter format via
  `frontmatter.rs` (one writes it, one parses it — currently duplicated
  knowledge that can drift).
- Done when: `cargo test` passes unchanged, both bins build.

## Step 2 — `Document` type + single loader

In `bundle.rs`:

```rust
pub struct Document {
    pub path: PathBuf,
    pub stem: String,      // filename without .md; doubles as resource URI id
    pub title: String,
    pub doc_type: String,
    pub resource: String,
    pub description: Option<String>,
    pub content: String,
    pub body_start: usize, // byte offset past frontmatter
}
impl Document {
    pub fn load(path: &Path) -> anyhow::Result<Self>;
    pub fn body(&self) -> &str;
}
pub fn load_bundle(dir: &Path) -> anyhow::Result<Vec<Document>>;
```

- Kills the triplicated dir-walk in list_docs / search_docs / list_resources;
  frontmatter parsed once per file.
- `load_bundle` errors (with path) on missing dir — fixes "missing dir looks
  like empty bundle"; warns + skips unreadable files; extension check becomes
  case-insensitive (`eq_ignore_ascii_case`).
- Fold in the security fix: `read_resource` rejects stems containing `/`,
  `\`, or `..` (path traversal).

## Step 3 — Search returns data, not formatted text

In `search.rs`:

```rust
pub struct SearchHit {
    pub title: String,
    pub resource: String,
    pub score: usize,
    pub excerpts: Vec<String>,
}
pub fn parse_query(query: &str) -> Vec<&str>;    // stop-word filter
pub fn search(docs: &[Document], query: &str) -> Vec<SearchHit>;
```

- The `#[tool]` method becomes a thin adapter; result formatting stays in
  main.rs (presentation).
- Magic numbers become named consts (excerpt radius 150/300, window 500, max
  3 excerpts, repeated-line threshold 3, fuzzy tier boundary 7).
- Stop words become `static LazyLock<HashSet<&str>>`.
- Integration tests rewritten to assert on SearchHit fields instead of
  `format!("{:?}", CallToolResult)` substring matching. (~half the work of
  this step; makes steps 4–5 safe.)

## Step 4 — Configuration lives in the server

```rust
struct KukaServer { knowledge_dir: PathBuf, tool_router: ToolRouter<KukaServer> }
```

- `main()` resolves KUKA_KNOWLEDGE_DIR once; free function `knowledge_dir()`
  deleted. Tests construct the server with a temp dir directly.
- Small on purpose — this is the seam step 5 plugs into.

## Step 5a — Chunk at extract time

- `pdftotext` emits form-feed (\x0c) between pages → free section boundary.
- Accumulate pages into ~8 KB chunks; one OKF file per chunk
  (`kmp-3000-manual-p012-018.md`) with frontmatter `parent:` and `pages:`,
  title "KMP 3000 Manual (pages 12–18)".
- Small docs (≤ target) produce one file exactly as today.
- Payoff: read_resource can never return more than ~8 KB; excerpts come from
  small files; hits carry page-level provenance.
- Agreed decisions: ~8 KB target, page-boundary splitting, one file per chunk
  (not offset tables).

## Step 5b — Inverted index

New `index.rs`:

```rust
pub struct Index {
    docs: Vec<DocMeta>,                    // doc_id = index
    vocab: HashMap<String, Vec<Posting>>,  // lowercased term → postings
}
struct Posting {
    doc_id: u32,
    freq: u32,           // total occurrences (scoring)
    positions: Vec<u32>, // byte offsets in ORIGINAL file — capped at 16
}
```

Build (once at startup, `Index::build(dir)`):
1. Load documents via `load_bundle`.
2. Compute `repeated_lines` at INDEX time; boilerplate tokens never enter the
   index (query-time filtering dance disappears).
3. Tokenize body walking words with byte offsets in the ORIGINAL content;
   lowercase per-token for the vocab key. This eliminates the
   lowercased-string-vs-original byte-offset mismatch entirely. Strip
   trailing punctuation per token.
4. Cap positions at 16 per (term, doc) — positions are the only index part
   that scales with corpus size (~one u32 per word uncapped); freq keeps
   scoring honest; 16 anchors >> the 3 excerpts ever shown.

Query:
1. `parse_query`; per term: exact vocab lookup, else (≥4 chars) fuzzy-scan
   vocab keys with length pre-filter + levenshtein — over the unique-word
   vocabulary, not every word of every document.
2. AND semantics across terms (as today).
3. Score: sum(freq) / doc token count (fixes long-doc bias; chunking makes
   sizes near-uniform). Proximity co-occurrence from positions as tiebreak.
4. Excerpts: seek(pos − 150), read ~450 bytes, from_utf8_lossy, trim to
   whole words. No document fully loaded at query time.

Server integration:
- `KukaServer { index: Arc<RwLock<Index>>, ... }`; built before
  serve(stdio()); startup log reports docs/terms/build time.
- list_docs / list_resources read index.docs (no disk).
- New `reload_docs` tool rebuilds on demand (no file watching). Rebuild of
  hundreds of chunked docs ≪ 1s, so no index persistence yet.

## Step 6 — Clean extraction + agent steering (added post-plan)

Driven by a real agent failure: a session asked "minimum safe distance for a
KMP 1500P", got TOC-anchored excerpts, gave up on search_docs, and fell back
to reading source PDFs / browsing resources. Three server defects invited it:

- **Excerpts anchored on TOC lines** (dot leaders co-occur every query term
  and sit earliest in the file). Fix: `clean_extracted_text` in chunk.rs —
  extract strips repeated header/footer lines AND TOC dot-leader lines
  before chunking, so bundle files themselves are clean (index-time filter
  retained as defense for hand-written files). Also `pdftotext -layout`
  for readable tables.
- **Hits displayed the source-PDF path** (an invitation to open files, and a
  dangling pointer in production). Fix: SearchHit carries `stem`; hits show
  `Resource: kuka://docs/{stem}` — the actionable escalation. Tool output
  never contains file paths (pinned by test).
- **No workflow guidance for agents.** Fix: search_docs description +
  get_info instructions encode "search first → retry terms → read the hit's
  resource URI; never source files"; CLAUDE.md added for coding sessions.

Roadmap noted here: DOCX/PPTX ingestion via pandoc (next capability);
Docling/marker for table-heavy PDFs; ocrmypdf for scans.

## Step 7 — Zone-based boilerplate detection (data-loss fix, added post-plan)

Driven by a real data-loss report: the RobotType lookup table (printed under
THREE payload sections in the MQTT Payload Definitions doc) vanished from the
bundle. Another session mis-diagnosed it as an image needing OCR; two greps
proved pdftotext extracts it fine and OUR cleaner deleted it. Root cause: the
step-6 rule "any line repeated ≥3× is a header" — but legitimate content
repeats too. Repetition is not identity; POSITION is the discriminator.

- `clean_extracted_text` rewritten: a line is boilerplate only if it repeats
  ≥3× AND ≥80% of its occurrences fall in page-edge zones (top 10 / bottom 5
  lines of a page). Mid-page repeated content survives. "Page N of M"
  markers (unique per page — repetition can never catch them) are matched by
  shape via slice patterns.
- The index-time repeated-lines filter was REMOVED (not softened): chunks
  have no page structure, so it cannot be made position-aware, and it would
  re-hide the table whenever a chunk contains it 3×. Principle: each
  data-quality rule lives at exactly one stage — the earliest with the
  information it needs. Index::build simplified to one tokenize pass;
  repeated_lines/BOILERPLATE_MIN_REPEATS deleted from search.rs.
- Verified live: "RobotType robot family code" returns the complete table
  in the excerpt; "RobotType valid values" returns the retry hint (honest —
  those words are absent), demonstrating steps 6+7 composing.

## Step 8 — OCR ingestion for image-based PDFs (in progress, user-requested 2026-07-05)

Goal: make image-based PDFs searchable — documents exported from PowerPoint
and similar render their text as images, so pdftotext gets nothing. Concrete
acceptance test: the user can ask questions about EmergencyFireAlarm.pdf
(currently refused by extract with "no text could be extracted").

Suggested approach (design when starting, per protocol ask user first):
- Preprocess with ocrmypdf (Tesseract-based; adds a text layer to the PDF,
  after which the EXISTING pipeline works unchanged — cleanest option), OR
  have extract.rs fall back to invoking OCR when both extractors yield
  nothing, mirroring the existing pdftotext fallback pattern.
- Install tesseract-ocr + ocrmypdf in the devcontainer (postCreateCommand).
- Keep provenance honest: tag OCR'd docs in frontmatter (e.g. tags: [ocr])
  since OCR text can contain recognition errors; the fuzzy matcher already
  tolerates 1-2 char errors, which helps.
- Verify: re-extract EmergencyFireAlarm.pdf, live query about its content.

## Step 9 — Office/text ingestion (9a) + diagrams (9b), approved 2026-07-06

User-approved scope, split across two agents; COORDINATION RULE: strictly
sequential (9a merges before 9b starts) because both touch extract.rs.
The dashboard + progress log in this file are the coordination channel:
each agent flips its row and logs start/finish per the standing protocol.

**9a (Codex):** extract accepts .docx/.pptx/.doc/.ppt via LibreOffice
headless → temp PDF → the EXISTING pipeline (pdftotext/OCR/clean/chunk,
page provenance intact), and .txt directly (read → clean → chunk; no
pages). Provenance (title/slug/resource frontmatter) must reference the
ORIGINAL file, not the temp PDF. Must NOT implement any image/diagram
handling. Acceptance: the user's .docx in kuka-docs extracts and is
answerable via search_docs. Devcontainer gains libreoffice-writer +
libreoffice-impress. Lesson refactor-14.

**9b (Claude):** per-page image extraction (pdftoppm/pdfimages) into
knowledge/images/, images: frontmatter per chunk, kuka://images/
blob resources, "Diagrams:" lines in search hits. Multimodal agents can
then read and interpret diagrams alongside text. Lesson refactor-15.
Future roadmap note: vision-model captions to make diagrams searchable.

## Step 5c — Escape hatch (designed in, not built)

`search(&Index, query) -> Vec<SearchHit>` is the entire public surface, so
swapping internals for tantivy (or adding a vector backend as hybrid) later
touches nothing above it. Trigger: startup index build exceeding a few
seconds, or slow fuzzy-vocab scans.

## Sequencing

| # | Work | Verify in devcontainer |
|---|------|------------------------|
| 1 | lib/bin split, shared frontmatter module | cargo test unchanged, both bins build |
| 2 | Document + load_bundle, error handling, traversal + extension fixes | tests + manual list_docs on real bundle |
| 3 | SearchHit API, consts, LazyLock stop words, test rewrite | rewritten tests pass |
| 4 | config into KukaServer | tests construct server with temp dir |
| 5a | chunking in extract | re-extract the 9 real PDFs, inspect output |
| 5b | Index, seek-based excerpts, reload_docs | full suite + old-vs-new query comparison |

## Outstanding smaller fixes (fold in where noted)

- Hardcoded `timestamp:` in extract.rs → real time (step 1/2 territory).
- Slug collisions in extract.rs → warn on overwrite (step 5a touches this code).
- Server `get_info` instructions should describe search_docs/list_docs and
  the bundle contents, not just ping (any step).
- `&str` over `String` for query params; `sort_unstable` where applicable
  (step 3).

## Progress Log

Newest entry last. Every status change in the dashboard gets a line here.

- 2026-07-04 — Plan written and agreed (steps 1–5b approved, incl. ~8 KB
  page-boundary chunks, one file per chunk, 16-position cap). No code
  changed yet. Next action: begin step 1 (create src/lib.rs, move
  frontmatter/search code into modules, move extract.rs to src/bin/).
- 2026-07-04 — STEP 1 COMPLETE. Discovered the mcp-server repo had no
  commits; created baseline commit 836f1f4 first (15 tests green), then
  the split as commit 17967a8. New layout: lib.rs (+ test_util),
  frontmatter.rs (parse + new OkfFrontmatter::render + round-trip test),
  bundle.rs (knowledge_dir, list_docs_in), search.rs (all search logic +
  tests), main.rs (MCP wiring only), src/bin/extract.rs (uses shared
  frontmatter). Cargo.toml: [[bin]] blocks removed, tempfile →
  dev-dependencies. Verified via docker exec into the running
  kuka-mcp-server container (project mounts at /workspaces/MCP-Rust):
  clippy clean except 3 pre-existing idiom warnings deliberately left for
  step 3 (trim_split_whitespace, unnecessary_sort_by ×2); 16/16 tests
  pass (15 baseline + 1 new round-trip). Lessons written:
  lessons/refactor-01-lib-and-bin-split.html,
  lessons/refactor-02-shared-frontmatter-module.html.
  Next action: ask user for permission to start step 2 (Document type).
- 2026-07-04 — STEP 2 implemented on branch refactor/step-2-document-type
  (commit d0b9f07). Document struct + load_bundle in bundle.rs; all three
  consumers (list_docs_in, search_docs_in, list_resources) now use it.
  Missing bundle dir → CallToolResult::error / protocol error (was: fake
  "no documents"); unreadable files → tracing::warn + skip; extension
  check case-insensitive; read_resource guarded by resource_stem_is_safe
  (path traversal). 21/21 tests (5 new). Manually verified over live MCP
  stdio in devcontainer: list_docs on real bundle OK, missing-dir error
  OK, ../../etc/passwd rejected OK. Lessons refactor-03-document-type
  and refactor-04-errors-that-lie written. PR opened for user review —
  step is complete when the user merges it. Next action after merge:
  flip dashboard to complete, then ask permission for step 3.
- 2026-07-04 — STEP 2 COMPLETE. User merged PR #1 as merge commit 948a356
  (merge-commit strategy agreed as the standing choice — it preserves the
  commit hashes cited in lessons/plan). Branch deleted local + remote.
  Next action: ask user for permission to start step 3 (SearchHit API,
  consts, LazyLock stop words, test rewrite) on branch
  refactor/step-3-searchhit-api.
- 2026-07-04 — STEP 3 implemented on branch refactor/step-3-searchhit-api
  (commit 793fa54). search.rs is pure domain logic (no rmcp): parse_query
  + search(docs, terms) -> Vec<SearchHit>; presentation (run_search /
  format_hit, wording, isError) moved to main.rs. Consts for all tuning
  knobs; STOP_WORDS in static LazyLock; clippy fully clean (0 warnings);
  27/27 tests (23 lib + 4 new bin-level; bin tests build own fixture —
  lib test_util invisible across crate boundary). Live MCP check: output
  format identical. FINDING (data, not code): the only real bundle doc,
  knowledge/emergencyfirealarm.md, has an EMPTY BODY (213 bytes, front-
  matter only — bad extraction); it matches via frontmatter title and
  yields an empty fallback excerpt "......"; pre-existing behavior, fix
  is re-running the extractor on that PDF. Lessons refactor-05 and
  refactor-06 written. PR opened; step complete when user merges.
  Next action after merge: ask permission for step 4 (config into
  KukaServer).
- 2026-07-04 — STEP 3 COMPLETE. User merged PR #2 (merge commit 1ace0b2).
  Branch deleted local + remote; local master synced. Reminder for the
  workflow: after the user merges on GitHub, the local repo must
  checkout master + pull + delete the branch — merging does not move the
  local checkout. Outstanding side item: re-extract empty
  knowledge/emergencyfirealarm.md (task chip raised). Next action: ask
  user for permission to start step 4 (config into KukaServer) on
  branch refactor/step-4-server-config.
- 2026-07-04 — LINE ENDINGS SETTLED (PR #3, merged 33ea4cb). The tree is
  shared between Windows host (was autocrlf=true) and the Linux
  devcontainer, which made container git show phantom "modified" files.
  Fixed: .gitattributes pins `* text=auto eol=lf`; host repo config now
  autocrlf=false; all tracked files physically LF on disk; both sides
  report clean; 27/27 tests pass. NOTE for future agents: if phantom
  modified files reappear with an empty `git diff`, it is the index stat
  cache — `git restore .` clears it. Prefer running git from the host;
  avoid git commands as root inside the container (they can wedge the
  shared .git/index ownership/stat data).
- 2026-07-04 — STEP 4 implemented on branch refactor/step-4-server-config
  (commit 8b029e8). KukaServer { knowledge_dir: PathBuf, tool_router };
  main() reads KUKA_KNOWLEDGE_DIR exactly once and logs it; free
  bundle::knowledge_dir() deleted. Folded in the smaller-fixes item:
  get_info instructions now describe search_docs/list_docs/resources.
  New bin test constructs a fully-wired KukaServer against a temp dir
  and calls the tool methods directly. 28/28 tests (23 lib + 5 bin),
  clippy clean. Live MCP check from /tmp with KUKA_KNOWLEDGE_DIR set:
  bundle found, new instructions served. Lesson refactor-07 written.
  PR opened; step complete when user merges. Next after merge: ask
  permission for step 5a (chunking in extract) on branch
  refactor/step-5a-chunking.
- 2026-07-04 — STEP 4 COMPLETE. User merged PR #4 (merge commit e221860).
  Branch deleted local + remote; master synced; tree clean. Steps 1-4
  all complete — the architecture is ready for step 5. Next action: ask
  user for permission to start step 5a (page-boundary chunking in
  extract, ~8 KB target, one file per chunk with parent/pages
  frontmatter).
- 2026-07-04 — STEP 5A implemented on branch refactor/step-5a-chunking
  (commit a79e531). New lib module chunk.rs (chunk_pages: form-feed page
  split, ~8 KB accumulation, paragraph sub-split for oversized pages,
  empty pages skipped w/ numbering kept). extract.rs writes one OKF file
  per chunk with parent/pages frontmatter; OkfFrontmatter gained
  Option<String> parent/pages; jiff for real timestamps; bails on empty
  extraction; within-run collision warning; case-insensitive .pdf.
  36/36 tests, clippy clean. REAL BUNDLE REBUILT (knowledge/ is
  gitignored — derived data): 9/10 PDFs extracted, MQTT Configuration →
  2 chunks, MQTT Payload Definitions → 3 chunks; live search shows
  page-provenance titles. EmergencyFireAlarm.pdf is IMAGE-ONLY (both
  extractors yield nothing) — extractor now refuses it loudly; stale
  empty knowledge/emergencyfirealarm.md deleted; fixing that doc = OCR,
  out of scope. Lesson refactor-08-page-chunking written. PR opened;
  step complete when user merges. Next after merge: ask permission for
  step 5b (inverted index) on branch refactor/step-5b-inverted-index.
- 2026-07-04 — STEP 5A COMPLETE. User merged PR #5 (merge commit
  a6a813e). Branch deleted local + remote; master synced; tree clean.
  Only step 5b (inverted index) remains. Next action: ask user for
  permission to start step 5b on branch refactor/step-5b-inverted-index
  (Index struct per plan §5b: vocab HashMap, postings with 16-position
  cap, index-time boilerplate filtering, per-token lowercasing with
  original byte offsets, seek-based excerpts, Arc<RwLock<Index>> in
  KukaServer, reload_docs tool).
- 2026-07-04 — STEP 5B implemented on branch
  refactor/step-5b-inverted-index (commit 2072866). index.rs per plan:
  vocab HashMap -> postings (doc_id, freq, positions capped at 16),
  bodies dropped after build, boilerplate filtered at build time,
  per-token lowercase keys with ORIGINAL byte offsets (offset hazard
  eliminated by construction), vocabulary-level substring+fuzzy term
  matching, length-normalised x1000 scoring, seek-based 450-byte
  excerpt reads. KukaServer { knowledge_dir, index: Arc<RwLock<Index>> };
  reload_docs tool (failure keeps old index); startup fails fast +
  logs docs/terms/build-time. search.rs slimmed to shared pieces;
  bundle::list_docs_in deleted (listing formats from index metadata in
  main.rs); parse_query splits on non-alphanumeric to match tokenizer.
  BEHAVIOR NOTES vs linear engine: fuzzy hits now carry real positions
  (score > 0, genuine excerpts); exact matching is token-substring
  rather than whole-text-substring; missing dir errors at startup/
  reload instead of per-call. 42/42 tests (36 lib + 6 bin), clippy
  clean. Live: 12 docs / 1051 terms in 23ms; step-5a benchmark query
  returns same 4 docs in same order with better excerpts; reload_docs
  verified incl. failure path; empty result for a term absent from the
  corpus verified honest. Lessons refactor-09 (inverted index) and
  refactor-10 (Arc/RwLock/reload) written. PR opened; step complete
  when user merges. After merge the PLAN IS FULLY COMPLETE except
  deferred 5c (tantivy/hybrid escape hatch, trigger conditions in §5c).
- 2026-07-04 — STEP 5B COMPLETE — PLAN COMPLETE. User merged PR #6
  (merge commit 4732155). Branch deleted local + remote; master synced;
  tree clean. All steps 1-5b done across 6 PRs with 10 refactor lessons
  (refactor-01 .. refactor-10) and 42 passing tests. Only 5c remains,
  deliberately deferred: revisit if index build exceeds a few seconds
  or fuzzy vocab scans slow down (then evaluate tantivy / SQLite FTS5,
  or hybrid vector retrieval per the architecture discussion). Other
  known follow-ups, none urgent: EmergencyFireAlarm.pdf needs OCR to be
  indexable; ranking could add the IDF half of TF-IDF; index
  persistence only matters at much larger corpus scale.
- 2026-07-05 — STEP 6 implemented on branch
  refactor/step-6-clean-extraction (stacked on docs/user-manual, so its
  PR shows base = docs/user-manual until PR #7 merges). Scope in §6
  above. Code: clean_extracted_text + 4 tests in chunk.rs (gotcha
  found: str::lines() doesn't split on \x0c, so page breaks are mapped
  to \n before repetition counting); extract.rs cleans before chunking
  + pdftotext -layout; SearchHit.resource → SearchHit.stem; format_hit
  emits Resource: kuka://docs/{stem}; sharpened search_docs description
  + get_info workflow instructions; CLAUDE.md added. 46/46 tests
  (40 lib + 6 bin), clippy clean. Bundle re-extracted (files now clean;
  chunk boundaries shifted slightly due to -layout); debug binary
  rebuilt (client config points at it). Live verify of the exact
  failing query: excerpts anchor on real content incl. readable table
  rows, resource URI shown, no .pdf paths in output. Docs updated:
  USER-MANUAL (capabilities, extraction/cleaning, other-formats
  section, results-reading section, tools table), lessons refactor-08
  and refactor-09 got update notes, new lesson refactor-11
  (clean extraction + agent steering). PR opened; step complete when
  user merges (after #7).
- 2026-07-05 — STEP 6 MERGE MISHAP + RESTORATION + ADDITIONS. PR #8 was
  stacked on PR #7 (base = docs/user-manual). The user merged #7 first
  and #8 second — but #8 merged into the docs/user-manual BRANCH, whose
  content had already been copied to master, so step 6 never reached
  master (GitHub shows #8 as MERGED regardless). LESSON FOR STACKED
  PRs: after the bottom PR merges, confirm GitHub retargeted the upper
  PR's base to master BEFORE merging it. Recovery: step-6 commit
  53a4973 restored from the local repo and rebased onto master as
  1f81396 on branch refactor/step-6-clean-extraction. Added on top
  (user-approved): (1) the no-results message now carries in-band retry
  guidance ("All search terms must match — try again with fewer or
  different terms") — tool output is the only steering channel that
  reaches EVERY harness (Codex etc.), unlike instructions/CLAUDE.md;
  (2) AGENTS.md mirroring CLAUDE.md for non-Claude coding harnesses,
  with cross-references to keep the two in sync. New PR opened against
  master; step 6 complete when it merges.
- 2026-07-05 — STEP 6 COMPLETE (for real). User merged PR #9 (e601cb3).
  This time VERIFIED on master before cleanup: both commits are
  ancestors, CLAUDE.md/AGENTS.md/refactor-11 present, retry hint in
  main.rs. Branch deleted local + remote; tree clean. All steps 1-6
  complete; deferred items unchanged (5c escape hatch, IDF ranking,
  index persistence, OCR for EmergencyFireAlarm.pdf, pandoc DOCX/PPTX
  ingestion as next capability).
- 2026-07-05 — STEP 7 implemented on branch
  refactor/step-7-zone-boilerplate (scope in §7 above). chunk.rs:
  zone-based clean_extracted_text (3-pass: count + zone stats →
  classify → filter) + is_page_marker slice-pattern matcher + new tests
  incl. the RobotType scenario (repeated mid-page table survives all 3
  sections) and page-marker shape matching. index.rs: repeated-lines
  filter removed, build simplified to single tokenize pass, tests
  rewritten to the trust-the-bundle contract (incl. end-to-end
  clean→index test). search.rs: repeated_lines +
  BOILERPLATE_MIN_REPEATS deleted; normalize_line retained for the
  cleaner. 47/47 tests, clippy clean. Bundle re-extracted: 250P now
  present 3× in mqtt p009-015 chunk; debug binary rebuilt. Live:
  "RobotType robot family code" → full table in excerpt with resource
  URI; "RobotType valid values" → retry hint. Docs updated:
  USER-MANUAL (capabilities, pipeline, extraction bullets — position-
  aware wording, index "trusts the bundle"), refactor-09 and
  refactor-11 update notes corrected, new lesson
  refactor-12-repetition-is-not-identity. NOTE: a mid-command container
  restart produced one exit-137; harmless, container came back up.
  PR opened against master; step complete when user merges.
- 2026-07-05 — STEP 7 COMPLETE. User merged PR #10 (b15f39e). Note:
  the step-8 plan commit 031bfd4 was pushed to the PR branch after the
  merge snapshot and had to be cherry-picked onto master afterwards
  (d42d8b3) — lesson: don't push to a PR branch once the user may have
  merged; verify master after every merge. STEP 8 (OCR ingestion, §8)
  is designed and handed to a Codex agent for implementation on branch
  refactor/step-8-ocr-ingestion; house rules apply (PR to master, user
  merges, docs + lesson refactor-13 in same commit, devcontainer-only
  cargo, rebuild debug binary).
- 2026-07-06 — STEP 8 STARTED on branch refactor/step-8-ocr-ingestion
  after user confirmation that master contains the Step 8 plan section
  and Step 7 is complete. Scope: add ocrmypdf fallback in extract.rs
  only when normal extraction yields empty text; move tempfile to runtime
  dependencies; install/document ocrmypdf; re-extract EmergencyFireAlarm;
  verify clippy/tests/build plus live search_docs; write lesson
  refactor-13-ocr-fallback. Current compile/test status: not yet run on
  this branch. Next action: implement the extractor fallback and focused
  tests.
- 2026-07-06 — STEP 8 IMPLEMENTED on branch
  refactor/step-8-ocr-ingestion. Code: extract.rs now falls back to
  ocrmypdf only when raw normal extraction is empty, extracts the OCR PDF
  through existing pdftotext/clean/chunk/write flow, tags OCR output as
  [extracted, ocr, technical-note], and prefixes OCR body text with the
  source title so title-only terms such as "fire" are searchable without
  changing the server/index. Cargo.toml moves tempfile to runtime deps;
  devcontainer installs ocrmypdf. Docs: USER-MANUAL updated; lesson
  refactor-13-ocr-fallback written. Verification: ocrmypdf 16.7.0
  installed in running container; EmergencyFireAlarm.pdf re-extracted to
  knowledge/emergencyfirealarm-p001-006.md and -p007-009.md with non-empty
  OCR bodies and OCR tags; reload_docs reported 16 docs / 1240 terms;
  live search_docs("fire alarm") returned EmergencyFireAlarm with
  Resource: kuka://docs/emergencyfirealarm-p001-006; cargo clippy
  --all-targets clean; cargo test 49/49; cargo build rebuilt target/debug.
  PR #11 opened against master; step becomes complete only after user merge.
- 2026-07-06 — STEP 8 COMPLETE. Codex implemented per Claude's design;
  PR #11 merged (b6cbf23). Claude post-merge review: code matches the
  design (try_ocr via ocrmypdf --skip-text + tempfile, empty-text
  trigger after both extractor paths, differentiated bail message, ocr
  tag in both write paths, tempfile moved to [dependencies],
  devcontainer postCreate + manual install done). One benign addition
  beyond design: include_ocr_source_title prepends the file stem to
  OCR'd body text so title terms are searchable (unit-tested).
  VERIFIED: 49/49 tests, clippy clean, debug binary rebuilt;
  EmergencyFireAlarm.pdf now extracts via pdf-extract→pdftotext→OCR
  chain into 2 chunks with tags [extracted, ocr, technical-note];
  live query "emergency fire alarm" returns the doc with page range +
  resource URI. Known minor notes for the future: (1) OCR output of
  diagram-heavy slides contains recognition noise (expected; fuzzy
  matching compensates); (2) in the default path, a pdf-extract ERROR
  (vs empty Ok) propagates before OCR can run — fine for current
  corpus, could route errors to the fallback chain someday.
- 2026-07-06 — STEP 9A STARTED on branch
  refactor/step-9a-office-ingestion after confirming master contains the
  9a dashboard row from f1f1c0b and Step 8 is complete. Scope: route
  PDF/Office/Text inputs in extract.rs; convert Office docs to temporary
  PDFs via soffice and reuse the existing PDF text/OCR pipeline; ingest
  TXT directly through clean/chunk/write; preserve original source
  filename in frontmatter; update devcontainer, USER-MANUAL, and lesson
  refactor-14. Hard boundary: do not touch main.rs, index.rs,
  frontmatter.rs, or chunk.rs. Current compile/test status: not yet run
  on this branch. Next action: implement extension routing and shared
  write path.
- 2026-07-06 — STEP 9A IMPLEMENTED on branch
  refactor/step-9a-office-ingestion. Code changes are confined to
  extract.rs: added IngestKind routing for pdf/docx/doc/pptx/ppt/txt;
  Office docs convert through soffice into a TempDir PDF and then use
  pdftotext/OCR/clean/chunk/write with original source provenance; TXT
  reads directly and uses the same clean/chunk/write tail; unsupported
  files still skip silently in batch mode and error in single-file mode.
  Devcontainer now installs libreoffice-writer + libreoffice-impress.
  Docs: USER-MANUAL updated and lesson refactor-14-office-ingestion
  written. Verification: LibreOffice 25.2.3.2 installed in running
  container; real DOCX "building map and extension map.docx" extracts
  into page-ranged chunks with resource: kuka-docs/building map and
  extension map.docx; full-directory extraction reported 11 extracted,
  0 failed; reload_docs reported 18 docs / 1551 terms; live
  search_docs("mapping route loop closure") returned the DOCX-derived
  chunk with Resource: kuka://docs/building-map-and-extension-map-p001-014;
  cargo clippy --all-targets clean; cargo test 51/51; cargo build
  rebuilt target/debug. PR #12 opened against master; step completes only
  after user merge.
- 2026-07-06 — STEP 9A COMPLETE. Codex implemented per Claude's design;
  PR #12 merged (0a035ab). Claude post-merge review: matches design —
  IngestKind routing (pdf/docx/doc/pptx/ppt/txt, case-insensitive,
  unit-tested), convert_office_to_pdf via soffice headless (existence
  check for silent-failure case), write_document extracted with source
  identity from the ORIGINAL file (resource: points at the .docx, not
  the temp PDF), txt path reads directly, Office path forces pdftotext
  so page separators survive into chunking (good judgement call). No
  scope creep into 9b territory. VERIFIED: 51/51 tests (4 extract bin
  tests), clippy clean, soffice installed, debug binary rebuilt;
  "building map and extension map.docx" (33 pages) → 2 chunks with
  correct provenance; live query returns it with page range + resource
  URI. Next: 9b (diagrams) — Claude, awaiting user approval.
- 2026-07-06 — STEP 9B implemented on branch refactor/step-9b-diagrams
  (user approved; Claude implementing per §9 split). extract.rs:
  try_extract_page_images via pdfimages -png -p (size floor 10 KB drops
  logos/header graphics, byte-hash dedupe via DefaultHasher for
  graphics repeated across pages, cap 20/doc), images written to
  knowledge/images/<slug>-pNNN-n.png, named after the ORIGINAL doc for
  Office sources; failure logs a warning and never blocks ingestion.
  OkfFrontmatter gains optional images: list; Document/DocMeta/
  SearchHit thread it through. main.rs: hits gain "Diagrams:
  kuka://images/..." lines; list_resources includes image resources
  (image/png); read_resource serves base64 blobs via
  ResourceContents::blob + with_mime_type (base64 crate added);
  traversal guard reused; instructions updated. 53/53 tests, clippy
  clean, debug binary rebuilt. Full re-extraction: 11/11 docs, 50
  diagrams kept across 6 docs. Live verify: fire-alarm hit advertises
  8 diagram URIs; resources/read returns mimeType image/png with PNG
  magic bytes in base64; traversal attempts rejected. Docs updated
  (USER-MANUAL capabilities/extraction/results sections), lesson
  refactor-15-serving-diagrams written. PR opened; step complete when
  user merges.
- 2026-07-06 — STEP 10 DESIGNED (not started). Codex-ready design doc at
  designs/step-10-streamable-http-transport.md: optional --http flag
  (stdio stays the no-flag default), rmcp transport-streamable-http-server
  feature + axum mount at /mcp, one shared Arc<RwLock<Index>> across all
  sessions, loopback-only binding with no auth (claude.ai/public HTTPS +
  OAuth explicitly out of scope; tunnel for testing), curl-level
  acceptance sequence included, devcontainer port forward 8382, lesson
  refactor-16. Committed on the open 9b branch (rides PR #13). NEXT
  SESSION (possibly Opus): after PR #13 merges, close out 9b (dashboard
  flip + branch cleanup), then hand the design doc to Codex for step 10
  with user approval per protocol.
- 2026-07-06 — 9B RIDER (user-approved, on the open PR #13 branch):
  MAX_IMAGES_PER_DOC raised 20→40 and the cap check moved after the
  quality filters so truncation is counted and WARNED, never silent
  (verification had shown the building map's pages 15-33 chunk starved
  of images at cap 20; the doc has 54 qualifying diagrams — 40 kept,
  "skipped 14 more" now reported). 53/53 tests, clippy clean, binary
  rebuilt, bundle re-extracted.
- 2026-07-06 — User raised MAX_IMAGES_PER_DOC 40→60 and re-extracted the
  building map manually: 54/54 diagrams kept, no truncation warning.
  Committed on the PR #13 branch.
- 2026-07-06 — STEP 9B COMPLETE (verified, not just trusted the GitHub
  label). User merged PR #13 as merge commit 995c399 directly into
  master — no stacking this time, confirmed 3ce494f is an ancestor of
  master and MAX_IMAGES_PER_DOC: usize = 60 is present in the actual
  file on master. 53/53 tests still pass post-merge. Branch deleted
  local + remote. Also discovered/fixed along the way: CLAUDE.md/
  AGENTS.md changes only take effect in NEW sessions (loaded once at
  startup) — a long-running session kept using the old "read but don't
  display" behavior until the user started a fresh session, which then
  correctly opened a diagram image per the updated instructions. Next
  action: ask user for permission to hand designs/step-10-streamable-
  http-transport.md to Codex for step 10.
- 2026-07-06 — STEP 10 HANDED TO CODEX. User confirmed intent to give
  Codex the design doc directly; Claude pre-created and pushed the
  branch (refactor/step-10-http-transport, off master @ 8ae84dd) so
  Codex starts clean per the doc's "Orientation" section. Codex should
  work on this branch, open a PR to master when done. Next action:
  wait for Codex's PR, then the usual review (verify content actually
  landed on master, not just the merge label; run tests; check the
  curl acceptance sequence; flip dashboard; clean up branch).
- 2026-07-12 — STEP 10 IMPLEMENTED on branch refactor/step-10-http-transport.
  mcp-server now accepts optional `--http <addr>`; no flag still uses stdio.
  Added rmcp streamable HTTP feature, axum 0.8 mount at `/mcp`, tokio net,
  clap Args tests, loopback/no-auth warning, devcontainer port 8382, manual
  HTTP section, and lesson refactor-16-streamable-http.html. Verification:
  local devcontainer cargo path used because `docker` is unavailable inside
  this VS Code container. `cargo clippy --manifest-path mcp-server/Cargo.toml
  --all-targets` clean; `cargo test --manifest-path mcp-server/Cargo.toml`
  passed 55/55; debug binary rebuilt. Live HTTP curl initialize returned
  Mcp-Session-Id and `search_docs` for "mission status payload" returned
  Found 4 result(s); no-flag stdio printf-pipe returned the same Found 4
  result(s). Next action: review diff, commit, push, and open PR to master.
