# Drop and TempDir (Lesson 19)

Lesson 19 replaced the manual `setup_test_bundle()` / `teardown()` pair in the integration tests with `tempfile::TempDir`, which implements `Drop` for automatic cleanup — even during a panic.

Key Rust patterns introduced:

- `Drop` trait — `fn drop(&mut self)` is called automatically when a value goes out of scope; no special syntax needed (≈ Java `AutoCloseable` + try-with-resources, but automatic and universal)
- Panic unwinding calls `drop()` on every value in scope — guaranteeing cleanup even when a test fails
- `tempfile::TempDir` — creates a unique temp directory; implements `Drop` to delete it on scope exit
- `TempDir::new().unwrap()` — constructs the temp directory
- `TempDir::path()` — returns `&Path` to the directory
- `cargo add tempfile` — added to `[dependencies]` (could be `[dev-dependencies]` for test-only crates)
- Returning an owned `Drop` type from a helper transfers ownership to the caller, so cleanup happens at the caller's scope end — not the helper's

Key distinction from Java: Java's `try-with-resources` is opt-in; Rust's `Drop` is automatic. Java's `finalize()` is closer in mechanism but unreliable (GC-dependent); `Drop` is deterministic.

**Implications**: The `Drop` trait and RAII pattern are now understood. `tempfile::TempDir` is the standard tool for temp directories in tests. `std::mem::drop(value)` mentioned as the way to drop a value early.
