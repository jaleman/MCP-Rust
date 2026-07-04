# Integration Tests (Lesson 18)

Lesson 18 added 3 integration tests for `search_docs` and `list_docs`, using a temp directory populated with a known OKF file and `KUKA_KNOWLEDGE_DIR` to redirect the server.

Key Rust patterns introduced:

- `std::env::temp_dir()` — returns the system temp directory as a `PathBuf` (≈ Java `System.getProperty("java.io.tmpdir")`)
- `PathBuf::join(component)` — appends a path component, returning a new `PathBuf`
- `std::fs::create_dir_all(&path)` — creates directory tree including missing parents; safe if dir exists (≈ `Files.createDirectories()` / `mkdir -p`)
- `std::fs::write(&path, content)` — writes bytes or string to a file (≈ `Files.writeString()`)
- `std::fs::remove_dir_all(&path)` — recursively deletes directory (≈ Apache Commons `FileUtils.deleteDirectory()` / `rm -rf`)
- `std::env::set_var("KEY", value)` — sets env var globally in the process (≈ `System.setProperty()`); dangerous in parallel tests
- `cargo test filter -- --test-threads=1` — runs only matching tests, single-threaded to prevent env var interference
- `format!("{:?}", value)` — uses `Debug` trait to produce a string of all field values; used here because `CallToolResult` implements `Debug` but not `Display`

Key design issue: no `finally` equivalent in Rust means `teardown()` is skipped on test panic, leaving the temp dir on disk. The `tempfile` crate's `TempDir` (implements `Drop`) solves this — deferred to Lesson 19.

Two separate test modules in one file: `mod tests` for unit tests, `mod integration_tests` for filesystem tests. Both use `#[cfg(test)]`.

**Implications**: `std::fs` helpers are now known. The env-var global-state problem and single-thread workaround have been explained. `Drop` trait teased as the correct solution, covered next.
