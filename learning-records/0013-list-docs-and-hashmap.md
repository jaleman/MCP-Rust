# List Documents Tool and HashMap (Lesson 14)

Lesson 14 added a second MCP tool `list_docs` that returns all documents in the knowledge bundle grouped by their OKF `type` field.

Key Rust patterns introduced:

- `use std::collections::{HashMap, HashSet};` — importing multiple items from the same module with curly-brace syntax
- `HashMap<String, Vec<String>>` — map from type to list of titles (≈ Java `Map<String, List<String>>`)
- `.entry(key).or_insert_with(Vec::new).push(value)` — the idiomatic group-by pattern (≈ Java `computeIfAbsent(k, x -> new ArrayList<>()).add(v)`)
- `grouped.keys().cloned().collect()` — extracts keys as owned Strings; `.cloned()` needed because `.keys()` yields `&String` references
- `grouped.remove(&key).unwrap_or_default()` — moves the Vec out of the map (avoids clone); preferred over `.get()` when ownership is needed for sorting/consuming
- `titles.sort()` — in-place alphabetical sort on `Vec<String>` (String implements Ord)

Key distinction explained: `or_insert_with(Vec::new)` vs `or_insert(Vec::new())` — the former lazily calls the closure only when needed; the latter always evaluates the argument even if the key exists. For `Vec::new()` the difference is negligible, but the pattern matters for expensive initialisations.

`getOrDefault()` Java gotcha noted: returns a copy of the default, not a reference stored in the map — mutations to it are lost.

**Implications**: HashMap and the entry API are now known patterns. The `cloned()` step when extracting keys has been explained. `remove()` vs `get()` distinction (owned vs borrowed) covered.
