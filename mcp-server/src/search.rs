// Shared search vocabulary: the SearchHit result type, query parsing, and the
// text-analysis helpers used at index-build time. The actual matching lives in
// index.rs — since step 5b, search runs against a prebuilt inverted index
// instead of scanning documents linearly.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use strsim::levenshtein; // edit-distance function from the strsim crate

// --- Tuning knobs, in one visible place ---------------------------------

/// ± window (in bytes) used for proximity co-occurrence scoring: positions
/// where several query terms appear close together outrank isolated hits.
pub(crate) const PROXIMITY_WINDOW: usize = 500;

/// Excerpt context around a match position: bytes before and after.
pub(crate) const EXCERPT_BEFORE: usize = 150;
pub(crate) const EXCERPT_AFTER: usize = 300;

/// Maximum number of excerpts shown per document.
pub(crate) const MAX_EXCERPTS: usize = 3;

/// A normalised line appearing at least this many times counts as boilerplate
/// (running headers, footers, page titles).
pub(crate) const BOILERPLATE_MIN_REPEATS: usize = 3;

/// Terms shorter than this must match exactly — fuzzy matching short words
/// produces too many false positives.
pub(crate) const FUZZY_MIN_TERM_LEN: usize = 4;

/// Terms up to this length tolerate 1 typo; longer terms tolerate 2.
pub(crate) const FUZZY_ONE_TYPO_MAX_LEN: usize = 7;

// Common words that appear in almost every document and carry no search value.
// LazyLock builds the set once, on first use, and shares it for the lifetime
// of the process.
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
    /// Term frequency normalised by document length, scaled ×1000 so it
    /// stays an integer. Longer documents no longer win just by being long.
    pub score: usize,
    /// Up to MAX_EXCERPTS non-overlapping snippets around the best positions.
    pub excerpts: Vec<String>,
}

/// Splits an already-lowercased query into search terms, dropping stop words.
/// Terms break on any non-alphanumeric character — the same rule the index
/// tokenizer uses — so "e-stop" queries the tokens "e" and "stop".
/// Returns borrowed slices into the input — no allocation per term.
pub fn parse_query(query_lower: &str) -> Vec<&str> {
    query_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !STOP_WORDS.contains(word))
        .collect()
}

// Is `word` within the typo tolerance of the query term? The tolerance scales
// with term length; terms below FUZZY_MIN_TERM_LEN never fuzzy-match.
// The length pre-filter skips the expensive levenshtein() call when the two
// strings cannot possibly be within distance — this is what keeps a fuzzy
// scan over the whole vocabulary cheap.
pub(crate) fn within_typo_tolerance(word: &str, term: &str) -> bool {
    if term.len() < FUZZY_MIN_TERM_LEN {
        return false;
    }
    let threshold = if term.len() <= FUZZY_ONE_TYPO_MAX_LEN { 1 } else { 2 };
    let (wlen, tlen) = (word.len(), term.len());
    if wlen > tlen + threshold || tlen > wlen + threshold {
        return false;
    }
    levenshtein(word, term) <= threshold
}

// Collapses internal whitespace and lowercases a line so that
// "  KUKA Robotics GmbH  " and "kuka robotics gmbh" count as the same line.
pub(crate) fn normalize_line(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// Returns a HashSet of normalised lines that appear BOILERPLATE_MIN_REPEATS
// or more times in text — running headers, footers, page titles.
pub(crate) fn repeated_lines(text: &str) -> HashSet<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_query tests ---

    #[test]
    fn parse_query_drops_stop_words() {
        assert_eq!(parse_query("what is the reflector height"), vec!["reflector", "height"]);
    }

    #[test]
    fn parse_query_all_stop_words_yields_empty() {
        // The caller must handle this case — matching empty terms is vacuous
        assert!(parse_query("what is the").is_empty());
    }

    #[test]
    fn parse_query_splits_on_punctuation() {
        // Same tokenization rule as the index: "e-stop" → "e" + "stop"
        assert_eq!(parse_query("e-stop v2.11"), vec!["e", "stop", "v2", "11"]);
    }

    // --- within_typo_tolerance tests ---

    #[test]
    fn typo_within_threshold_matches() {
        // "reflecor" is 2 edits from "reflectors" — term of 8 chars tolerates 2
        assert!(within_typo_tolerance("reflectors", "reflecor"));
        assert!(within_typo_tolerance("reflector", "reflecor"));
    }

    #[test]
    fn short_terms_never_fuzzy_match() {
        // Terms below FUZZY_MIN_TERM_LEN must match exactly elsewhere
        assert!(!within_typo_tolerance("arm", "amr"));
    }

    #[test]
    fn unrelated_words_do_not_match() {
        assert!(!within_typo_tolerance("hydraulic", "reflector"));
    }

    #[test]
    fn length_prefilter_rejects_impossible_pairs() {
        // 9 vs 4 chars can never be within edit distance 1
        assert!(!within_typo_tolerance("reflector", "refl"));
    }

    // --- normalize_line / repeated_lines tests ---

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
}
