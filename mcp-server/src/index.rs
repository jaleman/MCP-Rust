// The inverted index: term → postings, built once from the knowledge bundle.
//
// Query cost and memory are now independent of corpus size:
//   - matching consults the vocabulary (unique words, grows slowly) instead
//     of scanning every word of every document;
//   - document BODIES are not kept in memory — excerpts are read from disk
//     by seeking to the recorded byte offsets;
//   - the bundle is trusted as already CLEAN: headers/footers/TOC lines are
//     stripped at extract time (chunk.rs), where page structure still exists;
//   - positions are recorded per token in the ORIGINAL file bytes. Tokens are
//     lowercased individually for the vocabulary key, so there is no
//     lowercased copy of the document whose offsets could disagree with the
//     original (the classic to_lowercase()-changes-byte-length hazard).
//
// If the file changes on disk after the index was built, excerpt offsets go
// stale until reload — the reload_docs tool rebuilds the index on demand.

use crate::bundle::load_bundle;
use crate::search::{
    EXCERPT_AFTER, EXCERPT_BEFORE, MAX_EXCERPTS, MIN_SUBSTRING_TERM_LEN, PROXIMITY_WINDOW,
    SearchHit, within_typo_tolerance,
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
    /// Diagram filenames under knowledge/images/ for this document.
    pub images: Vec<String>,
    /// Tokens in the body — the denominator for length-normalised scoring.
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

            // The bundle is trusted as already clean — headers/footers and
            // TOC lines are stripped at EXTRACT time, where page structure
            // still exists. An index-time repeated-lines filter used to live
            // here and was removed deliberately: chunks carry no page info,
            // so it could not tell a running header from legitimately
            // repeated content, and it silently un-indexed a real lookup
            // table (the RobotType incident — see lesson refactor-12).
            let mut token_count = 0usize;

            for (offset_in_body, key) in tokenize(doc.body()) {
                token_count += 1;
                let pos = (doc.body_start + offset_in_body) as u32;

                let postings = vocab.entry(key).or_default();
                // Documents are indexed in ascending doc_id order, so this
                // term's last posting is ours iff it has our id — posting
                // lists stay sorted by doc_id for free.
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

            docs.push(DocMeta {
                path: doc.path,
                stem: doc.stem,
                title: doc.title,
                doc_type: doc.doc_type,
                resource: doc.resource,
                description: doc.description,
                body_start: doc.body_start,
                images: doc.images,
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
            .filter(|key| {
                if term.len() < MIN_SUBSTRING_TERM_LEN {
                    key.as_str() == term
                } else {
                    key.contains(term) || within_typo_tolerance(key, term)
                }
            })
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
                stem: meta.stem.clone(),
                images: meta.images.clone(),
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
    fn index_trusts_bundle_content_including_repeated_lines() {
        // The index has NO repetition filter of its own — cleaning happens at
        // extract time where page structure exists. A legitimately repeated
        // line in a bundle file (a lookup table printed under three payload
        // sections) MUST be indexed; an old index-time filter dropped exactly
        // this and made the RobotType table unsearchable.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Payload Guide
resource: kuka-docs/test.pdf
---

MissionCommand fields:
Code 0 means KMP 250P
MultiMissionCommand fields:
Code 0 means KMP 250P
MultiWorkflowCommand fields:
Code 0 means KMP 250P";
        fs::write(temp_dir.path().join("payload-guide.md"), doc).unwrap();

        let index = Index::build(temp_dir.path()).unwrap();
        assert!(index.vocab.contains_key("250p"), "repeated content must be indexed");
        assert_eq!(index.vocab["250p"][0].freq, 3, "every repetition counts");

        let hits = run_search(temp_dir.path(), "250p");
        assert_eq!(hits.len(), 1, "the repeated table must be searchable");
    }

    #[test]
    fn extract_cleaning_and_index_work_end_to_end() {
        // The pipeline contract: raw extractor output goes through
        // clean_extracted_text before landing in the bundle. Page-edge
        // headers disappear (unsearchable); mid-page repeated content stays.
        use crate::chunk::clean_extracted_text;

        let filler: String = (0..12).map(|i| format!("Filler sentence number {i}.\n")).collect();
        let page = |body: &str| format!("KUKA MANUAL HEADER\n{filler}{body}\n");
        let raw = format!(
            "{}\x0c{}\x0c{}",
            page("Reflector spacing is 8 metres."),
            page("Reflector height is 150 mm."),
            page("Reflector diameter is 50 mm.")
        );

        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = format!(
            "---\ntype: technical-note\ntitle: Cleaned Guide\nresource: kuka-docs/test.pdf\n---\n\n{}",
            clean_extracted_text(&raw)
        );
        fs::write(temp_dir.path().join("cleaned-guide.md"), doc).unwrap();

        assert!(
            run_search(temp_dir.path(), "manual header").is_empty(),
            "page-edge header was cleaned at extract time — nothing to find"
        );
        let hits = run_search(temp_dir.path(), "reflector height");
        assert_eq!(hits.len(), 1, "content survives cleaning and is searchable");
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
        assert_eq!(hits[0].stem, "reflector-guide", "stem is the resource identifier");
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
    fn matching_keys_requires_exact_match_below_minimum_term_length() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let doc = "\
---
type: technical-note
title: Dated Note
resource: kuka-docs/dated.pdf
---

This note was revised in 2026.";
        fs::write(temp_dir.path().join("dated-note.md"), doc).unwrap();

        assert!(
            run_search(temp_dir.path(), "2").is_empty(),
            "single-character terms must not substring-match tokens like 2026"
        );
        assert_eq!(run_search(temp_dir.path(), "2026").len(), 1);
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
