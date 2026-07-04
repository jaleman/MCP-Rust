use anyhow::Result;
use rmcp::{
    // MCP SDK types: error type, server trait, service wiring, tool routing machinery,
    // parameter wrapper, all protocol model types, JSON schema + serde derives,
    // the #[tool] / #[tool_router] / #[tool_handler] macros, and stdio transport
    ErrorData as McpError,
    RoleServer,
    ServerHandler,
    ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars,
    serde,
    service::RequestContext,
    tool,
    tool_handler,
    tool_router,
    transport::stdio,
};
use std::collections::{HashMap, HashSet};
use strsim::levenshtein; // edit-distance function from the strsim crate

// Snap DOWN to the nearest char boundary ≤ pos.
// Used for slice starts — avoids panicking when arithmetic lands mid-character.
fn floor_char_boundary(s: &str, pos: usize) -> usize {
    let pos = pos.min(s.len());
    (0..=pos)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0)
}

// Snap UP to the nearest char boundary ≥ pos.
// Used for slice ends — avoids panicking when arithmetic lands mid-character.
fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    (pos..=s.len())
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(s.len())
}

// Returns true if the document contains the query term — exactly or within a
// typo tolerance that scales with word length.
fn fuzzy_word_match(doc_lower: &str, term: &str) -> bool {
    // Fast path: exact substring match anywhere in the document
    if doc_lower.contains(term) {
        return true;
    }
    // Short terms produce too many false positives when fuzzy-matched
    if term.len() <= 3 {
        return false;
    }
    // Allow 1 typo for medium words (4–7 chars), 2 typos for longer words
    let threshold = if term.len() <= 7 { 1 } else { 2 };
    let term_len = term.len();
    // Split the document into individual words and check if any word is
    // within the edit-distance threshold of the query term.
    // Pre-filter by length: if two strings differ in length by more than the
    // threshold they cannot possibly be within edit distance, so skip the
    // expensive levenshtein() call entirely. This prevents hangs on documents
    // containing long JSON values, base64 strings, or URLs.
    doc_lower
        .split_whitespace()
        .filter(|word| {
            let wlen = word.len();
            wlen <= term_len + threshold && term_len <= wlen + threshold
        })
        .any(|word| levenshtein(word, term) <= threshold)
}

// Trims, collapses internal whitespace, and lowercases a line so that
// "  KUKA Robotics GmbH  " and "kuka robotics gmbh" count as the same line.
fn normalize_line(s: &str) -> String {
    s.trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// Returns a HashSet of normalised lines that appear 3 or more times in text.
// These are boilerplate candidates: running headers, footers, page titles.
fn repeated_lines(text: &str) -> HashSet<String> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let norm = normalize_line(line);
        if !norm.is_empty() {
            *freq.entry(norm).or_insert(0) += 1;
        }
    }
    freq.into_iter()
        .filter(|(_, count)| *count >= 3)
        .map(|(line, _)| line)
        .collect()
}

// Returns the trimmed line of text that contains the byte at position pos.
fn line_at_pos(text: &str, pos: usize) -> &str {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[pos..].find('\n').map(|i| pos + i).unwrap_or(text.len());
    text[line_start..line_end].trim()
}

// The input schema for the search_docs tool.
// #[derive] generates the Debug, Deserialize, and JsonSchema implementations
// automatically — equivalent to Lombok @Data + Jackson annotations in Java.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchInput {
    /// Search term to look up in the KUKA documentation
    query: String,
}

// Parses a single named field out of YAML-style frontmatter at the top of a
// markdown file. Frontmatter is the block between the opening --- and closing ---
// lines. Returns None if the file has no frontmatter or the field is not present.
fn extract_frontmatter_field(content: &str, field: &str) -> Option<String> {
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

// Returns the knowledge bundle directory path.
// Reads KUKA_KNOWLEDGE_DIR from the environment if set; falls back to "knowledge".
// Using an env var lets tests and production deployments override the path
// without recompiling.
fn knowledge_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("KUKA_KNOWLEDGE_DIR").unwrap_or_else(|_| "knowledge".to_string()),
    )
}

fn list_docs_in(dir: &std::path::Path) -> Result<CallToolResult, McpError> {
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

fn search_docs_in(dir: &std::path::Path, query: String) -> Result<CallToolResult, McpError> {
    let mut results: Vec<(usize, String)> = Vec::new();

    // Lowercase the query for matching; keep the original for display messages
    let query_lower = query.to_lowercase();

    // Common words that appear in almost every document and carry no search value.
    // Using a HashSet gives O(1) lookup per word instead of scanning a list.
    let stop_words: HashSet<&str> = HashSet::from([
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "have", "has", "had", "do",
        "does", "did", "will", "would", "can", "could", "should", "may", "might", "shall", "i",
        "you", "he", "she", "it", "we", "they", "what", "which", "who", "when", "where", "why",
        "how", "in", "on", "at", "to", "for", "of", "with", "by", "from", "and", "or", "but",
        "not", "no", "nor",
    ]);

    // Split the query into individual words, dropping stop words.
    // Every remaining term must match somewhere in a document for it to be included.
    let terms: Vec<&str> = query_lower
        .split_whitespace()
        .filter(|word| !stop_words.contains(word))
        .collect();

    // Guard: all() over an empty list returns true vacuously, which would match
    // every document. Return early with a helpful message instead.
    if terms.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "Query contains only common words. Please add specific search terms.".to_string(),
        )]));
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip non-markdown files (e.g. the source PDFs themselves)
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&path) {
                // Lowercase once and reuse for all term checks against this document
                let lower = content.to_lowercase();

                // Find where the body starts — after the closing "---" of the frontmatter.
                // This prevents excerpts from anchoring at position 0 and showing
                // the frontmatter block instead of the actual document content.
                let body_start = content.find("\n---\n").map(|p| p + 5).unwrap_or(0);

                // A document qualifies only if every query term matches (exactly or fuzzily)
                if terms.iter().all(|term| fuzzy_word_match(&lower, term)) {
                    let title =
                        extract_frontmatter_field(&content, "title").unwrap_or_else(|| {
                            path.file_stem().unwrap().to_string_lossy().to_string()
                        });

                    let resource =
                        extract_frontmatter_field(&content, "resource").unwrap_or_default();

                    // Collect byte positions of every exact match across all query terms,
                    // searching only within the body (after frontmatter) so frontmatter
                    // field values don't anchor the excerpt window
                    let mut positions: Vec<usize> = terms
                        .iter()
                        .flat_map(|term| {
                            lower[body_start..]
                                .match_indices(*term)
                                .map(|(pos, _)| pos + body_start)
                        })
                        .collect();
                    positions.sort();
                    positions.dedup();

                    // Build the boilerplate set from the body and remove any position
                    // that lands on a line appearing 3+ times (headers, footers, page titles).
                    let exact_positions_before_filter = positions.len();
                    let boilerplate = repeated_lines(&lower[body_start..]);
                    let positions: Vec<usize> = positions
                        .into_iter()
                        .filter(|&pos| {
                            !boilerplate.contains(&normalize_line(line_at_pos(&lower, pos)))
                        })
                        .collect();

                    // Skip only when exact matches existed but ALL landed on boilerplate lines.
                    // If exact_positions_before_filter is 0, this was a fuzzy-only match —
                    // let it fall through to the body-start excerpt fallback below.
                    if exact_positions_before_filter > 0 && positions.is_empty() {
                        continue;
                    }

                    // Proximity scoring: rank each position by how many distinct query terms
                    // appear within a ±500 char window around it. Positions where multiple
                    // terms co-occur (e.g. "mission" + "command" + "payload" together) score
                    // higher than isolated hits in page headers or table-of-contents lines.
                    let window_size = 500_usize;
                    let mut scored: Vec<(usize, usize)> = positions
                        .iter()
                        .map(|&pos| {
                            // floor/ceil snap to nearest valid UTF-8 char boundary before slicing
                            let win_start = floor_char_boundary(
                                &lower,
                                pos.saturating_sub(window_size).max(body_start),
                            );
                            let win_end = ceil_char_boundary(
                                &lower,
                                (pos + window_size).min(lower.len()),
                            );
                            let window = &lower[win_start..win_end];
                            let co_occurrence =
                                terms.iter().filter(|term| window.contains(*term)).count();
                            (co_occurrence, pos)
                        })
                        .collect();

                    // Sort highest co-occurrence first; break ties by position (earlier wins)
                    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

                    // Build up to 3 non-overlapping excerpt windows from highest-scoring positions
                    let mut excerpts: Vec<&str> = Vec::new();
                    let mut covered: Vec<(usize, usize)> = Vec::new();
                    for (_, pos) in &scored {
                        let pos = *pos;
                        // Skip if this position falls inside an already-emitted window
                        if covered.iter().any(|&(s, e)| pos >= s && pos < e) {
                            continue;
                        }
                        let start = floor_char_boundary(
                            &content,
                            pos.saturating_sub(150).max(body_start),
                        );
                        let end = ceil_char_boundary(&content, (pos + 300).min(content.len()));
                        excerpts.push(content[start..end].trim());
                        covered.push((start, end));
                        if excerpts.len() >= 3 {
                            break;
                        }
                    }

                    // Fall back to the start of the body (not the file) if all matches were fuzzy-only
                    if excerpts.is_empty() {
                        let end = (body_start + 400).min(content.len());
                        excerpts.push(content[body_start..end].trim());
                    }

                    // Score = filtered exact match positions; boilerplate hits excluded.
                    let score = positions.len();

                    results.push((
                        score,
                        format!(
                            "• {title}\n  Source: {resource}\n\n  ...{}...",
                            excerpts.join("\n\n  ...")
                        ),
                    ));
                }
            }
        }
    }

    // Sort highest score first so the most relevant document appears at the top
    results.sort_by(|a, b| b.0.cmp(&a.0));

    let text = if results.is_empty() {
        format!("No results found for '{query}'.")
    } else {
        let ranked: Vec<String> = results.into_iter().map(|(_, text)| text).collect();
        format!(
            "Found {} result(s) for '{query}':\n\n{}",
            ranked.len(),
            ranked.join("\n\n")
        )
    };

    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// The MCP server struct. The tool_router field is generated by the #[tool_router]
// macro and holds the dispatch table that routes incoming tool calls to the
// correct method. #[allow(dead_code)] silences the compiler warning about the
// field never being read directly.
#[derive(Clone)]
struct KukaServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<KukaServer>,
}

// #[tool_router] scans this impl block for methods marked #[tool] and wires them
// into the MCP protocol automatically — generating the tool list and dispatch logic.
#[tool_router]
impl KukaServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    // Lightweight health-check tool. Useful for verifying the server is reachable
    // before sending real queries.
    #[tool(description = "Ping the KUKA knowledge server to confirm it is running")]
    fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            "KUKA Knowledge server is online and ready.",
        )]))
    }

    #[tool(description = "List all documents in the KUKA knowledge bundle, grouped by type")]
    fn list_docs(&self) -> Result<CallToolResult, McpError> {
        list_docs_in(&knowledge_dir())
    }

    #[tool(description = "Search KUKA robot documentation for a given query")]
    fn search_docs(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        search_docs_in(&knowledge_dir(), input.query)
    }
}

// #[tool_handler] wires the tool_router into the ServerHandler trait so the MCP
// framework knows how to dispatch incoming tool calls.
#[tool_handler]
impl ServerHandler for KukaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "KUKA AMR robot knowledge server. \
             Use the ping tool to confirm the server is alive."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let knowledge_dir = knowledge_dir();
        let mut resources: Vec<Resource> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&knowledge_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                    let title = extract_frontmatter_field(&content, "title")
                        .unwrap_or_else(|| stem.clone());
                    let description = extract_frontmatter_field(&content, "description");
                    let uri = format!("kuka://docs/{stem}");

                    let mut raw = RawResource::new(uri, stem)
                        .with_title(title)
                        .with_mime_type("text/markdown".to_string());
                    if let Some(desc) = description {
                        raw = raw.with_description(desc);
                    }
                    resources.push(Annotated::new(raw, None));
                }
            }
        }

        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;

        // Strip the kuka://docs/ prefix to recover the file stem
        let stem = uri.strip_prefix("kuka://docs/").ok_or_else(|| McpError {
            code: ErrorCode::RESOURCE_NOT_FOUND,
            message: format!("Unknown resource URI: {uri}").into(),
            data: None,
        })?;

        let path = knowledge_dir().join(format!("{stem}.md"));
        let content = std::fs::read_to_string(&path).map_err(|_| McpError {
            code: ErrorCode::RESOURCE_NOT_FOUND,
            message: format!("Resource not found: {uri}").into(),
            data: None,
        })?;

        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            content,
            uri.clone(),
        )]))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Send tracing output to stderr so it doesn't mix with MCP's stdout messages.
    // Log level is controlled by the RUST_LOG environment variable at runtime.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false) // disable colour codes (they corrupt MCP's JSON stream)
        .init();

    tracing::info!("Starting KUKA MCP server");

    // Attach the server to stdin/stdout and block until the client disconnects
    let service = KukaServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // Bring all functions from main.rs into scope so tests can call them directly
    use super::*;

    // --- normalize_line / repeated_lines / line_at_pos tests ---

    #[test]
    fn repeated_lines_excludes_boilerplate() {
        let text = "KUKA Technical Reference\n\
                    KUKA Technical Reference\n\
                    KUKA Technical Reference\n\
                    KUKA Technical Reference\n\n\
                    Reflectors must be mounted at 150mm height.\n\n\
                    KUKA Technical Reference\n\
                    KUKA Technical Reference";
        let rep = repeated_lines(text);
        assert!(rep.contains("kuka technical reference"), "header should be in boilerplate set");
        assert!(
            !rep.contains("reflectors must be mounted at 150mm height."),
            "body content should not be in boilerplate set"
        );
    }

    #[test]
    fn line_at_pos_returns_correct_line() {
        let text = "first line\nsecond line\nthird line";
        // byte 15 is inside "second line"
        assert_eq!(line_at_pos(text, 15), "second line");
    }

    // --- fuzzy_word_match tests ---

    #[test]
    fn fuzzy_exact_match() {
        // An exact substring match should always return true
        assert!(fuzzy_word_match("reflector deployment guide", "reflector"));
    }

    #[test]
    fn fuzzy_typo_within_threshold() {
        // "reflecor" is 1 edit away from "reflector" (missing 't') — within threshold
        assert!(fuzzy_word_match("reflector deployment guide", "reflecor"));
    }

    #[test]
    fn fuzzy_short_term_requires_exact() {
        // Terms of 3 chars or fewer skip fuzzy matching and need an exact substring match
        assert!(fuzzy_word_match("kuka amr robot", "amr"));
        assert!(!fuzzy_word_match("kuka robot", "amr"));
    }

    #[test]
    fn fuzzy_no_match() {
        // A completely unrelated word should not match
        assert!(!fuzzy_word_match("reflector deployment guide", "hydraulic"));
    }

    // --- extract_frontmatter_field tests ---

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
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::fs;

    // Creates a uniquely-named temp directory and writes one known OKF document into it.
    // Returns TempDir — not PathBuf — so the caller owns the directory and controls
    // when it is dropped. When TempDir goes out of scope, Drop deletes it automatically.
    fn setup_test_bundle() -> tempfile::TempDir {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let doc = "\
---
type: technical-note
title: Reflector Guide
description: Test document for integration tests.
resource: kuka-docs/test.pdf
tags: [test]
timestamp: 2026-01-01T00:00:00Z
---

Reflectors must be mounted at a height of 150 to 2000 mm above floor level.
The maximum spacing between reflectors is 8 metres.";

        fs::write(temp_dir.path().join("reflector-guide.md"), doc).unwrap();
        temp_dir
    }

    #[test]
    fn search_finds_matching_document() {
        let temp_dir = setup_test_bundle();
        let result = search_docs_in(temp_dir.path(), "reflector height".to_string()).unwrap();
        let output = format!("{:?}", result);
        assert!(output.contains("Reflector Guide"), "Should find the document by title");
        assert!(output.contains("150"), "Should include excerpt containing the height value");
    }

    #[test]
    fn search_returns_no_results_for_unknown_query() {
        let temp_dir = setup_test_bundle();
        let result = search_docs_in(temp_dir.path(), "hydraulic pump".to_string()).unwrap();
        let output = format!("{:?}", result);
        assert!(output.contains("No results found"), "Unrelated query should return no results");
    }

    #[test]
    fn list_docs_shows_document_grouped_by_type() {
        let temp_dir = setup_test_bundle();
        let result = list_docs_in(temp_dir.path()).unwrap();
        let output = format!("{:?}", result);
        assert!(output.contains("Reflector Guide"), "Should list the document title");
        assert!(output.contains("technical-note"), "Should group by type from frontmatter");
    }

    #[test]
    fn search_ignores_boilerplate_only_matches() {
        // "kuka" appears ONLY in a repeated header (6 times, well above the threshold of 3).
        // After boilerplate filtering, all positions are removed and the document is skipped.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Maintenance Guide
description: Test for boilerplate filtering.
resource: kuka-docs/test.pdf
tags: []
timestamp: 2026-01-01T00:00:00Z
---

KUKA Manual Header
KUKA Manual Header
KUKA Manual Header
KUKA Manual Header

Only reflector content here with no matching terms.

KUKA Manual Header
KUKA Manual Header";

        fs::write(temp_dir.path().join("maintenance-guide.md"), doc).unwrap();
        let result = search_docs_in(temp_dir.path(), "kuka".to_string()).unwrap();
        let output = format!("{:?}", result);
        assert!(
            output.contains("No results found"),
            "Document matching only via boilerplate header should not appear in results"
        );
    }

    #[test]
    fn search_excerpt_anchors_on_body_not_boilerplate() {
        // "technical" appears in a repeated header (6×) AND in a body sentence.
        // After filtering, only the body position survives, so the excerpt must show
        // "150mm" (body content) rather than anchoring on the repeated header.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Placement Guide
description: Test for excerpt anchoring.
resource: kuka-docs/test.pdf
tags: []
timestamp: 2026-01-01T00:00:00Z
---

Technical Guidance Note
Technical Guidance Note
Technical Guidance Note
Technical Guidance Note

Technical specifications require 150mm minimum clearance for reflectors.

Technical Guidance Note
Technical Guidance Note";

        fs::write(temp_dir.path().join("placement-guide.md"), doc).unwrap();
        let result = search_docs_in(temp_dir.path(), "technical specifications".to_string()).unwrap();
        let output = format!("{:?}", result);
        assert!(
            output.contains("150mm"),
            "Excerpt should come from the body line, not the repeated header"
        );
    }

    #[test]
    fn search_returns_fuzzy_only_match() {
        // "reflecor" (missing 't') has no exact substring in the document, so
        // exact_positions_before_filter is 0 after match_indices. The boilerplate
        // guard must not fire — only exact hits that all land on boilerplate should
        // be skipped. Fuzzy-only matches must fall through to the body-start fallback.
        let temp_dir = setup_test_bundle();
        let result = search_docs_in(temp_dir.path(), "reflecor".to_string()).unwrap();
        let output = format!("{:?}", result);
        assert!(
            !output.contains("No results found"),
            "Fuzzy-only match should be returned, not silently dropped"
        );
        assert!(
            output.contains("Reflector Guide"),
            "Result should include the document title"
        );
    }
}
