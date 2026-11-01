//! Public Rust API regression cases backed by the BASE-01 on-disk corpus.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use perch_core::{epub, index::Index, jobs, model::Format, pdf, txt};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    profile: String,
    scale: u32,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
struct Sample {
    id: String,
    path: String,
    bytes: usize,
    sha256: String,
    format: Format,
    encoding: String,
    decoded: Option<TextInfo>,
    indexed: Option<TextInfo>,
    epub_version: Option<String>,
    title: Option<String>,
    page_count: Option<u32>,
    #[serde(default)]
    toc: Vec<String>,
    #[serde(default)]
    chapters: Vec<Chapter>,
    #[serde(default)]
    queries: Vec<Query>,
    #[serde(default)]
    outline: Vec<Outline>,
}

#[derive(Deserialize)]
struct TextInfo {
    path: String,
    chars: u64,
    utf8_bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
struct Chapter {
    href: String,
    start_char: u64,
    end_char: u64,
}

#[derive(Deserialize)]
struct Query {
    query: String,
    chunk_chars: u64,
    ranges: Vec<[u64; 2]>,
}

#[derive(Deserialize)]
struct Outline {
    title: String,
    page_index: u32,
    #[serde(default)]
    children: Vec<Outline>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data/corpus")
}

fn manifest() -> Manifest {
    let data = std::fs::read(root().join("manifest.json")).unwrap();
    let manifest: Manifest = serde_json::from_slice(&data).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.profile, "correctness");
    assert_eq!(manifest.scale, 1);
    manifest
}

fn content(sample: &Sample) -> Vec<u8> {
    std::fs::read(root().join(&sample.path)).unwrap()
}

fn expected(info: &TextInfo) -> String {
    std::fs::read_to_string(root().join(&info.path)).unwrap()
}

#[test]
fn corpus_sizes_fingerprints_and_character_counts_match() {
    let manifest = manifest();
    assert_eq!(manifest.samples.len(), 9);
    for sample in manifest.samples {
        let bytes = content(&sample);
        assert_eq!(bytes.len(), sample.bytes, "{} source size", sample.id);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            sample.sha256,
            "{} source hash",
            sample.id
        );
        assert_eq!(
            jobs::sniff_format(&bytes),
            sample.format,
            "{} format",
            sample.id
        );
        for info in [sample.decoded, sample.indexed].into_iter().flatten() {
            let text = expected(&info);
            assert_eq!(text.len(), info.utf8_bytes, "{} text bytes", info.path);
            assert_eq!(
                text.chars().count() as u64,
                info.chars,
                "{} text scalars",
                info.path
            );
            assert_eq!(
                format!("{:x}", Sha256::digest(text.as_bytes())),
                info.sha256,
                "{} text hash",
                info.path
            );
        }
    }
}

#[test]
fn txt_encodings_slices_and_locators_match_decoded_source() {
    for sample in manifest()
        .samples
        .into_iter()
        .filter(|sample| sample.format == Format::Txt)
    {
        let bytes = content(&sample);
        let info = sample.decoded.as_ref().unwrap();
        let text = expected(info);
        let document = txt::TxtDocument::open(root().join(&sample.path)).unwrap();
        let explicit = txt::TxtDocument::with_encoding(&bytes, &sample.encoding).unwrap();
        let decoded = txt::decode(&bytes);
        assert_eq!(document.text(), text, "{} automatic decode", sample.id);
        assert_eq!(explicit.text(), text, "{} explicit decode", sample.id);
        assert_eq!(decoded.text(), text, "{} decode-only", sample.id);
        assert_eq!(document.meta().encoding, sample.encoding);
        assert_eq!(document.meta().source_bytes, bytes.len());
        assert_eq!(document.char_count(), info.chars);
        assert!(!document.meta().had_decode_errors);
        assert!(!decoded.had_decode_errors());
        assert_eq!(document.meta().certain_encoding, sample.encoding == "UTF-8");
        assert!(explicit.meta().certain_encoding);

        // Compare every scalar boundary with std's independent UTF-8 iterator,
        // including emoji, combining marks and both sides of sampling boundaries.
        for (offset, (byte, character)) in text.char_indices().enumerate() {
            let offset = offset as u64;
            assert_eq!(document.char_to_byte(offset), Some(byte));
            assert_eq!(document.byte_to_char(byte), Some(offset));
            assert_eq!(
                document.text_range(offset..offset + 1),
                Some(&text[byte..byte + character.len_utf8()])
            );
            for continuation in byte + 1..byte + character.len_utf8() {
                assert_eq!(document.byte_to_char(continuation), None);
            }
        }
        assert_eq!(document.char_to_byte(info.chars), Some(text.len()));
        assert_eq!(document.byte_to_char(text.len()), Some(info.chars));
        assert_eq!(document.char_to_byte(info.chars + 1), None);
        for offset in [0, 255, 256, 4094, 4096, info.chars] {
            if offset <= info.chars {
                let locator = document.locator(offset).unwrap();
                assert_eq!(document.char_offset_of_locator(&locator), Some(offset));
                assert_eq!(
                    locator.perch.txt_byte_offset,
                    document.char_to_byte(offset).map(|byte| byte as u64)
                );
            }
        }
        if sample.id == "long-line" {
            assert!(info.chars > 20_000);
            assert_eq!(document.line_count(), 1);
            assert_eq!(document.line_text(0), Some(text.as_str()));
        }
    }
}

#[test]
fn epub2_ncx_and_epub3_nav_match_metadata_text_and_chapter_spans() {
    for sample in manifest()
        .samples
        .into_iter()
        .filter(|sample| sample.format == Format::Epub)
    {
        let bytes = content(&sample);
        let mut archive = epub::EpubArchive::from_bytes(bytes.clone()).unwrap();
        let path = archive.opf_path().unwrap();
        let opf = archive.read_to_string(&path).unwrap();
        let package = epub::Package::parse(&path, &opf).unwrap();
        assert_eq!(package.version, sample.epub_version);
        assert_eq!(package.metadata.title, sample.title);
        assert_eq!(package.spine_items().len(), 2);
        // Exercise separate directory paths rather than NAV with NCX fallback.
        assert_eq!(archive.contains("OEBPS/toc.ncx"), sample.id == "epub2");
        assert_eq!(archive.contains("OEBPS/nav.xhtml"), sample.id == "epub3");
        let metadata = jobs::refine_epub(&bytes, jobs::ExtractOptions::default()).unwrap();
        assert_eq!(metadata.title, sample.title);
        assert_eq!(metadata.author.as_deref(), Some("PerchCore"));
        assert_eq!(metadata.toc, sample.toc);
        let book = jobs::extract_epub_text(&bytes, jobs::ExtractOptions::default()).unwrap();
        assert_eq!(book.text, expected(sample.indexed.as_ref().unwrap()));
        assert!(!book.truncated);
        assert_eq!(book.chapters.len(), sample.chapters.len());
        for (span, expected) in book.chapters.iter().zip(&sample.chapters) {
            assert_eq!(span.href, expected.href);
            assert_eq!(span.start_char, expected.start_char);
            assert_eq!(span.end_char, expected.end_char);
        }
        let crossing = &sample.queries[0].ranges[0];
        assert_ne!(
            book.span_at(crossing[0]).unwrap().href,
            book.span_at(crossing[1] - 1).unwrap().href
        );
    }
}

#[test]
fn golden_phrase_ranges_survive_index_serialization() {
    for sample in manifest()
        .samples
        .into_iter()
        .filter(|sample| sample.indexed.is_some())
    {
        let book = jobs::extract_text(
            &content(&sample),
            sample.format,
            jobs::ExtractOptions::default(),
        )
        .unwrap();
        assert_eq!(
            book.text,
            expected(sample.indexed.as_ref().unwrap()),
            "{} extracted text",
            sample.id
        );
        assert!(!book.truncated);
        let index = jobs::index_book(&book, 4096);
        let reopened = Index::from_bytes(&index.to_bytes()).unwrap();
        for query in &sample.queries {
            assert_eq!(index.chunk_chars(), query.chunk_chars);
            let hits = index.search(&query.query);
            let ranges: Vec<_> = hits
                .iter()
                .map(|hit| [hit.char_start, hit.char_end])
                .collect();
            assert_eq!(
                ranges, query.ranges,
                "{} query {:?}",
                sample.id, query.query
            );
            assert_eq!(
                reopened.search(&query.query),
                hits,
                "{} reopened",
                sample.id
            );
            for hit in &hits {
                let locator = jobs::locate_hit(&book, hit).unwrap();
                match sample.format {
                    Format::Epub => {
                        assert_eq!(locator.href, book.span_at(hit.char_start).unwrap().href)
                    }
                    Format::Txt => assert_eq!(locator.perch.txt_offset, Some(hit.char_start)),
                    Format::Pdf => unreachable!(),
                }
            }
        }
        if sample.id == "cross-shard" {
            assert_eq!(sample.queries[0].ranges, vec![[4094, 4098]]);
        }
    }
}

fn assert_outline(actual: &[pdf::OutlineItem], expected: &[Outline]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.page_index, Some(expected.page_index));
        assert_outline(&actual.children, &expected.children);
    }
}

#[test]
fn pdf_structure_preserves_unicode_outline_and_zero_based_pages() {
    let sample = manifest()
        .samples
        .into_iter()
        .find(|sample| sample.format == Format::Pdf)
        .unwrap();
    let info = pdf::analyze(&content(&sample)).unwrap();
    assert_eq!(info.version, "1.7");
    assert_eq!(info.title, sample.title);
    assert_eq!(Some(info.page_count), sample.page_count);
    assert_eq!(info.author.as_deref(), Some("PerchCore"));
    assert_eq!(info.outline_count(), 3);
    assert_outline(&info.outline, &sample.outline);
    for page in 0..info.page_count {
        let locator = info.locator_for(page).unwrap();
        assert_eq!(pdf::page_index_of(&locator), Some(page));
    }
    assert!(info.locator_for(info.page_count).is_none());
}
