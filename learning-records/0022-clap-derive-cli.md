---
name: clap-derive-cli
description: Replaced manual args() parsing in extract.rs with clap's derive API
metadata:
  type: project
  lesson: 22
---

Upgraded `extract.rs` to use the `clap` crate (v4, derive feature) for CLI argument parsing.

**What changed:**
- Added `clap = { version = "4", features = ["derive"] }` to `Cargo.toml`
- Defined an `Args` struct with `#[derive(Parser)]` and `#[command(name, about, version)]`
- Used `///` doc comments on fields as help text (no separate attribute needed)
- Replaced the manual `match (args.next(), args.next())` block with `Args::parse()`

**Key patterns introduced:**
- Feature flags in Cargo.toml: `features = ["derive"]` opts into optional crate sub-features
- `#[derive(Parser)]` — compile-time macro that generates parsing logic from struct fields and doc comments
- `Args::parse()` — reads `std::env::args()`, validates, exits with a friendly error if args are wrong
- Free gains: `--help` (auto-generated from doc comments), `--version` (from Cargo.toml), descriptive error messages

**Java analogy:** Like picocli — annotate a struct/class with `@Command` / `@Parameters`, call one parse method, get help and validation for free.

**Why:** Cleaner foundation for Lesson 23 (pdftotext fallback), where a `--force-pdftotext: bool` flag is added in two lines. With manual parsing that would require a third `args.next()` and another match arm.
