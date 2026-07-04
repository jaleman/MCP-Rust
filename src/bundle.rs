// Locating and listing the knowledge bundle directory.
// (This module grows into the Document type + load_bundle in refactor step 2.)

use crate::frontmatter::extract_frontmatter_field;
use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Returns the knowledge bundle directory path.
// Reads KUKA_KNOWLEDGE_DIR from the environment if set; falls back to "knowledge".
// Using an env var lets tests and production deployments override the path
// without recompiling.
pub fn knowledge_dir() -> PathBuf {
    PathBuf::from(std::env::var("KUKA_KNOWLEDGE_DIR").unwrap_or_else(|_| "knowledge".to_string()))
}

pub fn list_docs_in(dir: &Path) -> Result<CallToolResult, McpError> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    let mut total: usize = 0;

    // Walk every file in the knowledge directory
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip anything that isn't a markdown file
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&path) {
                // Pull the title from frontmatter; fall back to the filename if absent
                let title = extract_frontmatter_field(&content, "title")
                    .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().to_string());

                // Pull the type from frontmatter; fall back to "uncategorised" if absent
                let doc_type = extract_frontmatter_field(&content, "type")
                    .unwrap_or_else(|| "uncategorised".to_string());

                grouped.entry(doc_type).or_insert_with(Vec::new).push(title);
                total += 1;
            }
        }
    }

    // Nothing in the bundle at all
    if grouped.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "No documents found in the knowledge bundle.".to_string(),
        )]));
    }

    // Sort the type keys alphabetically so the output is stable across runs
    let mut type_keys: Vec<String> = grouped.keys().cloned().collect();
    type_keys.sort();

    // Build one text section per type, with titles sorted within each section
    let sections: Vec<String> = type_keys
        .into_iter()
        .map(|doc_type| {
            let mut titles = grouped.remove(&doc_type).unwrap_or_default();
            titles.sort();
            let items: Vec<String> = titles.into_iter().map(|t| format!("  • {t}")).collect();
            format!("{doc_type}:\n{}", items.join("\n"))
        })
        .collect();

    Ok(CallToolResult::success(vec![Content::text(format!(
        "Knowledge bundle — {total} document(s):\n\n{}",
        sections.join("\n\n")
    ))]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::setup_test_bundle;

    #[test]
    fn list_docs_shows_document_grouped_by_type() {
        let temp_dir = setup_test_bundle();
        let result = list_docs_in(temp_dir.path()).unwrap();
        let output = format!("{:?}", result);
        assert!(output.contains("Reflector Guide"), "Should list the document title");
        assert!(output.contains("technical-note"), "Should group by type from frontmatter");
    }
}
