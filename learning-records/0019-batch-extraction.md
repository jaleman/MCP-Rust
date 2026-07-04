# Batch Extraction CLI Tool (Lesson 20)

Lesson 20 rewrote `extract.rs` from a hardcoded single-file script into a flexible CLI tool that accepts a file or directory path as a command-line argument.

Key Rust patterns introduced:

- `std::env::args()` — returns an iterator over CLI arguments; index 0 is the binary path, index 1 is the first user argument (≈ Java `main(String[] args)` but includes the binary name)
- `.nth(1)` — gets the element at index 1 from an iterator without consuming previous elements
- `std::process::exit(1)` — terminates the process with a non-zero exit code (≈ Java `System.exit(1)`)
- `eprintln!()` — writes to stderr; error/diagnostic messages belong on stderr, normal output on stdout (≈ Java `System.err.println()`)
- `Path::is_file()` / `Path::is_dir()` — returns false if path doesn't exist; no separate exists check needed (≈ Java `Files.isRegularFile()` / `Files.isDirectory()`)
- Per-item error handling: `match process_pdf(...) { Ok(()) => ok += 1, Err(e) => { eprintln!(); failed += 1; } }` — logs and continues rather than aborting
- `fn process_pdf(path: &Path, out: &Path) -> Result<()>` — extracted helper returning Result so callers can choose how to handle failure
- `cargo run --bin extract -- <arg>` — the `--` separator tells Cargo to pass remaining arguments to the binary rather than parsing them as Cargo flags

**Implications**: CLI argument handling, `eprintln!`, `std::process::exit`, and per-item error recovery in batch loops are now known patterns. The `clap` crate teased as the next step for complex CLIs with flags/subcommands.
