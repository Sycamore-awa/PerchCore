//! Frozen compatibility fixtures for API-02.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use perch_core::{
    index::Index,
    jobs::{BOOK_TEXT_VERSION, book_text_from_slice, mapped_book_text_from_slice},
    model::{Format, LOCATOR_VERSION, Locator},
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data/compatibility")
}

fn decode_hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    assert_eq!(
        digits.len() % 2,
        0,
        "hex fixture must contain complete bytes"
    );
    (0..digits.len())
        .step_by(2)
        .map(|index| {
            let high = (digits[index] as char).to_digit(16).unwrap() as u8;
            let low = (digits[index + 1] as char).to_digit(16).unwrap() as u8;
            (high << 4) | low
        })
        .collect()
}

#[test]
fn locator_v1_fixture_still_parses_and_validates() {
    let bytes = std::fs::read(fixtures().join("locator-v1.json")).unwrap();
    let locator: Locator = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(locator.perch.v, LOCATOR_VERSION);
    assert_eq!(locator.format(), Some(Format::Epub));
    assert_eq!(locator.locations.partial_cfi.as_deref(), Some("/4/2"));
    locator.validate().unwrap();
}

#[test]
fn text_v0_fixture_keeps_the_legacy_epub_default() {
    let bytes = std::fs::read(fixtures().join("text-v0.json")).unwrap();
    let text = book_text_from_slice(&bytes).unwrap();
    assert_eq!(text.format, Format::Epub);
    assert_eq!(text.text, "a");
    assert_eq!(text.chapters.len(), 1);
    assert_eq!(text.chapters[0].href, "/OPS/chapter.xhtml");
    assert_eq!(text.chapters[0].start_char, 0);
    assert_eq!(text.chapters[0].end_char, 1);
    assert!(!text.truncated);
    let mapped = mapped_book_text_from_slice(&bytes).unwrap();
    assert_eq!(mapped.book_text(), &text);
    assert_eq!(
        mapped.locate(0, 1).unwrap_err().code(),
        "BOOK_SOURCE_MAP_MISSING"
    );
}

#[test]
fn future_text_version_returns_the_stable_version_error() {
    let bytes = std::fs::read(fixtures().join("text-v0.json")).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["version"] = serde_json::json!(BOOK_TEXT_VERSION + 1);
    assert_eq!(
        book_text_from_slice(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code(),
        "BOOK_TEXT_UNSUPPORTED_VERSION"
    );
}

#[test]
fn index_v1_fixture_still_loads_and_searches() {
    let hex = std::fs::read_to_string(fixtures().join("index-v1.hex")).unwrap();
    let bytes = decode_hex(&hex);
    let index = Index::from_bytes(&bytes).unwrap();
    assert_eq!(index.text(), "a");
    assert_eq!(index.chunk_chars(), 4096);
    let hits = index.search("a");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].char_start, hits[0].char_end), (0, 1));
}

#[test]
fn future_index_fixture_returns_the_stable_version_error() {
    let hex = std::fs::read_to_string(fixtures().join("index-v1.hex")).unwrap();
    let mut bytes = decode_hex(&hex);
    bytes[8] = 2;
    assert_eq!(
        Index::from_bytes(&bytes).unwrap_err().code(),
        "INDEX_UNSUPPORTED_VERSION"
    );
}
