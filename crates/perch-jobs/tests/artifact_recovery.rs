#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use perch_jobs::{
    ExtractOptions, PathStore, Pipeline, SOURCE_MAP_VERSION, mapped_book_text_from_slice,
};
use perch_model::{BookId, Format, ImportState};
use perch_pipeline::{Gate, Policy, Priority};

const RECOVERY_REFINEMENT_INVALID: &str = "STORE_REFINEMENT_INVALID";
const RECOVERY_INDEX_INVALID: &str = "STORE_INDEX_INVALID";

#[derive(Clone, Copy, Debug)]
enum Damage {
    RefinementMissing,
    RefinementCorrupt,
    IndexMissing,
    IndexCorrupt,
    IndexFutureVersion,
    TextMissing,
    TextCorrupt,
    TextFutureVersion,
    SourceMapMissing,
    SourceMapNull,
    SourceMapFutureVersion,
    SourceMapCorrupt,
}

impl Damage {
    const fn expected_state(self) -> ImportState {
        match self {
            Self::RefinementMissing | Self::RefinementCorrupt => ImportState::Registered,
            Self::IndexMissing
            | Self::IndexCorrupt
            | Self::IndexFutureVersion
            | Self::TextMissing
            | Self::TextCorrupt
            | Self::TextFutureVersion
            | Self::SourceMapMissing
            | Self::SourceMapNull
            | Self::SourceMapFutureVersion
            | Self::SourceMapCorrupt => ImportState::Indexing,
        }
    }

    const fn expected_reason(self) -> &'static str {
        match self {
            Self::RefinementMissing | Self::RefinementCorrupt => RECOVERY_REFINEMENT_INVALID,
            _ => RECOVERY_INDEX_INVALID,
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "perch-artifact-recovery-{}-{name}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_book(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn build_ready_txt(dir: &Path, book: BookId, path: &Path) {
    let mut pipeline = Pipeline::new(
        PathStore::new(dir).unwrap(),
        Policy::default(),
        ExtractOptions::default(),
        64,
    )
    .unwrap();
    pipeline.store_mut().register(book, path, Format::Txt);
    pipeline.enqueue(book, ImportState::Registered, Priority::Normal);
    let summary = pipeline.run_until_idle(1_000, Gate::foreground(), 8);
    assert_eq!(summary.refined, 1);
    assert_eq!(summary.indexed, 1);
}

fn mutate_json(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    mutate(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn artifact_damage_matrix_is_isolated_and_fully_rebuildable() {
    let dir = temp_dir("matrix");
    let healthy = BookId::new();
    let healthy_path = write_book(&dir, "healthy.txt", "健康书保持可用".as_bytes());
    build_ready_txt(&dir, healthy, &healthy_path);

    let damages = [
        Damage::RefinementMissing,
        Damage::RefinementCorrupt,
        Damage::IndexMissing,
        Damage::IndexCorrupt,
        Damage::IndexFutureVersion,
        Damage::TextMissing,
        Damage::TextCorrupt,
        Damage::TextFutureVersion,
        Damage::SourceMapMissing,
        Damage::SourceMapNull,
        Damage::SourceMapFutureVersion,
        Damage::SourceMapCorrupt,
    ];
    let mut damaged_books = Vec::new();

    for (index, damage) in damages.into_iter().enumerate() {
        let book = BookId::new();
        let path = write_book(
            &dir,
            &format!("damaged-{index}.txt"),
            format!("恢复矩阵文本 {index}").as_bytes(),
        );
        build_ready_txt(&dir, book, &path);
        let book_dir = dir.join(book.to_string());
        let refinement_path = book_dir.join("refinement.json");
        let index_path = book_dir.join("index.bin");
        let text_path = book_dir.join("text.json");

        match damage {
            Damage::RefinementMissing => std::fs::remove_file(refinement_path).unwrap(),
            Damage::RefinementCorrupt => {
                std::fs::write(refinement_path, b"{not-json").unwrap();
            }
            Damage::IndexMissing => std::fs::remove_file(index_path).unwrap(),
            Damage::IndexCorrupt => std::fs::write(index_path, b"not-an-index").unwrap(),
            Damage::IndexFutureVersion => {
                let mut bytes = std::fs::read(&index_path).unwrap();
                bytes[8] = u8::MAX;
                std::fs::write(index_path, bytes).unwrap();
            }
            Damage::TextMissing => std::fs::remove_file(text_path).unwrap(),
            Damage::TextCorrupt => std::fs::write(text_path, b"{not-json").unwrap(),
            Damage::TextFutureVersion => mutate_json(&text_path, |value| {
                value["version"] = serde_json::json!(u32::MAX);
            }),
            Damage::SourceMapMissing => mutate_json(&text_path, |value| {
                value.as_object_mut().unwrap().remove("sourceMap");
            }),
            Damage::SourceMapNull => mutate_json(&text_path, |value| {
                value["sourceMap"] = serde_json::Value::Null;
            }),
            Damage::SourceMapFutureVersion => mutate_json(&text_path, |value| {
                value["sourceMap"]["version"] = serde_json::json!(u32::MAX);
            }),
            Damage::SourceMapCorrupt => mutate_json(&text_path, |value| {
                value["sourceMap"] = serde_json::json!({ "version": SOURCE_MAP_VERSION });
            }),
        }
        damaged_books.push((book, path, damage));
    }

    let reopened = PathStore::new(&dir).unwrap();
    assert_eq!(reopened.state(healthy).unwrap().state, ImportState::Ready);
    for (book, _, damage) in &damaged_books {
        let state = reopened.state(*book).unwrap();
        assert_eq!(state.state, damage.expected_state(), "{damage:?}");
        assert!(
            state
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(damage.expected_reason())),
            "{damage:?}: {:?}",
            state.reason
        );
    }

    let recovered = reopened.recovered().to_vec();
    let mut pipeline =
        Pipeline::new(reopened, Policy::default(), ExtractOptions::default(), 64).unwrap();
    for (book, path, _) in &damaged_books {
        pipeline.store_mut().register(*book, path, Format::Txt);
        let record = recovered
            .iter()
            .find(|record| record.book == *book)
            .unwrap();
        pipeline.resume(record.book, record.state, Priority::Normal, record.attempts);
    }
    let summary = pipeline.run_until_idle(2_000, Gate::foreground(), 64);
    assert_eq!(summary.refined, 2);
    assert_eq!(summary.indexed, u32::try_from(damaged_books.len()).unwrap());
    assert_eq!(summary.failed, 0);
    assert!(pipeline.is_idle());

    for (book, _, damage) in &damaged_books {
        assert_eq!(
            pipeline.store().state(*book).unwrap().state,
            ImportState::Ready,
            "{damage:?}"
        );
        let book_dir = pipeline.store().book_dir(*book);
        let index = perch_index::Index::load_from(book_dir.join("index.bin")).unwrap();
        assert_eq!(index.search("恢复矩阵").len(), 1, "{damage:?}");
        let text = mapped_book_text_from_slice(&std::fs::read(book_dir.join("text.json")).unwrap())
            .unwrap();
        assert!(text.source_map().is_ok(), "{damage:?}");
    }

    std::fs::remove_dir_all(&dir).ok();
}
