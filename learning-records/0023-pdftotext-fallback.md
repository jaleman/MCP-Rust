---
name: pdftotext-fallback
description: Introduced std::process::Command to call external pdftotext as a fallback when pdf-extract returns empty text
metadata:
  type: project
  lesson: 23
---

Lesson 23 added a pdftotext fallback to `extract.rs`. The trigger was `EmergencyFireAlarm.pdf` (PowerPoint-converted) returning empty text from `pdf-extract`.

**What changed:**
- Added `use anyhow::{bail, Context, Result}` — introduced `bail!` macro and `.context()` method
- Added `--force-pdftotext: bool` field to the clap `Args` struct (one field + one `#[arg(long)]` attribute)
- Added `try_pdftotext(pdf_path: &Path) -> Result<String>` — calls `pdftotext <path> -` via `std::process::Command`
- Updated `process_pdf` to accept a `force_pdftotext: bool` parameter
- Fallback logic: if `pdf-extract` returns empty text, automatically call `try_pdftotext`; if flag is set, skip `pdf-extract` entirely

**Key patterns introduced:**
- `std::process::Command::new("cmd").arg(x).arg(y).output()?` — blocking external process call
- `output.status.success()` — exit code 0 check
- `String::from_utf8(output.stdout).context("...")?` — bytes → String with readable error
- `String::from_utf8_lossy(&output.stderr)` — lossy variant for error messages (can't fail)
- `bail!("message")` — anyhow macro that returns `Err` immediately
- `.context("message")` — attaches a human-readable layer over any error
- `let x = if cond { expr1 } else { expr2 };` — if-as-expression (idiomatic Rust, no mutable intermediary)

**Java analogies established:**
- `Command::new("pdftotext").arg(path).arg("-").output()` = `new ProcessBuilder("pdftotext", path, "-").start().waitFor()`
- `bail!("msg")` = `throw new RuntimeException("msg")`
- `.context("msg")` = `catch (Exception e) { throw new RuntimeException("msg", e); }`

**Installation note:** pdftotext is part of `poppler-utils` (`sudo apt-get install -y poppler-utils`). The `-` argument writes to stdout — without it, pdftotext writes a `.txt` file next to the PDF.

**Why:** `EmergencyFireAlarm.pdf` (and likely other PowerPoint-converted PDFs) had no text layer. `pdf-extract` returned empty strings. `pdftotext` from poppler handles these cases better.
