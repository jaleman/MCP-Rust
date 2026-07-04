// Reading and writing the OKF frontmatter block.
//
// This module is the single definition of the OKF format. The extract binary
// WRITES frontmatter via OkfFrontmatter::render, and the server PARSES it via
// extract_frontmatter_field. Before this module existed, the writer lived in
// extract.rs and the parser in main.rs as two unrelated string formats —
// a change to one could silently break the other. Now they share one file
// and one round-trip test.

/// The fields of an OKF frontmatter block, in the order they are written.
/// In Java terms: the shared DTO that both the serializer and the deserializer
/// depend on, instead of two hand-rolled formats in different classes.
pub struct OkfFrontmatter {
    pub doc_type: String,
    pub title: String,
    pub description: String,
    pub resource: String,
    /// Rendered verbatim, e.g. "[extracted, technical-note]"
    pub tags: String,
    pub timestamp: String,
}

impl OkfFrontmatter {
    /// Renders a complete OKF document: frontmatter block followed by the body.
    pub fn render(&self, body: &str) -> String {
        format!(
            "---\n\
             type: {}\n\
             title: {}\n\
             description: {}\n\
             resource: {}\n\
             tags: {}\n\
             timestamp: {}\n\
             ---\n\n\
             {}",
            self.doc_type,
            self.title,
            self.description,
            self.resource,
            self.tags,
            self.timestamp,
            body,
        )
    }
}

// Parses a single named field out of YAML-style frontmatter at the top of a
// markdown file. Frontmatter is the block between the opening --- and closing ---
// lines. Returns None if the file has no frontmatter or the field is not present.
pub fn extract_frontmatter_field(content: &str, field: &str) -> Option<String> {
    // strip_prefix removes the opening "---\n"; split_once splits on the closing
    // "\n---" and takes the left side (.0); together these isolate the frontmatter block
    let inner = content.strip_prefix("---\n")?.split_once("\n---")?.0;
    for line in inner.lines() {
        // Each frontmatter line is "key: value" — strip_prefix checks for "field: "
        // and returns whatever follows if it matches
        if let Some(value) = line.strip_prefix(&format!("{field}: ")) {
            return Some(value.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_field_present() {
        // A valid frontmatter block with the requested field should return Some(value)
        let content = "---\ntitle: Reflector Guide\ntype: technical-note\n---\n\nBody text.";
        assert_eq!(
            extract_frontmatter_field(content, "title"),
            Some("Reflector Guide".to_string())
        );
    }

    #[test]
    fn frontmatter_field_absent() {
        // A frontmatter block that does not contain the field should return None
        let content = "---\ntype: technical-note\n---\n\nBody text.";
        assert_eq!(extract_frontmatter_field(content, "title"), None);
    }

    #[test]
    fn frontmatter_no_frontmatter() {
        // A file with no frontmatter at all should return None
        let content = "Just plain text with no frontmatter.";
        assert_eq!(extract_frontmatter_field(content, "title"), None);
    }

    #[test]
    fn render_and_parse_round_trip() {
        // What the writer produces, the parser must read back unchanged.
        // This is the test that keeps the two binaries agreeing on the format.
        let fm = OkfFrontmatter {
            doc_type: "technical-note".to_string(),
            title: "Reflector Guide".to_string(),
            description: "Extracted from KUKA documentation.".to_string(),
            resource: "kuka-docs/test.pdf".to_string(),
            tags: "[extracted, technical-note]".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        let doc = fm.render("Body text.");

        assert_eq!(
            extract_frontmatter_field(&doc, "title"),
            Some("Reflector Guide".to_string())
        );
        assert_eq!(
            extract_frontmatter_field(&doc, "type"),
            Some("technical-note".to_string())
        );
        assert_eq!(
            extract_frontmatter_field(&doc, "resource"),
            Some("kuka-docs/test.pdf".to_string())
        );
        assert!(doc.ends_with("---\n\nBody text."));
    }
}
