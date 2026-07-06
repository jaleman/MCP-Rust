use anyhow::{bail, Context, Result};
use clap::Parser;
use mcp_server::chunk::{chunk_pages, clean_extracted_text, Chunk};
use mcp_server::frontmatter::OkfFrontmatter; // shared OKF format — same module the server parses with
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

// CLI argument definition — clap reads these field types and doc comments
// to generate --help output and argument validation automatically.
#[derive(Parser)]
#[command(
    name    = "extract",
    about   = "Extracts KUKA documents to OKF markdown files",
    version                      // reads version from Cargo.toml automatically
)]
struct Args {
    /// A single document file or a directory of documents
    input: String,

    /// The knowledge/ directory to write OKF files into
    output_dir: String,

    /// Skip pdf-extract and go straight to pdftotext
    #[arg(long)]
    force_pdftotext: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IngestKind {
    Pdf,
    Office,
    Text,
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

    // Every file written during this run. Two different source files whose names
    // differ only in stripped punctuation produce the same slug — the second
    // would silently overwrite the first without this check.
    let mut written: HashSet<PathBuf> = HashSet::new();

    if input.is_file() {
        // Single-file mode — exit non-zero on failure so callers can detect it
        if let Err(e) = process_document(input, knowledge_dir, args.force_pdftotext, &mut written) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    } else if input.is_dir() {
        // Batch mode — process every supported document, logging failures and continuing
        let entries = std::fs::read_dir(input).unwrap_or_else(|e| {
            eprintln!("Cannot read directory: {e}");
            std::process::exit(1);
        });

        let mut ok: usize = 0;
        let mut failed: usize = 0;

        for entry in entries.flatten() {
            let path = entry.path();

            // Skip unsupported files silently in batch mode.
            if ingest_kind(&path).is_none() {
                continue;
            }

            // Match on the result so one bad PDF does not abort the batch
            match process_document(&path, knowledge_dir, args.force_pdftotext, &mut written) {
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

fn ingest_kind(path: &Path) -> Option<IngestKind> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("pdf") {
        Some(IngestKind::Pdf)
    } else if ["docx", "doc", "pptx", "ppt"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some(IngestKind::Office)
    } else if extension.eq_ignore_ascii_case("txt") {
        Some(IngestKind::Text)
    } else {
        None
    }
}

// Calls `pdftotext <pdf_path> -` and returns the captured stdout as a String.
// The `-` argument tells pdftotext to write to stdout instead of a file.
fn try_pdftotext(pdf_path: &Path) -> Result<String> {
    let output = std::process::Command::new("pdftotext")
        .arg("-layout") // preserve columnar layout — tables stay readable
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

// Runs OCR on an image-based PDF via ocrmypdf, then extracts the text layer
// it produced. --skip-text leaves pages that already have text untouched,
// which is safe for mixed documents.
fn try_ocr(pdf_path: &Path) -> Result<String> {
    let temp = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile()
        .context("cannot create temp file for OCR output")?;

    let output = std::process::Command::new("ocrmypdf")
        .arg("--skip-text")
        .arg(pdf_path)
        .arg(temp.path())
        .output()
        .context("ocrmypdf not found — install ocrmypdf (apt-get install ocrmypdf)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ocrmypdf failed: {stderr}");
    }

    try_pdftotext(temp.path())
}

fn convert_office_to_pdf(input: &Path, temp_dir: &tempfile::TempDir) -> Result<PathBuf> {
    let output = std::process::Command::new("soffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(temp_dir.path())
        .arg(input)
        .output()
        .context("soffice not found — install libreoffice-writer libreoffice-impress")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("soffice failed: {stderr}");
    }

    let pdf = temp_dir.path().join(format!(
        "{}.pdf",
        input.file_stem().unwrap().to_string_lossy()
    ));
    if !pdf.exists() {
        bail!("soffice produced no output for {}", input.display());
    }

    Ok(pdf)
}

fn extraction_tags(used_ocr: bool) -> &'static str {
    if used_ocr {
        "[extracted, ocr, technical-note]"
    } else {
        "[extracted, technical-note]"
    }
}

fn include_ocr_source_title(stem: &str, text: &str) -> String {
    format!("{stem}\n\n{text}")
}

// Extracts text from a single PDF and writes it as one or more OKF markdown
// files. Documents that fit in one chunk (~8 KB) produce a single file exactly
// as before; larger documents produce one file per chunk with parent/pages
// frontmatter so search hits carry page-level provenance.
fn extract_pdf_text(extract_path: &Path, force_pdftotext: bool) -> Result<(String, bool)> {
    // Try pdf-extract first — unless the caller forced pdftotext.
    // Note: only pdftotext emits form-feed page separators, so page-accurate
    // chunking needs --force-pdftotext; pdf-extract output falls back to
    // paragraph-based splitting for oversized documents.
    let mut text = if force_pdftotext {
        try_pdftotext(extract_path)?
    } else {
        let extracted = pdf_extract::extract_text(extract_path)?;
        if extracted.trim().is_empty() {
            // pdf-extract returned nothing — fall back to pdftotext
            println!("  pdf-extract returned empty text — trying pdftotext…");
            try_pdftotext(extract_path)?
        } else {
            extracted
        }
    };

    let used_ocr = if text.trim().is_empty() {
        println!("  no text layer — running OCR…");
        text = try_ocr(extract_path)?;
        true
    } else {
        false
    };

    Ok((text, used_ocr))
}

fn process_document(
    source_path: &Path,
    knowledge_dir: &Path,
    force_pdftotext: bool,
    written: &mut HashSet<PathBuf>,
) -> Result<()> {
    println!("Extracting: {}", source_path.display());

    let kind = ingest_kind(source_path)
        .with_context(|| format!("unsupported input type: {}", source_path.display()))?;

    let (text, used_ocr) = match kind {
        IngestKind::Pdf => extract_pdf_text(source_path, force_pdftotext)?,
        IngestKind::Office => {
            let temp_dir =
                tempfile::tempdir().context("cannot create temp dir for Office conversion")?;
            let pdf = convert_office_to_pdf(source_path, &temp_dir)?;
            // LibreOffice gives us a page-shaped PDF; use pdftotext so form-feed
            // page separators survive into the existing chunking pipeline.
            extract_pdf_text(&pdf, true)?
        }
        IngestKind::Text => {
            let text = fs::read_to_string(source_path)
                .with_context(|| format!("cannot read text file {}", source_path.display()))?;
            (text, false)
        }
    };

    write_document(source_path, text, used_ocr, knowledge_dir, written)
}

fn write_document(
    source_path: &Path,
    mut text: String,
    used_ocr: bool,
    knowledge_dir: &Path,
    written: &mut HashSet<PathBuf>,
) -> Result<()> {
    // Build the output filename: lowercase with spaces replaced by dashes
    let stem = source_path.file_stem().unwrap().to_string_lossy();
    let filename = source_path.file_name().unwrap().to_string_lossy();
    let slug = stem
        .to_lowercase()
        .replace(' ', "-")
        .replace(['.', '(', ')'], ""); // strip punctuation that would break file paths

    if used_ocr {
        text = include_ocr_source_title(&stem, &text);
    }

    let resource = format!("kuka-docs/{filename}");
    // Real extraction time — files used to carry a hardcoded date
    let timestamp = jiff::Timestamp::now().to_string();
    let tags = extraction_tags(used_ocr);

    // Strip repeated headers/footers and TOC dot-leader lines BEFORE chunking
    // so the bundle files themselves are clean — excerpts, resources, and any
    // future consumer benefit, not just search anchoring.
    let text = clean_extracted_text(&text);

    let chunks = chunk_pages(text.trim());

    // Both extractors produced nothing — refuse to write a frontmatter-only
    // file (an empty body used to slip through and pollute the bundle).
    if chunks.is_empty() {
        if used_ocr {
            bail!("no text could be extracted, even after OCR");
        } else {
            bail!("no text could be extracted (image-only or empty PDF?)");
        }
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
            tags: tags.to_string(),
            timestamp,
        }
        .render(&chunks[0].text);
        write_output(&knowledge_dir.join(format!("{slug}.md")), &okf, written)?;
    } else {
        println!("  {} chunk(s):", chunks.len());
        for chunk in &chunks {
            let Chunk {
                text,
                first_page,
                last_page,
            } = chunk;
            let okf = OkfFrontmatter {
                doc_type: "technical-note".to_string(),
                title: format!("{stem} (pages {first_page}-{last_page})"),
                description: "Extracted from KUKA documentation.".to_string(),
                resource: resource.clone(),
                parent: Some(slug.clone()),
                pages: Some(format!("{first_page}-{last_page}")),
                tags: tags.to_string(),
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
// about to be clobbered (two source files collapsing to the same slug). Overwriting
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

#[cfg(test)]
mod tests {
    use super::{extraction_tags, ingest_kind, process_document, IngestKind};
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn extraction_tags_include_ocr_only_when_ocr_was_used() {
        assert_eq!(extraction_tags(false), "[extracted, technical-note]");
        assert_eq!(extraction_tags(true), "[extracted, ocr, technical-note]");
    }

    #[test]
    fn include_ocr_source_title_prepends_title_to_body_text() {
        assert_eq!(
            super::include_ocr_source_title("EmergencyFireAlarm", "Alarm body"),
            "EmergencyFireAlarm\n\nAlarm body"
        );
    }

    #[test]
    fn ingest_kind_routes_supported_extensions_case_insensitively() {
        assert_eq!(ingest_kind("manual.PDF".as_ref()), Some(IngestKind::Pdf));
        assert_eq!(ingest_kind("note.docx".as_ref()), Some(IngestKind::Office));
        assert_eq!(ingest_kind("legacy.DOC".as_ref()), Some(IngestKind::Office));
        assert_eq!(
            ingest_kind("slides.pptx".as_ref()),
            Some(IngestKind::Office)
        );
        assert_eq!(ingest_kind("deck.PPT".as_ref()), Some(IngestKind::Office));
        assert_eq!(
            ingest_kind("procedure.Txt".as_ref()),
            Some(IngestKind::Text)
        );
        assert_eq!(ingest_kind("image.png".as_ref()), None);
    }

    #[test]
    fn text_file_ingestion_writes_okf_with_original_resource() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("Fleet Procedure.txt");
        let output_dir = temp.path().join("knowledge");
        fs::create_dir(&output_dir).unwrap();
        fs::write(
            &input,
            "Fleet handoff procedure\n\nConfirm the mission queue before releasing the AMR.",
        )
        .unwrap();

        let mut written = HashSet::new();
        process_document(&input, &output_dir, false, &mut written).unwrap();

        let output = fs::read_to_string(output_dir.join("fleet-procedure.md")).unwrap();
        assert!(output.contains("title: Fleet Procedure"));
        assert!(output.contains("resource: kuka-docs/Fleet Procedure.txt"));
        assert!(output.contains("tags: [extracted, technical-note]"));
        assert!(output.contains("Confirm the mission queue before releasing the AMR."));
    }
}
