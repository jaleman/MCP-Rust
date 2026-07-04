// The search engine: fuzzy matching, boilerplate filtering, proximity scoring,
// and excerpt building. This module is pure domain logic — it takes loaded
// Documents and returns SearchHit data. It knows nothing about MCP: turning
// hits into protocol responses is the binary's job (main.rs).

use crate::bundle::Document;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use strsim::levenshtein; // edit-distance function from the strsim crate

// --- Tuning knobs, in one visible place ---------------------------------

/// ± window (in bytes) used for proximity co-occurrence scoring: positions
/// where several query terms appear close together outrank isolated hits.
const PROXIMITY_WINDOW: usize = 500;

/// Excerpt context around a match position: bytes before and after.
const EXCERPT_BEFORE: usize = 150;
const EXCERPT_AFTER: usize = 300;

/// Maximum number of excerpts shown per document.
const MAX_EXCERPTS: usize = 3;

/// Length of the body-start fallback excerpt used for fuzzy-only matches.
const FALLBACK_EXCERPT_LEN: usize = 400;

/// A normalised line appearing at least this many times counts as boilerplate
/// (running headers, footers, page titles).
const BOILERPLATE_MIN_REPEATS: usize = 3;

/// Terms shorter than this must match exactly — fuzzy matching short words
/// produces too many false positives.
const FUZZY_MIN_TERM_LEN: usize = 4;

/// Terms up to this length tolerate 1 typo; longer terms tolerate 2.
const FUZZY_ONE_TYPO_MAX_LEN: usize = 7;

// Common words that appear in almost every document and carry no search value.
// LazyLock builds the set once, on first use, and shares it for the lifetime
// of the process — previously this HashSet was rebuilt on every search call.
static STOP_WORDS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    HashSet::from([
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "have", "has", "had", "do",
        "does", "did", "will", "would", "can", "could", "should", "may", "might", "shall", "i",
        "you", "he", "she", "it", "we", "they", "what", "which", "who", "when", "where", "why",
        "how", "in", "on", "at", "to", "for", "of", "with", "by", "from", "and", "or", "but",
        "not", "no", "nor",
    ])
});

/// One matching document, ready for whatever presentation layer wants it.
#[derive(Debug)]
pub struct SearchHit {
    pub title: String,
    pub resource: String,
    /// Number of exact match positions after boilerplate filtering.
    pub score: usize,
    /// Up to MAX_EXCERPTS non-overlapping snippets around the best positions.
    pub excerpts: Vec<String>,
}

/// Splits an already-lowercased query into search terms, dropping stop words.
/// Returns borrowed slices into the input — no allocation per term.
pub fn parse_query(query_lower: &str) -> Vec<&str> {
    query_lower
        .split_whitespace()
        .filter(|word| !STOP_WORDS.contains(word))
        .collect()
}

/// Runs the query terms against every document and returns hits sorted by
/// score, highest first. Ties keep bundle order (the sort is stable).
///
/// Callers are expected to have obtained `terms` from parse_query and to have
/// handled the empty-terms case themselves — all() over an empty term list is
/// vacuously true and would match every document.
pub fn search(docs: &[Document], terms: &[&str]) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = docs
        .iter()
        .filter_map(|doc| match_document(doc, terms))
        .collect();
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    hits
}

// Checks one document against the query terms. Returns a scored hit with
// excerpts if EVERY term matches (exactly or fuzzily), None otherwise.
fn match_document(doc: &Document, terms: &[&str]) -> Option<SearchHit> {
    // Lowercase once and reuse for all term checks against this document
    let lower = doc.content.to_lowercase();

    // Body offset was computed once at load time (bundle.rs) — excerpts
    // must never anchor inside the frontmatter block.
    let body_start = doc.body_start;

    // A document qualifies only if every query term matches (exactly or fuzzily)
    if !terms.iter().all(|term| fuzzy_word_match(&lower, term)) {
        return None;
    }

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
    positions.sort_unstable();
    positions.dedup();

    // Build the boilerplate set from the body and remove any position that
    // lands on a repeated line (headers, footers, page titles).
    let exact_positions_before_filter = positions.len();
    let boilerplate = repeated_lines(&lower[body_start..]);
    let positions: Vec<usize> = positions
        .into_iter()
        .filter(|&pos| !boilerplate.contains(&normalize_line(line_at_pos(&lower, pos))))
        .collect();

    // Skip only when exact matches existed but ALL landed on boilerplate lines.
    // If exact_positions_before_filter is 0, this was a fuzzy-only match —
    // let it fall through to the body-start excerpt fallback below.
    if exact_positions_before_filter > 0 && positions.is_empty() {
        return None;
    }

    // Proximity scoring: rank each position by how many distinct query terms
    // appear within ±PROXIMITY_WINDOW around it. Positions where multiple
    // terms co-occur (e.g. "mission" + "command" + "payload" together) score
    // higher than isolated hits in page headers or table-of-contents lines.
    let mut scored: Vec<(usize, usize)> = positions
        .iter()
        .map(|&pos| {
            // floor/ceil snap to nearest valid UTF-8 char boundary before slicing
            let win_start = floor_char_boundary(
                &lower,
                pos.saturating_sub(PROXIMITY_WINDOW).max(body_start),
            );
            let win_end = ceil_char_boundary(&lower, (pos + PROXIMITY_WINDOW).min(lower.len()));
            let window = &lower[win_start..win_end];
            let co_occurrence = terms.iter().filter(|term| window.contains(*term)).count();
            (co_occurrence, pos)
        })
        .collect();

    // Sort highest co-occurrence first; break ties by position (earlier wins)
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    // Build up to MAX_EXCERPTS non-overlapping windows from the best positions
    let mut excerpts: Vec<String> = Vec::new();
    let mut covered: Vec<(usize, usize)> = Vec::new();
    for &(_, pos) in &scored {
        // Skip if this position falls inside an already-emitted window
        if covered.iter().any(|&(s, e)| pos >= s && pos < e) {
            continue;
        }
        let start = floor_char_boundary(
            &doc.content,
            pos.saturating_sub(EXCERPT_BEFORE).max(body_start),
        );
        let end = ceil_char_boundary(&doc.content, (pos + EXCERPT_AFTER).min(doc.content.len()));
        excerpts.push(doc.content[start..end].trim().to_string());
        covered.push((start, end));
        if excerpts.len() >= MAX_EXCERPTS {
            break;
        }
    }

    // Fall back to the start of the body (not the file) if all matches were fuzzy-only
    if excerpts.is_empty() {
        let end = (body_start + FALLBACK_EXCERPT_LEN).min(doc.content.len());
        excerpts.push(doc.content[body_start..end].trim().to_string());
    }

    Some(SearchHit {
        title: doc.title.clone(),
        resource: doc.resource.clone(),
        // Score = filtered exact match positions; boilerplate hits excluded.
        score: positions.len(),
        excerpts,
    })
}

// --- Low-level helpers ---------------------------------------------------

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
    if term.len() < FUZZY_MIN_TERM_LEN {
        return false;
    }
    let threshold = if term.len() <= FUZZY_ONE_TYPO_MAX_LEN { 1 } else { 2 };
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

// Collapses internal whitespace and lowercases a line so that
// "  KUKA Robotics GmbH  " and "kuka robotics gmbh" count as the same line.
// (split_whitespace already ignores leading/trailing whitespace.)
fn normalize_line(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// Returns a HashSet of normalised lines that appear BOILERPLATE_MIN_REPEATS
// or more times in text — running headers, footers, page titles.
fn repeated_lines(text: &str) -> HashSet<String> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let norm = normalize_line(line);
        if !norm.is_empty() {
            *freq.entry(norm).or_insert(0) += 1;
        }
    }
    freq.into_iter()
        .filter(|(_, count)| *count >= BOILERPLATE_MIN_REPEATS)
        .map(|(line, _)| line)
        .collect()
}

// Returns the trimmed line of text that contains the byte at position pos.
fn line_at_pos(text: &str, pos: usize) -> &str {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[pos..].find('\n').map(|i| pos + i).unwrap_or(text.len());
    text[line_start..line_end].trim()
}

#[cfg(test)]
mod tests {
    // Bring all functions from this module into scope so tests can call them directly
    use super::*;

    // --- parse_query tests ---

    #[test]
    fn parse_query_drops_stop_words() {
        assert_eq!(parse_query("what is the reflector height"), vec!["reflector", "height"]);
    }

    #[test]
    fn parse_query_all_stop_words_yields_empty() {
        // The caller must handle this case — all() over empty terms is vacuously true
        assert!(parse_query("what is the").is_empty());
    }

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
        // Terms below FUZZY_MIN_TERM_LEN skip fuzzy matching and need an exact substring
        assert!(fuzzy_word_match("kuka amr robot", "amr"));
        assert!(!fuzzy_word_match("kuka robot", "amr"));
    }

    #[test]
    fn fuzzy_no_match() {
        // A completely unrelated word should not match
        assert!(!fuzzy_word_match("reflector deployment guide", "hydraulic"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::bundle::load_bundle;
    use crate::test_util::setup_test_bundle;
    use std::fs;

    // Convenience: load the bundle and run a query the way the server does.
    fn run_search(dir: &std::path::Path, query: &str) -> Vec<SearchHit> {
        let docs = load_bundle(dir).unwrap();
        let query_lower = query.to_lowercase();
        let terms = parse_query(&query_lower);
        search(&docs, &terms)
    }

    #[test]
    fn search_finds_matching_document() {
        let temp_dir = setup_test_bundle();
        let hits = run_search(temp_dir.path(), "reflector height");

        // Structured assertions — no more substring-matching against Debug output
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Reflector Guide");
        assert_eq!(hits[0].resource, "kuka-docs/test.pdf");
        assert!(hits[0].score > 0, "exact matches should produce a positive score");
        assert!(
            hits[0].excerpts.iter().any(|e| e.contains("150")),
            "an excerpt should contain the height value"
        );
    }

    #[test]
    fn search_returns_no_hits_for_unknown_query() {
        let temp_dir = setup_test_bundle();
        let hits = run_search(temp_dir.path(), "hydraulic pump");
        assert!(hits.is_empty(), "unrelated query should return no hits");
    }

    #[test]
    fn search_ranks_higher_scoring_document_first() {
        // Two documents match; the one with more (non-boilerplate) exact hits
        // must come first. This ordering was previously untestable through
        // the formatted-text API without fragile string offset checks.
        let temp_dir = setup_test_bundle(); // "reflector" appears 2× in this doc
        let doc = "\
---
type: technical-note
title: Reflector Everything
description: Mentions reflectors constantly.
resource: kuka-docs/many.pdf
tags: []
timestamp: 2026-01-01T00:00:00Z
---

Reflector reflector reflector. The reflector is a reflector among reflectors.";
        fs::write(temp_dir.path().join("reflector-everything.md"), doc).unwrap();

        let hits = run_search(temp_dir.path(), "reflector");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Reflector Everything");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn search_ignores_boilerplate_only_matches() {
        // "kuka" appears ONLY in a repeated header (6 times, above the threshold).
        // After boilerplate filtering, all positions are removed and the document
        // is skipped entirely.
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
        let hits = run_search(temp_dir.path(), "kuka");
        assert!(hits.is_empty(), "boilerplate-only match should produce no hit");
    }

    #[test]
    fn search_excerpt_anchors_on_body_not_boilerplate() {
        // "technical" appears in a repeated header (6×) AND in a body sentence.
        // After filtering, only the body position survives, so the excerpt must
        // show "150mm" (body content) rather than anchoring on the header.
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
        let hits = run_search(temp_dir.path(), "technical specifications");

        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].excerpts.iter().any(|e| e.contains("150mm")),
            "excerpt should come from the body line, not the repeated header"
        );
    }

    #[test]
    fn search_returns_fuzzy_only_match() {
        // "reflecor" (missing 't') has no exact substring in the document, so the
        // boilerplate guard must not fire; the hit falls back to a body-start
        // excerpt and a score of 0 (no exact positions).
        let temp_dir = setup_test_bundle();
        let hits = run_search(temp_dir.path(), "reflecor");

        assert_eq!(hits.len(), 1, "fuzzy-only match should be returned, not dropped");
        assert_eq!(hits[0].title, "Reflector Guide");
        assert_eq!(hits[0].score, 0, "fuzzy-only match has no exact positions");
        assert!(
            hits[0].excerpts[0].starts_with("Reflectors must be mounted"),
            "fallback excerpt should start at the body, not the frontmatter"
        );
    }
}
