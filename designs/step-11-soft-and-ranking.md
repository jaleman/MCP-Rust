# Design: Step 11 — Soft-AND / coverage-ranked matching (Codex-ready)

Stops `search_docs` from returning an empty result the moment ONE query
term out of several fails to match anywhere, even when the rest of the
query would have found the right document. Written by Claude for
implementation by Codex (or any agent). Independent of steps 12-14 (12 is
merged; 13/14 are separately designed, not started) — touches a different
function in index.rs (`search`, not `matching_keys`) so branches should
rebase cleanly regardless of order.

## Orientation (read first)

Read `AGENTS.md` and `REFACTOR-PLAN.md` (dashboard row 11, §11) before
starting. No preconditions — independent of every other queued step.
Branch `refactor/step-11-soft-and-ranking` off master; PR to master; the
user merges. **Ask the user before starting**; flip row 11 to `in progress`
+ log entries per protocol. cargo runs ONLY in the devcontainer
(`docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo …"`
— container name `kuka-mcp-server`, mounted at `/workspaces/MCP-Rust`;
cargo is not installed on the Windows host). Note: `docker exec -w <path>`
has been unreliable in this environment (`Cwd must be an absolute path`
even with an absolute path) — use `bash -c "cd ... && cargo ..."` instead.

## Why (concrete failures observed live, not hypothetical)

Two independent trace analyses of real agent sessions against this server
both hit the same root cause repeatedly. Quantified from the second trace:
**10 of 13 `search_docs` calls (77%) returned nothing useful**, most of them
because ONE guessed word in an otherwise-reasonable multi-word query didn't
happen to appear in the target chunk. Concrete examples:

- `"three-color indicator light meaning"` → 0 results. "meaning" appears
  nowhere near the actual table (which just says "Color / State / AMR
  Status"), so it alone erased what "three-color indicator light" would
  have found.
- `"three-color light tower signal column"` (from the first trace) → 0
  results. "three color light" would have hit the correct part-list page
  (which literally says "Three-color indicator light"), but the single
  absent word "tower" zeroed the whole result.
- Six consecutive queries hunting for a table's continuation (`"maintenance
  mode indicator lights red solid"`, `"Wi-Fi disconnected indicator"`, etc.)
  all failed for the same reason — each guessed one word that wasn't
  actually in the target chunk's text.

Root cause in code, confirmed by reading `Index::search` (index.rs
~line 156-178): every parsed query term gets its own doc-matching pass, and
if ANY one of those passes matches zero documents, the function returns
`Vec::new()` immediately (line ~173-175) — before the other terms' matches
are ever consulted. Documents matching every term but one are discarded
identically to documents matching nothing at all.

## Goal / acceptance criteria

1. A multi-word query where all but one term matches a real document no
   longer returns empty — that document surfaces, ranked appropriately.
2. A query where every term matches the same document still ranks that
   document at or near the top (full coverage should generally beat partial
   coverage) — do not regress `search_ranks_focused_document_first` or the
   other existing ranking tests.
3. A query where NO term matches anything, anywhere, still returns empty
   with the existing retry-hint message unchanged (`search_returns_no_hits_
   for_unknown_query`, `search_tool_reports_no_results_with_retry_hint`) —
   this behavior must NOT regress; a totally-alien query should still say so
   honestly, not dredge up irrelevant documents.
4. All existing tests pass unchanged except where a test's docstring
   literally asserts the old strict-AND behavior (see Tests section) — those
   get superseded with an explanatory note, not silently deleted.

## Design

### Current behavior (index.rs `search`, ~line 156-178)

```rust
pub fn search(&self, terms: &[&str]) -> Vec<SearchHit> {
    if terms.is_empty() {
        return Vec::new();
    }

    let mut per_term: Vec<HashMap<u32, (u32, Vec<u32>)>> = Vec::new();
    for term in terms {
        let mut docs_for_term: HashMap<u32, (u32, Vec<u32>)> = HashMap::new();
        for key in self.matching_keys(term) {
            for posting in &self.vocab[key] {
                let entry = docs_for_term.entry(posting.doc_id).or_default();
                entry.0 += posting.freq;
                entry.1.extend(&posting.positions);
            }
        }
        // AND semantics: a term that matches nowhere empties the result
        if docs_for_term.is_empty() {
            return Vec::new();
        }
        per_term.push(docs_for_term);
    }

    // Intersect: candidate documents contain every term.
    let mut candidates: Vec<u32> = per_term[0]
        .keys()
        .copied()
        .filter(|id| per_term.iter().all(|m| m.contains_key(id)))
        .collect();
    candidates.sort_unstable();

    let mut hits: Vec<SearchHit> = Vec::new();
    for doc_id in candidates {
        let meta = &self.docs[doc_id as usize];
        let freq_total: u32 = per_term.iter().map(|m| m[&doc_id].0).sum();
        let per_term_positions: Vec<&Vec<u32>> =
            per_term.iter().map(|m| &m[&doc_id].1).collect();
        // ... anchors / proximity scoring / excerpts (UNCHANGED, see below) ...
        hits.push(SearchHit { /* ... score: freq-based ... */ });
    }

    hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    hits
}
```

### Target behavior

Two changes, both confined to the top and bottom of this one function —
the anchors/proximity/excerpt-reading block in the middle is UNCHANGED
(it already iterates `per_term_positions.len()`, not `terms.len()`, so it
tolerates a shorter list without modification):

1. **Don't early-return on a term with zero matches.** Push its (empty)
   map into `per_term` and keep going — a term matching nowhere should
   contribute nothing to a document's score, not annihilate the whole
   query.
2. **Union instead of intersect, ranked by coverage.** Candidates become
   every doc_id appearing in ANY per-term map (not every map). Only return
   fully empty when NO term matched anywhere (this is what still triggers
   the "No results" / retry-hint message in main.rs — no main.rs changes
   needed, since that check is just `hits.is_empty()`).
3. Replace `m[&doc_id]` direct indexing (which now panics for docs absent
   from some per-term maps) with `.get(&doc_id)` and fold in a per-document
   **coverage** count — how many distinct terms matched this document.
4. Sort by `(coverage DESC, score DESC)` — documents matching every term
   still rank first; partial matches are ordered by how much they cover,
   then by the existing length-normalised frequency score.

Sketch (adapt to actual surrounding code — the anchors/excerpts block
between the coverage/freq computation and the `hits.push` is unchanged from
today and is elided here for brevity):

```rust
pub fn search(&self, terms: &[&str]) -> Vec<SearchHit> {
    if terms.is_empty() {
        return Vec::new();
    }

    let mut per_term: Vec<HashMap<u32, (u32, Vec<u32>)>> = Vec::new();
    for term in terms {
        let mut docs_for_term: HashMap<u32, (u32, Vec<u32>)> = HashMap::new();
        for key in self.matching_keys(term) {
            for posting in &self.vocab[key] {
                let entry = docs_for_term.entry(posting.doc_id).or_default();
                entry.0 += posting.freq;
                entry.1.extend(&posting.positions);
            }
        }
        // Soft-AND: a term matching nowhere contributes nothing to any
        // document's coverage/score, but no longer erases the whole query.
        per_term.push(docs_for_term);
    }

    // Candidates: every document matched by AT LEAST ONE term. Coverage
    // (below) is what makes full-AND documents still win — this is a
    // ranking change, not a recall-only relaxation.
    let mut candidates: Vec<u32> = per_term.iter().flat_map(|m| m.keys().copied()).collect();
    candidates.sort_unstable();
    candidates.dedup();

    if candidates.is_empty() {
        return Vec::new();
    }

    // (coverage, SearchHit) pairs; coverage drives the final sort and is
    // discarded before returning — no public API change.
    let mut ranked: Vec<(usize, SearchHit)> = Vec::new();
    for doc_id in candidates {
        let meta = &self.docs[doc_id as usize];

        let coverage = per_term.iter().filter(|m| m.contains_key(&doc_id)).count();
        let freq_total: u32 = per_term
            .iter()
            .filter_map(|m| m.get(&doc_id))
            .map(|(freq, _)| freq)
            .sum();
        let per_term_positions: Vec<&Vec<u32>> = per_term
            .iter()
            .filter_map(|m| m.get(&doc_id).map(|(_, positions)| positions))
            .collect();

        // --- anchors / proximity scoring / excerpt reading: UNCHANGED,
        // copy verbatim from the current function; it already operates on
        // per_term_positions.len(), which may now be < terms.len() ---

        ranked.push((coverage, SearchHit {
            title: meta.title.clone(),
            stem: meta.stem.clone(),
            images: meta.images.clone(),
            score: (freq_total as usize * 1000) / meta.token_count.max(1),
            excerpts, // from the unchanged block above
        }));
    }

    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.score.cmp(&a.1.score)));
    ranked.into_iter().map(|(_, hit)| hit).collect()
}
```

Update the function's doc-comment (currently "Every term must match at
least one vocabulary key in a document for it to qualify (AND semantics, as
before)") to describe soft-AND/coverage semantics instead.

**Not in scope** (keep the change minimal): no change to `SearchHit`'s
public fields, no change to `matching_keys`, no change to `run_search` in
main.rs (the no-results branch is already exactly `hits.is_empty()`, which
remains correct — it now only fires when literally no term matched
anywhere). A "matched N/M terms" annotation on hits would be a reasonable
follow-up but is explicitly NOT required for this step — don't add it
unless trivial.

## Tests

Index.rs `mod tests` — existing tests to verify still pass UNCHANGED (they
exercise single-term or all-terms-present queries, so coverage/score
ordering collapses to the old behavior):
`search_finds_matching_document`, `search_ranks_focused_document_first`,
`search_matches_typo_via_vocabulary`, `search_substring_matches_short_terms`,
`index_trusts_bundle_content_including_repeated_lines`,
`extract_cleaning_and_index_work_end_to_end` (its "manual header" case
returns empty because BOTH words are absent post-cleaning — still a
zero-coverage, zero-candidate case).

`search_returns_no_hits_for_unknown_query` (query `"hydraulic pump"`
against the reflector-guide fixture, where NEITHER word exists) must still
return empty — this is the "no term matched anywhere" case and must not
regress. Keep this test; add a NEW one alongside it for the case it used to
conflate:

- `search_surfaces_partial_match_instead_of_empty`: same fixture, query
  something like `"reflector hydraulic"` where "reflector" exists in the
  doc and "hydraulic" does not — assert the result is NOT empty, the
  Reflector Guide doc is returned, and its score is lower than (or equal
  to) what a full-coverage query for "reflector" alone would produce (or at
  minimum, just assert it's present and non-empty — don't over-specify the
  exact score value).
- `search_ranks_full_coverage_above_partial_coverage`: a synthetic
  two-document bundle where doc A contains both query terms and doc B
  contains only one; assert doc A ranks first regardless of doc B's raw
  frequency score (this is the test that actually exercises the
  coverage-before-score sort key — construct doc B to have a HIGHER
  frequency score than doc A on its one matching term, so the test would
  fail under score-only sorting and only pass once coverage is checked
  first).

Main.rs `tool_tests` — no changes expected (the no-results and
found-results branches are unchanged; existing `search_tool_reports_no_
results_with_retry_hint` and `search_tool_formats_hits` should keep passing
as-is). If they don't, that's a signal something in main.rs assumed
strict-AND and needs a look — flag it rather than silently adjusting the
test to match.

## Verification (exact commands, run in the devcontainer)

```bash
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo clippy --all-targets"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo test"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo build"
```

Live repro against the real bundle (stdio, same pattern as prior steps'
live checks) — re-run the exact failing queries from the trace analyses and
confirm they now return the previously-missed documents instead of "No
results found":

- `"three-color indicator light meaning"` — should now surface the
  components-list page (pages 9-15) and/or §2.2.9 (page 22), not empty.
- `"three-color light tower signal column"` — should now surface the
  components-list page via "three color light" coverage, despite "tower"
  and "column" matching nothing.
- Confirm a genuinely nonsense query (e.g. `"hydraulic pump zephyr"`,
  assuming none of those words are in the real bundle) still returns "No
  results found... try again with fewer or different terms" — the honest
  empty case must still work.

## Documentation (same commit — house rule)

- **USER-MANUAL.md** §7 "How to read search results": the bullet **"All
  terms must match (after stop-word removal). No results usually means one
  of your words appears nowhere in the bundle — drop or replace the rarest
  word and retry."** is no longer accurate and must be rewritten — soft-AND
  semantics mean documents matching MORE of the query terms rank higher,
  and "No results" now means NONE of the terms matched anywhere. Keep the
  guidance actionable (e.g., a query returning weak/irrelevant partial
  matches is a signal to add a more specific term, not that nothing was
  found).
- **REFACTOR-PLAN.md**: row 11 + progress-log entries (start, implemented,
  complete — per the standing handoff protocol).
- **Lesson `lessons/refactor-18-soft-and-ranking.html`** per the standing
  template (`../assets/style.css`, lesson-header block, WHY prose,
  before/after Rust from the actual commit, **equivalent Java samples**
  alongside — e.g. contrasting a Lucene/Elasticsearch `MUST` vs `SHOULD`
  boolean-query clause, or a Java `Comparator.comparing(...).thenComparing(...)`
  chain for the coverage-then-score sort, against Rust's `sort_by` closure).
