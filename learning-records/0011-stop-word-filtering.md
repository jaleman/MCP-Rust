# Stop Word Filtering (Lesson 12)

Lesson 12 added stop word filtering to `search_docs`, stripping common words ("what", "are", "the", "for", etc.) from the query before matching so only meaningful terms reach `fuzzy_word_match`.

Key Rust patterns introduced:

- `use std::collections::HashSet;` — importing from the standard library's collections module
- `HashSet::from([...])` — creating a HashSet from an array literal (≈ Java's `Set.of(...)`)
- `HashSet<&str>` — type annotation needed because Rust cannot infer element type from the array alone
- `.filter(|word| !stop_words.contains(word))` — iterator filter (≈ Java `Stream.filter()`); keeps elements where predicate returns true
- `if terms.is_empty() { return Ok(...); }` — vacuous truth guard: `all()` over zero elements returns true, which would match every document

Also reinforced: explicit `return` keyword needed for early returns that are not the last expression in the function.

Vacuous truth concept explained: `all()` over an empty iterator is true by definition — the empty guard exists to catch this case.

**Implications**: `HashSet`, `.filter()`, and the vacuous truth pattern are now known. The stop word list is hardcoded; a future improvement would be to load it from a config file.
