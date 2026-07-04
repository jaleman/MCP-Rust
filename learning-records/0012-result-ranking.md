# Result Ranking (Lesson 13)

Lesson 13 added relevance-based ranking to `search_docs`. Results are now sorted by total term occurrence count (highest first) before being returned.

Key Rust patterns introduced:

- `(usize, String)` — tuple type; anonymous compound value; fields accessed by `.0`, `.1`
- Tuple destructuring `|(_, text)|` in closure parameters — `_` discards a field without an unused-variable warning
- `str::matches(pattern).count()` — counts non-overlapping substring occurrences
- `.map(|term| expr).sum()` — transforms each element then sums the results (≈ Java `mapToInt(...).sum()`)
- `Vec::sort_by(|a, b| b.0.cmp(&a.0))` — in-place sort with a comparator closure; reversed arguments produce descending order
- `Ordering` — the return type of `.cmp()`: `Less`, `Equal`, `Greater` (≈ Java comparator returning negative/zero/positive)
- `into_iter()` vs `iter()` — `into_iter()` consumes the Vec and yields owned values, needed when moving a String out of a tuple

Also noted: `sort_by_key()` with `std::cmp::Reverse` as a cleaner alternative, deferred to the primary source.

Fuzzy matches score zero for the fuzzy-matched term (only exact occurrences counted). Accepted as a reasonable default.

**Implications**: Tuples, `.map()`, `.sum()`, and `sort_by()` are now known patterns. `into_iter()` vs `iter()` distinction explained for the first time in context.
