// Library crate shared by the two binaries:
//   - mcp-server (src/main.rs)        — the MCP server
//   - extract    (src/bin/extract.rs) — the PDF → OKF markdown extractor
//
// In Java terms this is like the shared "core" module in a multi-module Maven
// build: common code lives here once and each application module depends on it.
// Cargo does it within a single crate — the `pub mod` lines below declare the
// library's modules, and the binaries import them as `mcp_server::...`
// (the crate name from Cargo.toml, with `-` becoming `_`).

pub mod bundle;
pub mod chunk;
pub mod frontmatter;
pub mod search;

// Test-only helper shared by the tests in bundle.rs and search.rs.
// #[cfg(test)] means this module is compiled ONLY during `cargo test` —
// it does not exist in the release binaries at all (unlike Java, where
// test fixtures live in a separate src/test tree to achieve the same thing).
#[cfg(test)]
pub(crate) mod test_util {
    use std::fs;

    // Creates a uniquely-named temp directory and writes one known OKF document into it.
    // Returns TempDir — not PathBuf — so the caller owns the directory and controls
    // when it is dropped. When TempDir goes out of scope, Drop deletes it automatically.
    pub(crate) fn setup_test_bundle() -> tempfile::TempDir {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let doc = "\
---
type: technical-note
title: Reflector Guide
description: Test document for integration tests.
resource: kuka-docs/test.pdf
tags: [test]
timestamp: 2026-01-01T00:00:00Z
---

Reflectors must be mounted at a height of 150 to 2000 mm above floor level.
The maximum spacing between reflectors is 8 metres.";

        fs::write(temp_dir.path().join("reflector-guide.md"), doc).unwrap();
        temp_dir
    }
}
