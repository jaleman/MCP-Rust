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
