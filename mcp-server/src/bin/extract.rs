use anyhow::{bail, Context, Result};
use clap::Parser;
use mcp_server::frontmatter::OkfFrontmatter; // shared OKF format — same module the server parses with
use std::path::Path;

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

    if input.is_file() {
        // Single-file mode — exit non-zero on failure so callers can detect it
        if let Err(e) = process_pdf(input, knowledge_dir, args.force_pdftotext) {
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

            // Skip non-PDF files silently
            if path.extension().and_then(|e| e.to_str()) != Some("pdf") {
                continue;
            }

            // Match on the result so one bad PDF does not abort the batch
            match process_pdf(&path, knowledge_dir, args.force_pdftotext) {
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

// Extracts text from a single PDF and writes it as an OKF markdown file.
// Returns Ok(()) on success or an error the caller can log and continue from.
fn process_pdf(pdf_path: &Path, knowledge_dir: &Path, force_pdftotext: bool) -> Result<()> {
    println!("Extracting: {}", pdf_path.display());

    // Try pdf-extract first — unless the caller forced pdftotext
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

    // Build the OKF document through the shared frontmatter module — the same
    // code the server parses with, so the format cannot drift between binaries.
    let frontmatter = OkfFrontmatter {
        doc_type: "technical-note".to_string(),
        title: stem.to_string(),
        description: "Extracted from KUKA documentation.".to_string(),
        resource: format!("kuka-docs/{filename}"),
        tags: "[extracted, technical-note]".to_string(),
        timestamp: "2026-06-26T00:00:00Z".to_string(),
    };
    let okf = frontmatter.render(text.trim());

    let output = knowledge_dir.join(format!("{slug}.md"));
    std::fs::write(&output, &okf)?;
    println!("  → {}", output.display());

    Ok(())
}
