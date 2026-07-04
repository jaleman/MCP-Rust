# Multiple Excerpts Per Document (Lesson 15)

Lesson 15 upgraded `search_docs` to return up to 3 non-overlapping excerpts per matching document instead of just the first match position.

Key Rust patterns introduced:

- `str::match_indices(pattern)` — returns an iterator of `(usize, &str)` tuples for every occurrence of the pattern; unlike `find()` which returns only the first
- `.flat_map(|item| iterator)` — transforms each element into an iterator, then merges all iterators into one flat stream (≈ Java `Stream.flatMap()`)
- `Vec::dedup()` — removes consecutive duplicates; must be preceded by `.sort()` to guarantee all duplicates are adjacent
- Non-overlap window guard: `if pos < last_end { continue; }` — skips positions that fall inside an already-emitted excerpt window
- `400_usize` — numeric literal with type suffix, needed when the type cannot be inferred from context alone
- `excerpts.join("\n\n  ...")` — joins multiple passages with a visual separator

Algorithm: collect all exact match positions across all terms → sort → dedup → walk positions emitting windows, skipping any position inside a previous window → cap at 3 excerpts → fall back to document start if all matches were fuzzy-only.

**Implications**: `flat_map()`, `match_indices()`, and `dedup()` are now known patterns. The non-overlap window pattern is a reusable algorithm for any sliding-window problem.
