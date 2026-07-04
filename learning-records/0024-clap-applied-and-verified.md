---
name: clap-applied-and-verified
description: Lesson 22's clap migration was finished in extract.rs and verified building/running in the dev container
metadata:
  type: project
---

The Lesson 22 clap migration had been half-applied: `Args` struct with `#[derive(Parser)]` existed in `extract.rs` and `clap` was added to `Cargo.toml` (via `cargo add clap --features derive`), but `main()` still used the old manual `std::env::args().skip(1)` + `match` parsing — the `Args` struct was unused.

Completed the migration by replacing the manual parsing block with `Args::parse()`.

**Verified in the dev container:**
- `cargo build` — clean compile
- `./target/debug/extract` (no args) — friendly clap error: "the following required arguments were not provided: <INPUT> <OUTPUT_DIR>"
- `./target/debug/extract --help` — auto-generated usage from doc comments
- `./target/debug/extract --version` — prints `extract 0.1.0` from Cargo.toml

`extract.rs` is now ready for Lesson 23's `--force-pdftotext` flag, which adds one field to the same `Args` struct.

See [[clap-derive-cli]] and [[pdftotext-fallback]].
