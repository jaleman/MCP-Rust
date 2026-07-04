# Fuzzy and Multi-Term Search (Lesson 11)

Lesson 11 upgraded `search_docs` to handle misspelled queries and multi-word queries.

Two changes shipped together:
- **Multi-term**: query is split into words with `.split_whitespace()`, and the document must match ALL words via `.all()`
- **Fuzzy**: each unmatched word is checked against every word in the document using `strsim::levenshtein()` with an adaptive threshold (1 edit for 4–7 char terms, 2 for longer)

Key Rust patterns introduced:

- `strsim = "0.11"` added to `Cargo.toml` — the pattern for adding a crate dependency
- `use strsim::levenshtein;` — bringing a crate function into scope
- `if expr { return val; }` — early return syntax in Rust (same as Java)
- `let threshold = if a <= b { 1 } else { 2 };` — `if/else` as an expression (≈ Java ternary)
- `.split_whitespace()` — iterator over whitespace-separated words (≈ Java `split("\\s+")`)
- `.all(|item| condition)` — all must pass (≈ Java `Stream.allMatch()`)
- `.any(|item| condition)` — any must pass (≈ Java `Stream.anyMatch()`)
- `find_map(|term| lower.find(*term))` — the `*term` dereferences `&&str` to `&str` for `find()` to accept it as a Pattern

Design decision: short terms (≤ 3 chars) skip fuzzy matching entirely to avoid false positives. The excerpt position anchor falls back to doc start (pos=0) when all matches were fuzzy-only.

**Implications**: `.all()` and `.any()` are now known patterns. The `&&str` → `*term` deref pattern was explicitly explained. The Cargo dependency workflow has been demonstrated once.
