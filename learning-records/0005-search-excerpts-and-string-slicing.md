# Search Excerpts and String Slicing (Lesson 10)

Lesson 10 upgraded `search_docs` to return document excerpts — a 600-character window of text around each match — instead of only the document title and source citation. This makes the server genuinely answer questions rather than just locate documents.

Key Rust patterns introduced:

- `str::find(&pattern)` returns `Option<usize>` — the byte offset of the first match. Replaces the previous `contains()` check while also giving the position needed for windowing.
- `usize::saturating_sub(n)` — underflow-safe subtraction that clamps to 0. Used for the lookback window: `pos.saturating_sub(200)`.
- `.min(n)` on usize — caps a value at a maximum. Used for the lookahead cap: `(pos + query.len() + 400).min(content.len())`.
- String slicing `&content[start..end]` — extracts a substring by byte range. Requires both bounds to fall on UTF-8 character boundaries (safe for ASCII-only KUKA docs).

Also explained: why byte-position slicing is safe for ASCII content and where `floor_char_boundary()` would be needed for multi-language text (deferred).

**Implications**: Can reference `find()` / `saturating_sub` / `.min()` without re-explaining. String slicing by byte range is now a known pattern. The UTF-8 boundary caveat has been raised once — mention it only if they encounter a panic on non-ASCII input.
