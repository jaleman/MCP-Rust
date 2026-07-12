# Design: Step 12 — Bound worst-case search_docs output (Codex-ready)

Stops `search_docs` from ever returning a response too large for an agent's
context, and stops single/double-character query terms (numbers especially)
from matching almost the entire vocabulary. Written by Claude for
implementation by Codex (or any agent). Independent of step 11 (soft-AND
ranking) — can be done first or alone; touches a different function in
index.rs (`matching_keys`, not `search`'s AND logic) so the two steps should
rebase cleanly against each other either order.

## Orientation (read first)

Read `AGENTS.md` and `REFACTOR-PLAN.md` (dashboard row 12, §12) before
starting. No preconditions — steps 1-10 are all complete and this doesn't
depend on step 11. Branch `refactor/step-12-result-bounds` off master; PR to
master; the user merges. **Ask the user before starting**; flip row 12 to
`in progress` + log entries per protocol. cargo runs ONLY in the
devcontainer (`docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server
…` — the running container is named `kuka-mcp-server`, mounted at
`/workspaces/MCP-Rust`; cargo is not installed on the Windows host).

## Why (concrete failure observed live, not hypothetical)

Two independent, compounding defects, found by tracing a real query through
the actual code and the actual running server:

1. **No minimum term length before substring matching.** `parse_query`
   (search.rs) splits on non-alphanumeric characters with no length filter,
   so `"2.2.9"` tokenizes to terms `"2"`, `"2"`, `"9"`. `matching_keys`
   (index.rs, ~line 139) does `key.contains(term)` for every term against
   every vocabulary key — a single-digit term matches nearly any vocab key
   that contains that digit anywhere (dates like "2026", page/section
   numbers, part numbers), which is most of a 170-page manual's vocabulary.
2. **No cap on hits returned.** `run_search` (main.rs, ~line 139-171) does
   `hits.iter().map(format_hit).collect()` — every matching document gets
   formatted into the response, unconditionally.

Together: `search_docs("2.2.9")` against the real bundle returned an
84,646-character / 2,117-line response — unusable in an agent's context, and
the search term ("2.2.9", copied straight from a section number the user
mentioned) is entirely reasonable input, not an edge case.

## Goal / acceptance criteria

1. A short, digit-heavy query (e.g. `"2.2.9"`) returns a small, useful
   response — either a tightly bounded top-N list or a "too broad, add more
   specific terms" style message — never another 80K-character dump.
2. Existing short-but-meaningful terms still work exactly as before: `"amr"`
   (3 chars) must still substring-match `"kuka.amr"`'s tokenized `"amr"` key
   (existing test `search_substring_matches_short_terms`, index.rs). Do not
   regress this.
3. A query that legitimately matches many documents returns the top-scoring
   N with a clear, honest trailer stating how many more were omitted —
   never silently truncated, never silently unbounded.
4. All existing tests still pass; new tests cover both fixes.

## Design

### Fix 1 — minimum term length for substring matching (index.rs)

In `matching_keys` (index.rs ~line 139-145):

```rust
fn matching_keys(&self, term: &str) -> Vec<&str> {
    self.vocab
        .keys()
        .filter(|key| key.contains(term) || within_typo_tolerance(key, term))
        .map(String::as_str)
        .collect()
}
```

Add a length gate: terms shorter than a new constant require an **exact**
vocab-key match, not `.contains()`. `within_typo_tolerance` already refuses
to fuzzy-match below `FUZZY_MIN_TERM_LEN` (4 chars, search.rs), so only the
substring branch needs gating here.

Add to search.rs's "Tuning knobs" block (next to `FUZZY_MIN_TERM_LEN`):

```rust
/// Terms shorter than this must match a vocabulary key exactly — substring
/// containment on a 1-2 char term (especially digits) matches a huge
/// fraction of any real corpus (dates, page numbers, part numbers).
pub(crate) const MIN_SUBSTRING_TERM_LEN: usize = 3;
```

Then in `matching_keys`:

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

`3` is chosen so the existing `"amr"` test (3 chars) is untouched — verify
this against the full test suite before locking the constant in; if any
other existing test relies on substring behavior for a 1-2 char term, that's
a signal to reconsider the threshold, not to special-case around it.

### Fix 2 — cap hits actually formatted (main.rs)

In `run_search` (main.rs ~line 139-171), after `let hits = index.search(&terms);`,
hits are already sorted by score descending (index.rs `hits.sort_by_key(|hit|
std::cmp::Reverse(hit.score))`) — truncation is just a top-N slice.

Add a constant near the top of main.rs (or alongside the other tuning knobs
in search.rs, whichever reads more naturally in context — this one is a
presentation-layer concern, not a matching concern, so main.rs is the better
fit):

```rust
/// Hard ceiling on how many documents one search_docs call formats into its
/// response. Documents are already ranked; this caps worst-case output size
/// regardless of how broadly a query matches.
const MAX_HITS_SHOWN: usize = 20;
```

Update the hit-formatting branch of `run_search`:

```rust
} else {
    let total = hits.len();
    let shown = &hits[..total.min(MAX_HITS_SHOWN)];
    let ranked: Vec<String> = shown.iter().map(format_hit).collect();
    let mut text = format!(
        "Found {total} result(s) for '{query}'{}:\n\n{}",
        if total > shown.len() { format!(", showing top {}", shown.len()) } else { String::new() },
        ranked.join("\n\n")
    );
    if total > shown.len() {
        text.push_str(&format!(
            "\n\n…{} more result(s) omitted. Add more specific terms to narrow the query.",
            total - shown.len()
        ));
    }
    text
};
```

(Exact wording is a judgment call — keep it short and keep the retry
guidance in-band, consistent with the existing no-results message's
philosophy that tool OUTPUT is the one steering channel every harness
passes through.)

## Tests

Add to index.rs `mod tests`:

- `matching_keys_requires_exact_match_below_minimum_term_length` (or
  equivalent via a `run_search` helper): build a tiny bundle where a vocab
  key like `"2026"` exists but the bare token `"2"` does not; assert
  `search(&["2"])` returns no match against that document purely from
  substring containment (i.e., confirms `"2"` no longer matches `"2026"`).
- Regression: re-affirm `search_substring_matches_short_terms` (existing,
  `"amr"` case) still passes unchanged.

Add to main.rs `tool_tests`:

- `search_tool_caps_hit_count_with_trailer`: build a fixture bundle with
  more documents than `MAX_HITS_SHOWN` (e.g. 25 tiny docs all containing a
  common term), call `search_docs`, assert the formatted text contains
  exactly `MAX_HITS_SHOWN` bullet entries (`•`) and a trailer mentioning the
  omitted count.
- Regression: existing `search_tool_formats_hits` (single hit, well under
  the cap) must still pass with unchanged wording for the non-truncated
  case.

## Verification (exact commands, run in the devcontainer)

```bash
docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server cargo clippy --all-targets
docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server cargo test
docker exec -w /workspaces/MCP-Rust/mcp-server kuka-mcp-server cargo build
```

Live repro of the exact failure this step fixes (before/after comparison —
run once on master to confirm the overflow still reproduces, then again
after the fix on the branch): call `search_docs` with query `"2.2.9"`
against the real running server (same stdio/HTTP path used in prior steps'
live checks) and confirm the response is short and bounded, not another
80K-character dump.

## Documentation (same commit — house rule)

- **USER-MANUAL.md** §7 "How to read search results": add a bullet
  explaining (a) very short query terms (1-2 chars) now require an exact
  word match rather than matching any word containing that substring, and
  (b) results are capped at `MAX_HITS_SHOWN` with a trailer when more exist.
- **REFACTOR-PLAN.md**: row 12 + progress-log entries (start, implemented,
  complete — per the standing handoff protocol).
- **Lesson `lessons/refactor-17-bounding-search-output.html`** per the
  standing template (`../assets/style.css`, lesson-header block, WHY prose,
  before/after Rust from the actual commit, **equivalent Java samples**
  alongside — e.g. a Java search endpoint enforcing `LIMIT`/pagination, or
  the general principle of never trusting an unbounded `Stream.collect()`
  into a response body).
