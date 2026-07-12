# Design: Step 13 — Surface chunk continuity (Codex-ready)

Makes multi-chunk documents navigable: when a search hit or resource read
lands on a chunk that is NOT the last chunk of its source document, the
output tells the agent where the text continues. Written by Claude for
implementation by Codex (or any agent). Independent of step 14 — touches
`Document`/`DocMeta`/`SearchHit` plumbing and presentation, not
`matching_keys`.

## Orientation (read first)

Read `AGENTS.md` and `REFACTOR-PLAN.md` (dashboard row 13, §13) before
starting. No preconditions — steps 11 and 12 are merged. Branch
`refactor/step-13-chunk-continuity` off master; PR to master; the user
merges. **Ask the user before starting**; flip row 13 to `in progress` +
log entries per protocol. cargo runs ONLY in the devcontainer:
`docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo <cmd>"`
(do NOT use `docker exec -w` — it fails in this environment with "Cwd must
be an absolute path"; see AGENTS.md).

## Why (concrete failure observed live, not hypothetical)

From a real agent trace: the agent found the *start* of the KMF §2.2.9
indicator-light table on chunk `ba_kmf_1500p-cb_series_en-20250512-p022-022`,
but the table continues in the separate chunk `…-p023-024`. Nothing in the
hit or the resource read said so. The agent burned SIX follow-up
`search_docs` queries guessing at the continuation's content (all failed),
then recovered only by calling `list_docs`, eyeballing the raw 69-document
listing, and inferring adjacency from the page ranges in the filenames.
The information it needed — "this chunk has a next chunk, here is its URI"
— exists on disk in every chunked file's frontmatter and is currently
discarded at load time.

Root cause, verified in code:
- `extract.rs` writes `parent:` (the un-chunked source document's slug) and
  `pages:` (this chunk's page range, e.g. `22-22`) into every chunk's
  frontmatter; `frontmatter.rs` (`OkfFrontmatter`, `extract_frontmatter_field`)
  fully supports both fields.
- `Document::load` (bundle.rs) never reads them; `DocMeta` (index.rs) never
  carries them. They exist on disk and nowhere else.

## Goal / acceptance criteria

1. A search hit whose chunk has a following chunk (same `parent:`, next
   page range) shows a `Continues: kuka://docs/{next-stem}` line. The LAST
   chunk of a document shows no such line, and single-chunk / hand-written
   documents (no `parent:`) are completely unaffected.
2. Reading a `kuka://docs/…` resource for a non-final chunk appends a
   clearly marked continuation pointer so an agent reading the full chunk
   also learns where the text continues.
3. Live acceptance test against the real bundle: a query that hits
   `…p022-022` (e.g. `"indicator light red yellow green"`) shows
   `Continues: kuka://docs/ba_kmf_1500p-cb_series_en-20250512-p023-024`,
   and reading the `…p022-022` resource shows the same pointer. Reading
   the FINAL chunk of that manual shows no pointer.
4. All existing tests pass; new tests cover the chain computation and both
   presentation surfaces.

## Design

### 1. Parse the fields (bundle.rs)

Add to `Document` (fields, doc-commented like the existing ones):

```rust
/// For chunked documents: the parent document slug and this chunk's page
/// range, from the optional `parent:` / `pages:` frontmatter (extract.rs
/// writes both on every chunk). None for single-file documents.
pub parent: Option<String>,
pub pages: Option<String>,
```

In `Document::load`, alongside the existing field extraction:

```rust
let parent = extract_frontmatter_field(&content, "parent");
let pages = extract_frontmatter_field(&content, "pages");
```

`extract_frontmatter_field` already handles these names (frontmatter.rs
has round-trip tests for exactly these two fields) — no frontmatter.rs
changes needed.

### 2. Compute the chain at index-build time (index.rs)

Add to `DocMeta`:

```rust
/// Stem of the chunk that continues this one (same parent, next page
/// range). None for final chunks and unchunked documents.
pub next_stem: Option<String>,
```

In `Index::build`, after the existing per-document loop has filled `docs`
(all DocMetas exist, so every neighbour is known), compute the chains:

```rust
// Group chunk indices by parent slug, ordered by starting page number.
// "22-22" → 22; a pages value whose leading number doesn't parse is
// skipped (that chunk simply joins no chain — degrade, don't fail).
let mut families: HashMap<String, Vec<(u32, usize)>> = HashMap::new();
for (i, doc) in docs.iter().enumerate() {
    if let (Some(parent), Some(pages)) = (&doc.parent_slug_field, &doc.pages_field) {
        if let Some(start) = pages.split('-').next().and_then(|p| p.trim().parse::<u32>().ok()) {
            families.entry(parent.clone()).or_default().push((start, i));
        }
    }
}
for (_, mut chunks) in families {
    chunks.sort_unstable();
    for pair in chunks.windows(2) {
        let (_, this_idx) = pair[0];
        let (_, next_idx) = pair[1];
        docs[this_idx].next_stem = Some(docs[next_idx].stem.clone());
    }
}
```

Implementation notes:
- The sketch above names the raw fields loosely — decide whether `DocMeta`
  keeps `parent`/`pages` as fields too or only consumes them transiently
  during build. Keeping them on `DocMeta` is fine (list_docs could use
  them later) but only `next_stem` is REQUIRED by this step.
- `windows(2)` needs the borrow untangled (can't mutate `docs` while
  iterating a structure borrowed from it) — collect the `(this_idx,
  next_stem)` pairs first, then apply. Or index arithmetic. Codex's
  choice; keep it simple.
- Chains are recomputed on every `reload_docs` for free since they're part
  of `Index::build`.

### 3. Thread into hits (search.rs + index.rs)

`SearchHit` (search.rs) gains:

```rust
/// Resource stem of the chunk that continues this one, when the matched
/// chunk is not the last of its source document. The presentation layer
/// renders it as a "Continues:" URI line.
pub continues: Option<String>,
```

`Index::search` (index.rs) copies `meta.next_stem.clone()` into it where
the other meta fields are cloned.

### 4. Presentation (main.rs)

`format_hit` — after the `Diagrams:` block, mirroring its
only-when-present pattern:

```rust
if let Some(next) = &hit.continues {
    out.push_str(&format!("\n  Continues: kuka://docs/{next}"));
}
```

`read_resource` — the docs branch currently returns raw file content read
straight from disk and never consults the index. After a successful read,
look up the stem's `next_stem` in the index and, when present, append a
clearly separated trailer to the returned text:

```rust
let continues = {
    let index = self.index.read().unwrap();
    index.docs().iter().find(|d| d.stem == stem).and_then(|d| d.next_stem.clone())
};
let content = match continues {
    Some(next) => format!(
        "{content}\n\n---\n[This section continues in kuka://docs/{next}]"
    ),
    None => content,
};
```

The trailer is metadata appended by the server, visually fenced off from
the document text — same philosophy as the retry hint riding inside the
no-results message: tool/resource OUTPUT is the one steering channel every
harness passes to its model.

### Not in scope

- No "Previous:" back-links (forward navigation is what the observed
  failure needed; add later if a trace ever shows backwards hunting).
- No changes to `list_docs` output or `list_resources` (the continuation
  is discoverable from hits and reads, which is where agents actually are
  when they need it).
- No changes to extract.rs or the bundle format — the frontmatter already
  carries everything needed.

## Tests

bundle.rs:
- Extend `document_load_parses_fields_and_body` (or add a sibling test):
  a fixture with `parent:` and `pages:` populates both fields; the
  existing fixture without them yields `None` for both.

index.rs:
- `build_computes_next_stem_chain`: synthetic three-chunk bundle (same
  `parent: fleet-manual`, pages `1-8`, `9-15`, `16-20`, written to the
  temp dir in NON-sorted filename order to prove ordering comes from
  `pages`, not directory order): chunk 1 → chunk 2 → chunk 3 → None.
  A fourth, unrelated single-file doc (no parent) has `next_stem: None`.
- `search_hit_carries_continues`: search the three-chunk bundle for a term
  in the middle chunk; the hit's `continues` is the third chunk's stem.

main.rs `tool_tests`:
- `search_tool_shows_continues_line`: two-chunk fixture; formatted output
  for a first-chunk hit contains `Continues: kuka://docs/<second-stem>`;
  a hit on the second (final) chunk does not contain `Continues:`.
- `read_resource_appends_continuation_trailer`: reading the first chunk's
  resource returns text ending with the `[This section continues in
  kuka://docs/<second-stem>]` trailer; reading the second chunk's resource
  returns the file content unmodified. (read_resource is async — follow
  the existing async test patterns in the file, or test through whatever
  seam the existing resource tests use.)

## Verification (exact commands, run in the devcontainer)

```bash
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo clippy --all-targets"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo test"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo build"
```

Live acceptance against the real bundle (stdio JSON-RPC, same pattern as
steps 11/12 — initialize, initialized, then tools/call):
1. `search_docs("indicator light red yellow green")` → the `…p022-022`
   hit carries `Continues: kuka://docs/ba_kmf_1500p-cb_series_en-20250512-p023-024`.
2. `resources/read` on `kuka://docs/ba_kmf_1500p-cb_series_en-20250512-p022-022`
   → text ends with the continuation trailer naming `…p023-024`.
3. `resources/read` on the LAST chunk of that manual (check `list_docs`
   for its highest page range) → no trailer.

## Documentation (same commit — house rule)

- **USER-MANUAL.md** §7 "How to read search results": new bullet for the
  `Continues:` line (chunked documents point to the next section when the
  match isn't the last chunk), and a sentence in the resources section
  about the appended continuation trailer.
- **REFACTOR-PLAN.md**: row 13 + progress-log entries (start, implemented,
  complete — per the standing handoff protocol).
- **Lesson `lessons/refactor-19-chunk-continuity.html`** per the standing
  template (`../assets/style.css`, lesson-header, WHY prose, before/after
  Rust from the actual commit, **equivalent Java samples** — natural
  angles: a linked-list-style `next` pointer computed by a group-and-sort
  pass vs. Java's `Collectors.groupingBy` + sort + neighbour assignment;
  Optional<String> vs Option<String> for the absent-next case). Cross-link
  refactor-18 (previous appendix) and refactor-08 (page chunking, where
  the parent/pages frontmatter was born); add the forward link FROM
  refactor-18 to this lesson.
