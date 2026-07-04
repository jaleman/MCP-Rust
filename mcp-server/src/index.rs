// The inverted index: term → postings, built once from the knowledge bundle.
//
// Query cost and memory are now independent of corpus size:
//   - matching consults the vocabulary (unique words, grows slowly) instead
//     of scanning every word of every document;
//   - document BODIES are not kept in memory — excerpts are read from disk
//     by seeking to the recorded byte offsets;
//   - boilerplate (repeated headers/footers) is filtered once at build time,
//     so those tokens never even enter the index;
//   - positions are recorded per token in the ORIGINAL file bytes. Tokens are
//     lowercased individually for the vocabulary key, so there is no
//     lowercased copy of the document whose offsets could disagree with the
//     original (the classic to_lowercase()-changes-byte-length hazard).
//
// If the file changes on disk after the index was built, excerpt offsets go
// stale until reload — the reload_docs tool rebuilds the index on demand.

use crate::bundle::load_bundle;
use crate::search::{
    EXCERPT_AFTER, EXCERPT_BEFORE, MAX_EXCERPTS, PROXIMITY_WINDOW, SearchHit, normalize_line,
    repeated_lines, within_typo_tolerance,
};
use anyhow::Result;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Positions stored per (term, document). Positions are the only part of the
/// index that grows with corpus size rather than vocabulary size — uncapped
/// they would cost ~one u32 per word of text. Sixteen anchors is far more
/// than the MAX_EXCERPTS ever shown; `freq` keeps scoring honest beyond the cap.
const MAX_POSITIONS_PER_POSTING: usize = 16;

/// Everything the server needs to know about one document WITHOUT its body:
/// listing metadata, plus the file path and body offset for excerpt reads.
#[derive(Debug)]
pub struct DocMeta {
    pub path: PathBuf,
    pub stem: String,
    pub title: String,
    pub doc_type: String,
    pub resource: String,
    pub description: Option<String>,
    pub body_start: usize,
    /// Non-boilerplate tokens in the body — the denominator for scoring.
    token_count: usize,
}

// One entry in a term's posting list: which document, how often, and where.
struct Posting {
    doc_id: u32,
    freq: u32,
    positions: Vec<u32>, // byte offsets into the ORIGINAL file, capped
}

pub struct Index {
    docs: Vec<DocMeta>, // doc_id = position in this Vec
    vocab: HashMap<String, Vec<Posting>>,
}

impl Index {
    /// Loads the bundle and indexes every document. A missing bundle
    /// directory is an error (same contract as load_bundle).
    pub fn build(dir: &Path) -> Result<Index> {
        let documents = load_bundle(dir)?;
        let mut docs: Vec<DocMeta> = Vec::new();
        let mut vocab: HashMap<String, Vec<Posting>> = HashMap::new();

        for doc in documents {
            let doc_id = docs.len() as u32;

            // Boilerplate is decided per document, once, at build time.
            let boilerplate = repeated_lines(doc.body());

            let mut token_count = 0usize;

            // Walk the body line by line (tracking the running byte offset)
            // so whole boilerplate lines are skipped before tokenizing.
            let mut line_start = doc.body_start;
            for line in doc.content[doc.body_start..].split_inclusive('\n') {
                if !boilerplate.contains(&normalize_line(line)) {
                    for (offset_in_line, key) in tokenize(line) {
                        token_count += 1;
                        let pos = (line_start + offset_in_line) as u32;

                        let postings = vocab.entry(key).or_default();
                        // Documents are indexed in ascending doc_id order, so
                        // this term's last posting is ours iff it has our id —
                        // posting lists stay sorted by doc_id for free.
                        match postings.last_mut() {
                            Some(p) if p.doc_id == doc_id => {
                                p.freq += 1;
                                if p.positions.len() < MAX_POSITIONS_PER_POSTING {
                                    p.positions.push(pos);
                                }
                            }
                            _ => postings.push(Posting {
                                doc_id,
                                freq: 1,
                                positions: vec![pos],
                            }),
                        }
                    }
                }
                line_start += line.len();
            }

            docs.push(DocMeta {
                path: doc.path,
                stem: doc.stem,
                title: doc.title,
                doc_type: doc.doc_type,
                resource: doc.resource,
                description: doc.description,
                body_start: doc.body_start,
                token_count,
            });
            // doc.content is dropped here — bodies never stay in memory
        }

        Ok(Index { docs, vocab })
    }

    pub fn docs(&self) -> &[DocMeta] {
        &self.docs
    }

    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    pub fn term_count(&self) -> usize {
        self.vocab.len()
    }

    // All vocabulary keys that count as a match for one query term:
    // substring containment (covers exact matches and "reflector" finding
    // "reflectors") or typo tolerance. This scans unique TERMS, not document
    // text — the vocabulary is tiny compared to the corpus and grows slowly.
    fn matching_keys(&self, term: &str) -> Vec<&str> {
        self.vocab
            .keys()
            .filter(|key| key.contains(term) || within_typo_tolerance(key, term))
            .map(String::as_str)
            .collect()
    }

    /// Runs parsed query terms against the index. Every term must match at
    /// least one vocabulary key in a document for it to qualify (AND
    /// semantics, as before). Returns hits sorted by score, highest first.
    pub fn search(&self, terms: &[&str]) -> Vec<SearchHit> {
        if terms.is_empty() {
            return Vec::new();
        }

        // Per term: doc_id → (total freq, merged positions)
        let mut per_term: Vec<HashMap<u32, (u32, Vec<u32>)>> = Vec::new();
        for term in terms {
            let mut docs_for_term: HashMap<u32, (u32, Vec<u32>)> = HashMap::new();
            for key in self.matching_keys(term) {
                for posting in &self.vocab[key] {
                    let entry = docs_for_term.entry(posting.doc_id).or_default();
                    entry.0 += posting.freq;
                    entry.1.extend(&posting.positions);
                }
            }
            // AND semantics: a term that matches nowhere empties the result
            if docs_for_term.is_empty() {
                return Vec::new();
            }
            per_term.push(docs_for_term);
        }

        // Intersect: candidate documents contain every term. Sorted so equal
        // scores keep bundle order deterministically.
        let mut candidates: Vec<u32> = per_term[0]
            .keys()
            .copied()
            .filter(|id| per_term.iter().all(|m| m.contains_key(id)))
            .collect();
        candidates.sort_unstable();

        let mut hits: Vec<SearchHit> = Vec::new();
        for doc_id in candidates {
            let meta = &self.docs[doc_id as usize];

            let freq_total: u32 = per_term.iter().map(|m| m[&doc_id].0).sum();
            let per_term_positions: Vec<&Vec<u32>> =
                per_term.iter().map(|m| &m[&doc_id].1).collect();

            // Candidate anchors: every recorded position of every term
            let mut anchors: Vec<u32> = per_term_positions
                .iter()
                .flat_map(|positions| positions.iter().copied())
                .collect();
            anchors.sort_unstable();
            anchors.dedup();

            // Proximity: prefer anchors where many DISTINCT terms appear
            // within the window — co-occurrence beats isolated mentions.
            let mut scored: Vec<(usize, u32)> = anchors
                .iter()
                .map(|&pos| {
                    let co_occurrence = per_term_positions
                        .iter()
                        .filter(|positions| {
                            positions.iter().any(|&q| q.abs_diff(pos) as usize <= PROXIMITY_WINDOW)
                        })
                        .count();
                    (co_occurrence, pos)
                })
                .collect();
            scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

            // Excerpts: read small byte ranges from disk around the best
            // anchors — the document body is never loaded whole.
            let mut excerpts: Vec<String> = Vec::new();
            let mut covered: Vec<(u32, u32)> = Vec::new();
            for &(_, pos) in &scored {
                if covered.iter().any(|&(s, e)| pos >= s && pos < e) {
                    continue;
                }
                let start = pos
                    .saturating_sub(EXCERPT_BEFORE as u32)
                    .max(meta.body_start as u32);
                let end = pos + EXCERPT_AFTER as u32;
                if let Ok(text) = read_excerpt(&meta.path, u64::from(start), u64::from(end - start))
                {
                    excerpts.push(text);
                    covered.push((start, end));
                }
                if excerpts.len() >= MAX_EXCERPTS {
                    break;
                }
            }

            hits.push(SearchHit {
                title: meta.title.clone(),
                resource: meta.resource.clone(),
                // Frequency normalised by document length (×1000 to stay
                // integral): a focused 2-page note now outranks a long manual
                // with the same number of scattered mentions.
                score: (freq_total as usize * 1000) / meta.token_count.max(1),
                excerpts,
            });
        }

        hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
        hits
    }
}

// Splits one line into (byte_offset, lowercased_key) tokens. A token is a run
// of alphanumeric characters; everything else separates. The KEY is
// lowercased per token, but the offset always points into the ORIGINAL bytes.
fn tokenize(line: &str) -> Vec<(usize, String)> {
    let mut tokens: Vec<(usize, String)> = Vec::new();
    let mut start: Option<usize> = None;

    for (i, ch) in line.char_indices() {
        if ch.is_alphanumeric() {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            tokens.push((s, line[s..i].to_lowercase()));
        }
    }
    if let Some(s) = start {
        tokens.push((s, line[s..].to_lowercase()));
    }
    tokens
}

// Reads `len` bytes at `start` from the file and returns them as trimmed text.
// The seek may land mid-character or mid-word; from_utf8_lossy turns partial
// characters at the edges into U+FFFD, which we trim away with the whitespace.
fn read_excerpt(path: &Path, start: u64, len: u64) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;

    let mut buf = Vec::with_capacity(len as usize);
    file.take(len).read_to_end(&mut buf)?;

    Ok(String::from_utf8_lossy(&buf)
        .trim_matches(|c: char| c == '\u{FFFD}' || c.is_whitespace())
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::parse_query;
    use crate::test_util::setup_test_bundle;
    use std::fs;

    // Build the index and run a query the way the server does.
    fn run_search(dir: &Path, query: &str) -> Vec<SearchHit> {
        let index = Index::build(dir).unwrap();
        let query_lower = query.to_lowercase();
        let terms = parse_query(&query_lower);
        index.search(&terms)
    }

    // --- tokenizer ---

    #[test]
    fn tokenize_records_original_offsets_and_lowercased_keys() {
        let line = "Größe: KUKA.AMR v2!";
        let tokens = tokenize(line);
        let keys: Vec<&str> = tokens.iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(keys, vec!["größe", "kuka", "amr", "v2"]);

        // Offsets must point into the ORIGINAL string ("Größe" has a
        // 2-byte ö, so "KUKA" starts at byte 8, not 7)
        assert_eq!(tokens[0].0, 0);
        assert_eq!(&line[tokens[1].0..tokens[1].0 + 4], "KUKA");
        assert_eq!(&line[tokens[2].0..tokens[2].0 + 3], "AMR");
    }

    // --- build ---

    #[test]
    fn build_errors_on_missing_directory() {
        assert!(Index::build(Path::new("no-such-directory-anywhere")).is_err());
    }

    #[test]
    fn build_indexes_documents_and_terms() {
        let temp_dir = setup_test_bundle();
        let index = Index::build(temp_dir.path()).unwrap();
        assert_eq!(index.doc_count(), 1);
        assert!(index.term_count() > 10, "body words should be in the vocabulary");
        assert_eq!(index.docs()[0].title, "Reflector Guide");
    }

    #[test]
    fn boilerplate_lines_never_enter_the_index() {
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

        let index = Index::build(temp_dir.path()).unwrap();
        // "kuka" appears only on the repeated header lines → filtered at
        // build time → not in the vocabulary at all
        assert!(!index.vocab.contains_key("kuka"));
        assert!(index.vocab.contains_key("reflector"));
    }

    #[test]
    fn positions_are_capped_but_freq_keeps_counting() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let body = "reflector ".repeat(50);
        let doc = format!(
            "---\ntype: t\ntitle: Cap Test\nresource: r\n---\n\n{body}"
        );
        fs::write(temp_dir.path().join("cap-test.md"), doc).unwrap();

        let index = Index::build(temp_dir.path()).unwrap();
        let postings = &index.vocab["reflector"];
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].freq, 50, "freq counts every occurrence");
        assert_eq!(
            postings[0].positions.len(),
            MAX_POSITIONS_PER_POSTING,
            "positions stop at the cap"
        );
    }

    // --- search (ported from the linear-scan integration tests) ---

    #[test]
    fn search_finds_matching_document() {
        let temp_dir = setup_test_bundle();
        let hits = run_search(temp_dir.path(), "reflector height");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Reflector Guide");
        assert_eq!(hits[0].resource, "kuka-docs/test.pdf");
        assert!(hits[0].score > 0);
        assert!(
            hits[0].excerpts.iter().any(|e| e.contains("150")),
            "an excerpt read from disk should contain the height value"
        );
    }

    #[test]
    fn search_returns_no_hits_for_unknown_query() {
        let temp_dir = setup_test_bundle();
        assert!(run_search(temp_dir.path(), "hydraulic pump").is_empty());
    }

    #[test]
    fn search_ranks_focused_document_first() {
        // Length-normalised scoring: the document DENSE in the term wins,
        // even against one with more absolute mentions spread over more text.
        let temp_dir = setup_test_bundle();
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

        assert!(
            run_search(temp_dir.path(), "kuka").is_empty(),
            "boilerplate-only match must produce no hit"
        );
    }

    #[test]
    fn search_excerpt_anchors_on_body_not_boilerplate() {
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
    fn search_matches_typo_via_vocabulary() {
        // "reflecor" fuzzy-matches the vocabulary keys "reflector(s)". Unlike
        // the old linear scan, fuzzy matches now carry real positions — the
        // hit gets a genuine excerpt and a nonzero score.
        let temp_dir = setup_test_bundle();
        let hits = run_search(temp_dir.path(), "reflecor");

        assert_eq!(hits.len(), 1, "typo query should still find the document");
        assert_eq!(hits[0].title, "Reflector Guide");
        assert!(hits[0].score > 0, "fuzzy matches carry real positions now");
        assert!(hits[0].excerpts[0].contains("Reflectors"));
    }

    #[test]
    fn search_substring_matches_short_terms() {
        // "amr" is too short for fuzzy but must still match "KUKA.AMR"
        // (tokenized as "kuka" + "amr") and substrings like "amrs".
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Fleet Note
resource: kuka-docs/fleet.pdf
---

The KUKA.AMR fleet manager coordinates all vehicles.";
        fs::write(temp_dir.path().join("fleet-note.md"), doc).unwrap();

        let hits = run_search(temp_dir.path(), "amr");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Fleet Note");
    }
}
