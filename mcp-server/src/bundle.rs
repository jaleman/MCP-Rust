// The knowledge bundle: locating the directory, loading documents, listing them.
//
// The central type here is Document — one loaded markdown file with its
// frontmatter already parsed. Before this existed, three separate places
// (list_docs, search_docs, list_resources) each walked the directory, read
// files, and re-parsed frontmatter field by field. Now there is exactly one
// loader, and every consumer works with typed fields instead of raw strings.

use crate::frontmatter::extract_frontmatter_field;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// One loaded markdown document with its frontmatter parsed into fields.
/// In Java terms: the domain entity a repository returns, instead of handing
/// callers a raw file and making each of them parse it.
#[derive(Debug)]
pub struct Document {
    pub path: PathBuf,
    /// Filename without the .md extension — doubles as the resource URI id.
    pub stem: String,
    pub title: String,
    pub doc_type: String,
    pub resource: String,
    pub description: Option<String>,
    /// The full file text, frontmatter included.
    pub content: String,
    /// Byte offset where the body starts — just past the closing "---" line.
    pub body_start: usize,
    /// Diagram filenames under knowledge/images/ belonging to this document
    /// (from the optional `images:` frontmatter list). Empty when absent.
    pub images: Vec<String>,
}

impl Document {
    /// Loads and parses one markdown file. Errors (unreadable file) are
    /// returned to the caller rather than swallowed — load_bundle decides
    /// what to do with them.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;

        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Frontmatter fields, each with the same fallback the old call sites used
        let title = extract_frontmatter_field(&content, "title").unwrap_or_else(|| stem.clone());
        let doc_type = extract_frontmatter_field(&content, "type")
            .unwrap_or_else(|| "uncategorised".to_string());
        let resource = extract_frontmatter_field(&content, "resource").unwrap_or_default();
        let description = extract_frontmatter_field(&content, "description");

        // "images: [a.png, b.png]" → vec of bare filenames; absent → empty
        let images: Vec<String> = extract_frontmatter_field(&content, "images")
            .map(|list| {
                list.trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        // Find where the body starts — after the closing "---" of the frontmatter.
        // Computed once here so no consumer ever anchors excerpts inside frontmatter.
        let body_start = content.find("\n---\n").map(|p| p + 5).unwrap_or(0);

        Ok(Self {
            path: path.to_path_buf(),
            stem,
            title,
            doc_type,
            resource,
            description,
            content,
            body_start,
            images,
        })
    }

    /// The document text after the frontmatter block.
    pub fn body(&self) -> &str {
        &self.content[self.body_start..]
    }
}

// Case-insensitive .md check: Windows-originated files are often "FILE.MD",
// and the old case-sensitive comparison silently skipped them.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Loads every markdown document in the bundle directory.
///
/// A missing directory is an ERROR, not an empty bundle — the server is
/// usually launched by an MCP client from an arbitrary working directory,
/// so "dir not found" is the most likely failure mode and must not present
/// as "no documents found". Individual unreadable files are logged and
/// skipped so one bad file doesn't take down the whole bundle.
pub fn load_bundle(dir: &Path) -> Result<Vec<Document>> {
    let entries = std::fs::read_dir(dir).with_context(|| {
        format!(
            "knowledge directory not found: {} — set KUKA_KNOWLEDGE_DIR or run from the project root",
            dir.display()
        )
    })?;

    let mut docs: Vec<Document> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_markdown(&path) {
            continue;
        }
        match Document::load(&path) {
            Ok(doc) => docs.push(doc),
            Err(e) => tracing::warn!("skipping {}: {e:#}", path.display()),
        }
    }
    Ok(docs)
}

/// Guards resource URIs against path traversal: a stem like "../../secret"
/// would otherwise escape the knowledge directory when joined onto it.
/// Only plain file stems are valid — no separators, no parent references.
pub fn resource_stem_is_safe(stem: &str) -> bool {
    !stem.is_empty() && !stem.contains(['/', '\\']) && !stem.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::setup_test_bundle;
    use std::fs;

    #[test]
    fn document_load_parses_fields_and_body() {
        let temp_dir = setup_test_bundle();
        let doc = Document::load(&temp_dir.path().join("reflector-guide.md")).unwrap();

        assert_eq!(doc.stem, "reflector-guide");
        assert_eq!(doc.title, "Reflector Guide");
        assert_eq!(doc.doc_type, "technical-note");
        assert_eq!(doc.resource, "kuka-docs/test.pdf");
        assert_eq!(doc.description, Some("Test document for integration tests.".to_string()));
        // body() must start AFTER the frontmatter block
        assert!(doc.body().trim_start().starts_with("Reflectors must be mounted"));
    }

    #[test]
    fn load_bundle_errors_on_missing_directory() {
        // A missing directory must be an error, not an empty Vec — otherwise a
        // misconfigured KUKA_KNOWLEDGE_DIR looks like an empty knowledge bundle.
        let result = load_bundle(Path::new("no-such-directory-anywhere"));
        assert!(result.is_err(), "missing dir should be an error");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("no-such-directory-anywhere"), "error should name the path");
    }

    #[test]
    fn load_bundle_accepts_uppercase_extension() {
        // Windows-originated files often arrive as FILE.MD — the extension
        // check must be case-insensitive.
        let temp_dir = setup_test_bundle();
        let doc = "---\ntype: manual\ntitle: Uppercase Doc\n---\n\nBody.";
        fs::write(temp_dir.path().join("UPPERCASE.MD"), doc).unwrap();

        let docs = load_bundle(temp_dir.path()).unwrap();
        assert_eq!(docs.len(), 2, "both .md and .MD files should load");
        assert!(docs.iter().any(|d| d.title == "Uppercase Doc"));
    }

    #[test]
    fn resource_stem_guard_rejects_traversal() {
        // Safe: plain stems
        assert!(resource_stem_is_safe("reflector-guide"));
        assert!(resource_stem_is_safe("kmp-3000-manual-p012-018"));
        // Unsafe: anything that could escape the knowledge directory
        assert!(!resource_stem_is_safe("../secret"));
        assert!(!resource_stem_is_safe("..\\secret"));
        assert!(!resource_stem_is_safe("sub/dir"));
        assert!(!resource_stem_is_safe("dir\\file"));
        assert!(!resource_stem_is_safe(""));
    }
}
