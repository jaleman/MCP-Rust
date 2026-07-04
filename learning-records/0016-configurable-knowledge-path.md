# Configurable Knowledge Path (Lesson 17)

Lesson 17 replaced the hardcoded `"knowledge"` path in `list_docs` and `search_docs` with a helper function that reads `KUKA_KNOWLEDGE_DIR` from the environment, falling back to `"knowledge"` if unset.

Key Rust patterns introduced:

- `std::env::var("KEY")` — reads an env var; returns `Result<String, VarError>` (≈ Java `System.getenv()` but forces explicit handling of absence via Result, not null)
- `VarError::NotPresent` — the error variant returned when the variable is not set
- `PathBuf` — owned, heap-allocated path (≈ `String`); contrasted with `Path` which is a borrowed view (≈ `&str`)
- `std::path::PathBuf::from(string)` — constructs a PathBuf from a String
- `.unwrap_or_else(|_| fallback)` — returns the Ok value or calls the closure on Err; `|_|` discards the error value (≈ Java null check with default)
- Variable shadowing: local variable `knowledge_dir` shadows function `knowledge_dir()` within its scope — valid in Rust, does not cause ambiguity outside that scope

Motivation: configurable path unblocks integration tests (tests can set KUKA_KNOWLEDGE_DIR to a temp directory) and fixes the production CWD problem (server can be launched from any directory).

**Implications**: `std::env::var()`, `PathBuf`, and `.unwrap_or_else()` are now known patterns. The distinction between `Path` (borrowed) and `PathBuf` (owned) mirrors `&str` vs `String`.
