use anyhow::{bail, Context, Result};
use clap::Parser;
use mcp_server::chunk::{Chunk, chunk_pages};
use mcp_server::frontmatter::OkfFrontmatter; // shared OKF format — same module the server parses with
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// CLI argument definition — clap reads these field types and doc comments
// to generate --help output and argument validation automatically.
#[derive(Parser)]
#[command(
    name    = "extract",
    about   = "Extracts KUKA PDFs to OKF markdown files",
    version                      // reads version from Cargo.toml automatically
)]
struct Args {
    /// A single PDF file or a directory of PDFs
    input: String,

    /// The knowledge/ directory to write OKF files into
    output_dir: String,

    /// Skip pdf-extract and go straight to pdftotext
    #[arg(long)]
    force_pdftotext: bool,
}

fn main() {
    // Args::parse() reads std::env::args(), validates against the Args struct above,
    // and exits with a friendly --help-style message if anything is missing or malformed.
    let args = Args::parse();

    let input = Path::new(&args.input);
    let knowledge_dir = Path::new(&args.output_dir);

    // Verify the output directory exists before processing any files
    if !knowledge_dir.is_dir() {
        eprintln!(
            "Output directory does not exist: {}",
            knowledge_dir.display()
        );
        std::process::exit(1);
    }

    // Every file written during this run. Two different PDFs whose names
    // differ only in stripped punctuation produce the same slug — the second
    // would silently overwrite the first without this check.
    let mut written: HashSet<PathBuf> = HashSet::new();

    if input.is_file() {
        // Single-file mode — exit non-zero on failure so callers can detect it
        if let Err(e) = process_pdf(input, knowledge_dir, args.force_pdftotext, &mut written) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    } else if input.is_dir() {
        // Batch mode — process every PDF, logging failures and continuing
        let entries = std::fs::read_dir(input).unwrap_or_else(|e| {
            eprintln!("Cannot read directory: {e}");
            std::process::exit(1);
        });

        let mut ok: usize = 0;
        let mut failed: usize = 0;

        for entry in entries.flatten() {
            let path = entry.path();

            // Skip non-PDF files silently (case-insensitive: FILE.PDF counts)
            let is_pdf = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
            if !is_pdf {
                continue;
            }

            // Match on the result so one bad PDF does not abort the batch
            match process_pdf(&path, knowledge_dir, args.force_pdftotext, &mut written) {
                Ok(()) => ok += 1,
                Err(e) => {
                    eprintln!("Skipping {}: {e}", path.display());
                    failed += 1;
                }
            }
        }

        println!("\nDone: {ok} extracted, {failed} failed.");
    } else {
        eprintln!("Input path not found: {}", input.display());
        std::process::exit(1);
    }
}

// Calls `pdftotext <pdf_path> -` and returns the captured stdout as a String.
// The `-` argument tells pdftotext to write to stdout instead of a file.
fn try_pdftotext(pdf_path: &Path) -> Result<String> {
    let output = std::process::Command::new("pdftotext")
        .arg(pdf_path)
        .arg("-") // "-" means: write output to stdout
        .output()
        .context("pdftotext not found — install poppler-utils")?;

    // A non-zero exit code means pdftotext rejected the file
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("pdftotext failed: {stderr}");
    }

    // stdout is Vec<u8> — convert to String, propagating an error if not valid UTF-8
    String::from_utf8(output.stdout).context("pdftotext output is not valid UTF-8")
}

// Extracts text from a single PDF and writes it as one or more OKF markdown
// files. Documents that fit in one chunk (~8 KB) produce a single file exactly
// as before; larger documents produce one file per chunk with parent/pages
// frontmatter so search hits carry page-level provenance.
fn process_pdf(
    pdf_path: &Path,
    knowledge_dir: &Path,
    force_pdftotext: bool,
    written: &mut HashSet<PathBuf>,
) -> Result<()> {
    println!("Extracting: {}", pdf_path.display());

    // Try pdf-extract first — unless the caller forced pdftotext.
    // Note: only pdftotext emits form-feed page separators, so page-accurate
    // chunking needs --force-pdftotext; pdf-extract output falls back to
    // paragraph-based splitting for oversized documents.
    let text = if force_pdftotext {
        try_pdftotext(pdf_path)?
    } else {
        let extracted = pdf_extract::extract_text(pdf_path)?;
        if extracted.trim().is_empty() {
            // pdf-extract returned nothing — fall back to pdftotext
            println!("  pdf-extract returned empty text — trying pdftotext…");
            try_pdftotext(pdf_path)?
        } else {
            extracted
        }
    };

    // Build the output filename: lowercase with spaces replaced by dashes
    let stem = pdf_path.file_stem().unwrap().to_string_lossy();
    let filename = pdf_path.file_name().unwrap().to_string_lossy();
    let slug = stem
        .to_lowercase()
        .replace(' ', "-")
        .replace(['.', '(', ')'], ""); // strip punctuation that would break file paths

    let resource = format!("kuka-docs/{filename}");
    // Real extraction time — files used to carry a hardcoded date
    let timestamp = jiff::Timestamp::now().to_string();

    let chunks = chunk_pages(text.trim());

    // Both extractors produced nothing — refuse to write a frontmatter-only
    // file (an empty body used to slip through and pollute the bundle).
    if chunks.is_empty() {
        bail!("no text could be extracted (image-only or empty PDF?)");
    }

    if chunks.len() == 1 {
        // Fits in one chunk: single file, no parent/pages — exactly as before
        let okf = OkfFrontmatter {
            doc_type: "technical-note".to_string(),
            title: stem.to_string(),
            description: "Extracted from KUKA documentation.".to_string(),
            resource,
            parent: None,
            pages: None,
            tags: "[extracted, technical-note]".to_string(),
            timestamp,
        }
        .render(&chunks[0].text);
        write_output(&knowledge_dir.join(format!("{slug}.md")), &okf, written)?;
    } else {
        println!("  {} chunk(s):", chunks.len());
        for chunk in &chunks {
            let Chunk { text, first_page, last_page } = chunk;
            let okf = OkfFrontmatter {
                doc_type: "technical-note".to_string(),
                title: format!("{stem} (pages {first_page}-{last_page})"),
                description: "Extracted from KUKA documentation.".to_string(),
                resource: resource.clone(),
                parent: Some(slug.clone()),
                pages: Some(format!("{first_page}-{last_page}")),
                tags: "[extracted, technical-note]".to_string(),
                timestamp: timestamp.clone(),
            }
            .render(text);

            // Page ranges are usually unique per chunk; sub-split oversized
            // pages share a range, so disambiguate with a counter suffix.
            let base = format!("{slug}-p{first_page:03}-{last_page:03}");
            let mut output = knowledge_dir.join(format!("{base}.md"));
            let mut n = 1;
            while written.contains(&output) {
                n += 1;
                output = knowledge_dir.join(format!("{base}-{n}.md"));
            }
            write_output(&output, &okf, written)?;
        }
    }

    Ok(())
}

// Writes one OKF file, warning when a file written EARLIER IN THIS RUN is
// about to be clobbered (two PDFs collapsing to the same slug). Overwriting
// files from a previous run is normal re-extraction and stays silent.
fn write_output(path: &Path, content: &str, written: &mut HashSet<PathBuf>) -> Result<()> {
    if !written.insert(path.to_path_buf()) {
        eprintln!(
            "  WARNING: {} was already written by another PDF in this run — overwriting",
            path.display()
        );
    }
    std::fs::write(path, content)?;
    println!("  → {}", path.display());
    Ok(())
}
