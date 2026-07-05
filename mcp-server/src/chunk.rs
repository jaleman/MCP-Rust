// Preparing extracted document text for the knowledge bundle: cleaning out
// non-content lines, then splitting into chunk-sized pieces along page
// boundaries. pdftotext separates pages with a form-feed character (\x0c),
// which gives us accurate section boundaries for free; pages are accumulated
// until a chunk reaches the target size.
//
// Why chunk at all: search excerpts, MCP resources, and AI-agent context
// windows all want document units of a few KB — not 300-page manuals. One
// OKF file per chunk keeps every downstream consumer simple, and page-range
// provenance ("pages 12-18 of the manual") makes search results MORE useful,
// not less.
//
// Why clean at EXTRACT time (not query time): if repeated headers/footers and
// table-of-contents lines never reach the bundle files, then excerpts,
// resources, and every future consumer are clean — not just search anchoring.

use crate::search::{normalize_line, repeated_lines};

/// A line containing a run of dots this long is a table-of-contents entry
/// ("Introduction ........... 3") — navigation, not content. TOC lines are
/// especially harmful in search results: they contain every section title
/// packed close together, so they outscore the real content they point at.
const TOC_DOT_RUN: &str = ".....";

/// Cleans raw extracted text before chunking: strips repeated header/footer
/// lines (running titles, "Page N of M", classification banners) and TOC
/// dot-leader lines, while preserving page boundaries (\x0c) and blank lines
/// (paragraph structure).
pub fn clean_extracted_text(text: &str) -> String {
    // Repetition is counted across the WHOLE document — a header only counts
    // as boilerplate because it appears on many pages. Page breaks (\x0c) are
    // treated as line breaks for counting; otherwise a header that directly
    // follows a page break would merge into the previous line and escape the
    // count (str::lines only splits on \n).
    let boilerplate = repeated_lines(&text.replace('\x0c', "\n"));

    text.split('\x0c')
        .map(|page| {
            page.lines()
                .filter(|line| {
                    let norm = normalize_line(line);
                    // Blank lines carry paragraph structure — always keep
                    norm.is_empty()
                        || (!boilerplate.contains(&norm) && !line.contains(TOC_DOT_RUN))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\x0c")
}

/// Target chunk size. Big enough for coherent context, small enough that an
/// agent can read several hits without flooding its context window.
pub const CHUNK_TARGET_BYTES: usize = 8 * 1024;

/// A page with more than twice the target on its own gets sub-split on
/// paragraph boundaries (covers extractors that emit no form feeds at all,
/// where the whole document arrives as one "page").
const OVERSIZED_PAGE_BYTES: usize = CHUNK_TARGET_BYTES * 2;

/// One chunk of a document: its text and the 1-based page range it covers.
#[derive(Debug)]
pub struct Chunk {
    pub text: String,
    pub first_page: usize,
    pub last_page: usize,
}

/// Splits text into chunks of roughly CHUNK_TARGET_BYTES along page
/// boundaries. Empty pages are skipped (their page numbers still count, so
/// ranges stay aligned with the PDF). A document that fits in one chunk
/// yields exactly one Chunk — the caller can treat that as "no chunking".
pub fn chunk_pages(text: &str) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut cur_text = String::new();
    let mut cur_first = 0; // 1-based page number; 0 = no open chunk
    let mut cur_last = 0;

    for (i, raw_page) in text.split('\x0c').enumerate() {
        let page_no = i + 1;
        let page = raw_page.trim();
        if page.is_empty() {
            continue;
        }

        // A single page far beyond the target can't be combined with anything —
        // close the open chunk and sub-split this page on paragraph boundaries.
        if page.len() > OVERSIZED_PAGE_BYTES {
            if !cur_text.is_empty() {
                chunks.push(Chunk {
                    text: std::mem::take(&mut cur_text),
                    first_page: cur_first,
                    last_page: cur_last,
                });
            }
            for part in split_oversized_page(page) {
                chunks.push(Chunk {
                    text: part,
                    first_page: page_no,
                    last_page: page_no,
                });
            }
            continue;
        }

        // Would this page overflow the open chunk? Flush it first.
        if !cur_text.is_empty() && cur_text.len() + 2 + page.len() > CHUNK_TARGET_BYTES {
            chunks.push(Chunk {
                text: std::mem::take(&mut cur_text),
                first_page: cur_first,
                last_page: cur_last,
            });
        }

        if cur_text.is_empty() {
            cur_first = page_no;
        } else {
            cur_text.push_str("\n\n");
        }
        cur_text.push_str(page);
        cur_last = page_no;
    }

    if !cur_text.is_empty() {
        chunks.push(Chunk {
            text: cur_text,
            first_page: cur_first,
            last_page: cur_last,
        });
    }

    chunks
}

// Splits one oversized page on blank-line paragraph boundaries, accumulating
// paragraphs up to the target. A single paragraph larger than the target is
// emitted as-is — we never split mid-paragraph (documented limitation; such
// text is usually extraction noise like embedded data tables).
fn split_oversized_page(page: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();

    for para in page.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if !cur.is_empty() && cur.len() + 2 + para.len() > CHUNK_TARGET_BYTES {
            parts.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- clean_extracted_text ---

    #[test]
    fn clean_strips_repeated_headers_and_keeps_content() {
        let text = "KUKA ROBOTICS CORP.\nReal content about reflectors.\x0c\
                    KUKA ROBOTICS CORP.\nMore real content.\x0c\
                    KUKA ROBOTICS CORP.\nFinal page content.";
        let cleaned = clean_extracted_text(text);
        assert!(!cleaned.contains("KUKA ROBOTICS CORP."), "repeated header must be stripped");
        assert!(cleaned.contains("Real content about reflectors."));
        assert!(cleaned.contains("Final page content."));
    }

    #[test]
    fn clean_strips_toc_dot_leader_lines() {
        let text = "Contents\nIntroduction ................... 3\n\
                    Minimum Safe Distances ........ 5\n\nReal introduction text.";
        let cleaned = clean_extracted_text(text);
        assert!(!cleaned.contains("..........."), "TOC dot leaders must be stripped");
        assert!(!cleaned.contains("Minimum Safe Distances ........"));
        assert!(cleaned.contains("Real introduction text."));
    }

    #[test]
    fn clean_preserves_page_boundaries_and_blank_lines() {
        let text = "Header\nPage one text.\n\nSecond paragraph.\x0cHeader\nPage two text.\x0cHeader\nPage three.";
        let cleaned = clean_extracted_text(text);
        // "Header" repeats 3× → stripped, but the form feeds must survive
        assert_eq!(cleaned.matches('\x0c').count(), 2, "page separators must be preserved");
        assert!(cleaned.contains("\n\n"), "blank lines (paragraph structure) must survive");
    }

    #[test]
    fn clean_keeps_lines_appearing_fewer_than_three_times() {
        let text = "Unique heading\nBody text here.\x0cAnother unique heading\nMore body.";
        let cleaned = clean_extracted_text(text);
        assert!(cleaned.contains("Unique heading"), "non-repeated lines are content");
    }

    // Builds a fake N-page document where each page has the given size.
    fn pages_of(sizes: &[usize]) -> String {
        sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| format!("Page {} starts here. {}", i + 1, "x ".repeat(size / 2)))
            .collect::<Vec<_>>()
            .join("\x0c")
    }

    #[test]
    fn small_document_is_one_chunk() {
        let chunks = chunk_pages("A short document.\nOne page, little text.");
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].first_page, chunks[0].last_page), (1, 1));
    }

    #[test]
    fn small_multipage_document_is_still_one_chunk() {
        // 3 pages × ~1 KB — total well under the target, so no chunking
        let text = pages_of(&[1000, 1000, 1000]);
        let chunks = chunk_pages(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].first_page, chunks[0].last_page), (1, 3));
    }

    #[test]
    fn large_document_chunks_on_page_boundaries() {
        // 10 pages × ~3 KB → ~30 KB total → expect ~4 chunks of 2-3 pages
        let text = pages_of(&[3000; 10]);
        let chunks = chunk_pages(&text);

        assert!(chunks.len() >= 3, "30 KB should split into several chunks");
        // Page ranges must be contiguous and cover 1..=10 in order
        assert_eq!(chunks.first().unwrap().first_page, 1);
        assert_eq!(chunks.last().unwrap().last_page, 10);
        for pair in chunks.windows(2) {
            assert_eq!(
                pair[1].first_page,
                pair[0].last_page + 1,
                "chunks must not skip or overlap pages"
            );
        }
        // No chunk should be wildly above target (max: target + one page)
        for c in &chunks {
            assert!(c.text.len() <= CHUNK_TARGET_BYTES + 3100);
        }
    }

    #[test]
    fn no_text_is_lost_in_chunking() {
        let text = pages_of(&[3000; 10]);
        let chunks = chunk_pages(&text);
        // Every page's marker sentence must appear in exactly one chunk
        for page_no in 1..=10 {
            let marker = format!("Page {page_no} starts here.");
            let hits = chunks.iter().filter(|c| c.text.contains(&marker)).count();
            assert_eq!(hits, 1, "page {page_no} must appear in exactly one chunk");
        }
    }

    #[test]
    fn empty_pages_are_skipped_but_numbering_is_kept() {
        // Page 2 is blank (e.g. a diagram-only page): page numbers must still
        // line up with the PDF, so the next chunk starts at page 3.
        let text = format!("{}\x0c   \x0c{}", "a ".repeat(3000), "b ".repeat(3000));
        let chunks = chunk_pages(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].first_page, chunks[0].last_page), (1, 1));
        assert_eq!((chunks[1].first_page, chunks[1].last_page), (3, 3));
    }

    #[test]
    fn oversized_single_page_splits_on_paragraphs() {
        // One "page" (no form feeds — like pdf-extract output) of ~24 KB in
        // 1 KB paragraphs → must be sub-split near the target size
        let page = (0..24)
            .map(|i| format!("Paragraph {i}. {}", "y ".repeat(500)))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk_pages(&page);

        assert!(chunks.len() >= 2, "oversized page must be sub-split");
        for c in &chunks {
            assert_eq!((c.first_page, c.last_page), (1, 1), "all parts belong to page 1");
            assert!(c.text.len() <= CHUNK_TARGET_BYTES + 1100);
        }
    }

    #[test]
    fn whitespace_only_input_yields_no_chunks() {
        assert!(chunk_pages("  \n \x0c  \n ").is_empty());
    }
}
