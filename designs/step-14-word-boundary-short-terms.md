# Design: Step 14 — Word-boundary-aware short-term matching (Codex-ready)

Stops 3-character query terms like `"red"` from raw-substring-matching
unrelated words that merely *contain* them ("wi**red**", "powe**red**",
"requi**red**"), while keeping the legitimate short-word matches that
substring containment was there for ("amr" → "amrs"). Written by Claude for
implementation by Codex (or any agent). The last step of the four-step
search-fix arc (11 soft-AND, 12 result bounds, 13 chunk continuity — all
merged). Small and self-contained: one function in index.rs, one new
constant in search.rs.

## Orientation (read first)

Read `AGENTS.md` and `REFACTOR-PLAN.md` (dashboard row 14, §14) before
starting. No preconditions — steps 11-13 are merged. Branch
`refactor/step-14-word-boundary-short-terms` off master; PR to master; the
user merges. **Ask the user before starting**; flip row 14 to `in progress`
+ log entries per protocol. cargo runs ONLY in the devcontainer:
`docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo <cmd>"`
(do NOT use `docker exec -w` — it fails in this environment; see AGENTS.md).

## Why (confirmed live, not hypothetical)

From a real trace: the query `"indicator light red yellow green"` matched
the page-22 chunk of the KMF manual — but grepping the actual bundle file
showed page 22 contains **no color "Red" entry at all** (that table row
lives in the next chunk). The "match" came from `matching_keys`'s
`key.contains(term)` matching `"red"` inside the vocab keys `"wired"`
("Wired remote controller") and `"powered"` ("the battery is powered on").
The result happened to be the right chunk by coincidence (it genuinely
contains "yellow"/"green"/"indicator"/"light") — but the same mechanism
gives *false coverage credit* everywhere: any English past-tense verb whose
stem ends in "r" contains "red" (fired, hired, covered, required, offered,
measured, configured...), and under step 11's coverage ranking, spurious
term-coverage now directly inflates a document's rank.

Current code (`matching_keys`, index.rs ~line 149):

```rust
fn matching_keys(&self, term: &str) -> Vec<&str> {
    self.vocab
        .keys()
        .filter(|key| {
            if term.len() < MIN_SUBSTRING_TERM_LEN {
                key.as_str() == term
            } else {
                key.contains(term) || within_typo_tolerance(key, term)
            }
        })
        .map(String::as_str)
        .collect()
}
```

Step 12's `MIN_SUBSTRING_TERM_LEN = 3` gates only 1-2 character terms;
a 3-character term still gets raw `.contains()`. Raising the threshold to
4 would break `"amr"` (must keep finding `"amrs"`), so the fix has to be
smarter than a bigger number.

## The design decision (made here, per §14's instruction to pick)

Three-tier gate by term length, replacing the current two-tier one:

| Term length | Rule | Rationale |
|---|---|---|
| 1-2 chars | exact vocab-key match (UNCHANGED, step 12) | single digits/letters match everything |
| 3 chars (i.e. `< FUZZY_MIN_TERM_LEN`) | **prefix match with a bounded suffix**: `key.starts_with(term) && key.len() <= term.len() + SHORT_TERM_MAX_SUFFIX` | see below |
| 4+ chars | `contains` OR fuzzy (UNCHANGED) | long terms rarely false-match; recall matters more |

Why prefix-with-bounded-suffix for the middle tier:

- **Plain prefix** kills the dominant noise class — `"red"` can never match
  `"wired"`/`"powered"`/`"required"` (those don't *start* with "red") —
  while keeping inflections: `"amr"` → `"amrs"`, `"red"` → `"reds"`.
- **The suffix bound** (2 extra chars) closes the residual prefix
  loophole: without it, `"red"` would still match `"reduce"`, `"reduced"`,
  `"redundant"`. With `key.len() <= term.len() + 2`: `"amrs"` (+1) and
  `"reds"` (+1) pass, `"boxes"` from `"box"` (+2) passes, `"reduce"` (+3)
  and `"redundant"` (+6) are rejected. The bound is effectively "allow
  plural/inflection endings, reject different words that share a prefix."
- Exact-only for 3-char terms was rejected because real docs pluralize
  ("AMRs" appears throughout the fleet manuals), and prefix-unbounded was
  rejected because it just swaps end-of-word coincidences for
  start-of-word ones.

New constant in search.rs's tuning-knob block (next to
`MIN_SUBSTRING_TERM_LEN`):

```rust
/// For terms long enough to leave exact matching but too short for
/// substring containment (3 chars, between MIN_SUBSTRING_TERM_LEN and
/// FUZZY_MIN_TERM_LEN): the key must START with the term and exceed it by
/// at most this many characters. Allows plural/inflection endings
/// ("amr" → "amrs") while rejecting different words sharing a prefix
/// ("red" → "reduce") and — the original bug — words merely containing
/// the term ("red" in "wired", "powered").
pub(crate) const SHORT_TERM_MAX_SUFFIX: usize = 2;
```

Updated `matching_keys`:

```rust
fn matching_keys(&self, term: &str) -> Vec<&str> {
    self.vocab
        .keys()
        .filter(|key| {
            if term.len() < MIN_SUBSTRING_TERM_LEN {
                key.as_str() == term
            } else if term.len() < FUZZY_MIN_TERM_LEN {
                key.starts_with(term) && key.len() <= term.len() + SHORT_TERM_MAX_SUFFIX
            } else {
                key.contains(term) || within_typo_tolerance(key, term)
            }
        })
        .map(String::as_str)
        .collect()
}
```

Also update the function's doc-comment (it currently describes two tiers)
to describe the three-tier gate.

**Not in scope:** any change to 4+ character matching (`"light"` in
`"flight"` is a real but rarer noise source — revisit only if a trace shows
it misleading an agent), stemming libraries, and the tokenizer.

## Goal / acceptance criteria

1. A document whose only "red"-related tokens are words *containing* "red"
   ("wired", "powered", "reduced") gets NO match — and therefore no
   coverage credit — for the query term `"red"`.
2. Documents with genuine `red` / `reds` tokens still match `"red"`.
3. `"amr"` still matches documents containing `KUKA.AMR` (token `amr`) and
   `AMRs` (token `amrs`) — the existing `search_substring_matches_short_terms`
   test must pass unchanged.
4. Live, against the real bundle: `search_docs("red")` no longer returns
   the p022-022 chunk (whose body has "Wired"/"powered" but no color-Red
   row — verified by grep during the trace analysis), while p023-024 (the
   chunk with the genuine "Red / Solid on" table row) still returns.

## Tests

index.rs `mod tests`:

- `short_terms_match_prefix_inflections_not_containing_words` (or split
  into two tests): a bundle with one doc containing only
  "wired powered reduced redundant" and another containing "red reds" —
  query `"red"` matches ONLY the second doc. Assert both directions.
- Regression: `search_substring_matches_short_terms` unchanged; extend it
  (or add a sibling) so the fixture also contains "AMRs" and asserts the
  plural still matches via the new prefix rule.
- `matching_keys_requires_exact_match_below_minimum_term_length` (step 12's
  test) must pass unchanged — the 1-2 char tier is untouched.

search.rs: no behavior change (`within_typo_tolerance` untouched), no new
tests required there beyond the constant existing.

## Verification (exact commands, run in the devcontainer)

```bash
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo clippy --all-targets"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo test"
docker exec kuka-mcp-server bash -c "cd /workspaces/MCP-Rust/mcp-server && cargo build"
```

Live acceptance (stdio JSON-RPC, same pattern as steps 11-13):
1. `search_docs("red")` → result list does NOT include
   `ba_kmf_1500p-cb_series_en-20250512-p022-022`, DOES include
   `…p023-024`.
2. `search_docs("amr fleet")` (or similar) → still returns fleet-manual
   hits, confirming no short-term recall regression on the real corpus.
3. Optional sanity: `search_docs("indicator light red yellow green")` →
   p023-024 (genuine red+yellow rows) should now rank at/above p022-022
   rather than below it, since p022-022 no longer gets spurious "red"
   coverage.

## Documentation (same commit — house rule)

- **USER-MANUAL.md** §7 "How to read search results": extend the
  short-terms bullet — 1-2 character terms match exact words only
  (unchanged); 3-character terms match words that *start* with the term
  and are at most 2 characters longer (plurals like "AMRs" match; "red"
  no longer matches "wired" or "reduce").
- **REFACTOR-PLAN.md**: row 14 + progress-log entries (start, implemented,
  complete — per the standing handoff protocol).
- **Lesson `lessons/refactor-20-word-boundary-matching.html`** per the
  standing template (`../assets/style.css`, lesson-header, WHY prose,
  before/after Rust from the actual commit, **equivalent Java samples** —
  natural angles: `String.contains` vs `startsWith` + length guard;
  tiered matching as a chain of predicates vs Java's `Predicate.or()`;
  how a tiny relevance bug becomes a ranking bug once coverage scoring
  exists). Cross-link refactor-17 (where MIN_SUBSTRING_TERM_LEN was born)
  and refactor-18 (coverage ranking, which amplified this bug); add the
  forward link FROM refactor-19 to this lesson.
