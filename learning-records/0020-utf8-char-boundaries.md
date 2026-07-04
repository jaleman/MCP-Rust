# UTF-8 Char Boundaries (Lesson 21)

Lesson 21 fixed a silent server crash caused by slicing a Rust `&str` at a byte position that was not a valid UTF-8 char boundary.

**Root cause:** the MQTT Payload Definitions document contains smart-quote characters (`"` / `"`, U+201C/U+201D), which are 3-byte UTF-8 sequences. The proximity scoring loop computed window boundaries as `pos ± 500` — raw arithmetic blind to character structure. For two specific match positions (10365 and 15574), those arithmetic results landed on continuation bytes (`0x80`–`0xBF`). Slicing `&str` at a continuation byte is an instant panic in Rust; the server process crashed silently and the MCP tool call never returned.

Key Rust patterns introduced:

- `str::is_char_boundary(i)` — returns `true` if byte position `i` is a valid slice boundary (i.e., not a UTF-8 continuation byte). The only way to check a position before slicing.
- `floor_char_boundary(s, pos)` — walks backwards from `pos` to find the nearest boundary ≤ pos; used for **slice starts**
- `ceil_char_boundary(s, pos)` — walks forwards from `pos` to find the nearest boundary ≥ pos; used for **slice ends**
- Both are effectively O(1): UTF-8 sequences are at most 4 bytes, so at most 3 steps

**Where the fix was applied:**
1. Proximity scoring window: `&lower[win_start..win_end]`
2. Excerpt building: `content[start..end]`

**Rule:** any time arithmetic produces a slice boundary (`pos - N`, `pos + N`, `.min(len)`), snap it through `floor_char_boundary` (start) or `ceil_char_boundary` (end) before slicing. Never assume arithmetic lands on a boundary.

**Why only multi-term queries triggered it:** single-term queries also hit the MQTT doc, but the specific positions that produce bad window boundaries (10365, 15574) only appear as matches for "payload" and "mission" respectively, and the proximity scoring loop only runs when a doc matches ALL terms.

**Java analogy:** `String.substring()` counts Java `char` units (UTF-16), so it's always on a valid boundary and never panics. Rust slicing is raw bytes — the programmer is responsible for alignment.
