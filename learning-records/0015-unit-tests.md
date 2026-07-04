# Unit Tests (Lesson 16)

Lesson 16 added 7 unit tests for the two pure functions in main.rs: `fuzzy_word_match` and `extract_frontmatter_field`.

Key Rust patterns introduced:

- `#[cfg(test)]` — conditional compilation attribute; the entire `mod tests` block is excluded from normal builds and only compiled when running `cargo test` (≈ Java's test source directory, but inline)
- `mod tests { ... }` — a child module for test code, conventionally named `tests`
- `use super::*;` — imports everything from the parent module into the test module; `super` means "one level up" (≈ being in the same Java package)
- `#[test]` — marks a function as a test case (≈ JUnit `@Test`)
- `assert!(condition)` — panics if false (≈ JUnit `assertTrue`)
- `assert_eq!(expected, actual)` — panics if unequal and prints both values (≈ JUnit `assertEquals`)
- `cargo test` — runs all tests; `cargo test <filter>` runs only tests whose names contain the filter string (≈ `mvn test` / `gradle test`)

Key distinction: pure functions (no I/O, no side effects) are unit-testable trivially. The MCP tool methods (`search_docs`, `list_docs`) require filesystem access and are candidates for integration tests in a future lesson.

**Implications**: Basic Rust test structure is now known. The pure-function vs I/O distinction for testability has been established. Integration tests deferred to a future lesson.
