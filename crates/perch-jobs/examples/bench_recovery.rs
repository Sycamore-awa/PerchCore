//! STORE-01 recovery benchmark worker.
//!
//! `prepare` creates one real Ready artifact through the public pipeline. `measure`
//! opens an existing artifact library once and reports only the `PathStore::new`
//! duration plus process RSS. The Python runner starts a fresh process for every
//! measurement so allocator and in-process caches do not leak across samples.

use std::path::Path;
use std::time::Instant;

use perch_jobs::{ExtractOptions, PathStore, Pipeline, StepOutcome};
use perch_model::{BookId, Format, ImportState};
use perch_pipeline::{Gate, Policy, Priority};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("prepare") => prepare(
            required(&mut args, "source")?,
            required(&mut args, "format")?,
            required(&mut args, "output directory")?,
            required(&mut args, "book UUID")?,
        ),
        Some("measure") => measure(
            required(&mut args, "artifact directory")?,
            parse_usize(required(&mut args, "expected recovered")?)?,
            parse_usize(required(&mut args, "expected ready")?)?,
            parse_usize(required(&mut args, "expected indexing")?)?,
        ),
        _ => Err("usage: bench_recovery prepare <source> <txt|epub> <out> <uuid> | measure <out> <recovered> <ready> <indexing>".into()),
    }
}

fn required(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("missing {name}"))
}

fn parse_usize(value: String) -> Result<usize, Box<dyn std::error::Error>> {
    Ok(value.parse()?)
}

fn prepare(
    source: String,
    format: String,
    output: String,
    book: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let format = match format.as_str() {
        "txt" => Format::Txt,
        "epub" => Format::Epub,
        _ => return Err(format!("unsupported format: {format}").into()),
    };
    let book = BookId::parse(&book)?;
    let store = PathStore::new(&output)?;
    let mut pipeline = Pipeline::new(
        store,
        Policy::default(),
        ExtractOptions::default(),
        perch_index::DEFAULT_CHUNK_CHARS,
    )?;
    pipeline.store_mut().register(book, source, format);
    pipeline.enqueue(book, ImportState::Registered, Priority::Normal);
    if !matches!(
        pipeline.step(0, Gate::foreground()),
        StepOutcome::Refined { .. }
    ) {
        return Err("preparation refine step did not succeed".into());
    }
    if !matches!(
        pipeline.step(0, Gate::foreground()),
        StepOutcome::Indexed { .. }
    ) {
        return Err("preparation index step did not succeed".into());
    }
    let directory = pipeline.store().book_dir(book);
    let sizes = artifact_sizes(&directory)?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "book": book,
            "directory": directory,
            "state_bytes": sizes[0],
            "refinement_bytes": sizes[1],
            "index_bytes": sizes[2],
            "text_bytes": sizes[3],
            "total_bytes": sizes.iter().sum::<u64>(),
        }))?
    );
    Ok(())
}

fn artifact_sizes(directory: &Path) -> Result<[u64; 4], std::io::Error> {
    Ok([
        std::fs::metadata(directory.join("state.json"))?.len(),
        std::fs::metadata(directory.join("refinement.json"))?.len(),
        std::fs::metadata(directory.join("index.bin"))?.len(),
        std::fs::metadata(directory.join("text.json"))?.len(),
    ])
}

fn measure(
    output: String,
    expected_recovered: usize,
    expected_ready: usize,
    expected_indexing: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let store = PathStore::new(output)?;
    let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);

    let recovered = store.recovered();
    let ready = recovered
        .iter()
        .filter(|item| item.state == ImportState::Ready)
        .count();
    let indexing = recovered
        .iter()
        .filter(|item| item.state == ImportState::Indexing)
        .count();
    let registered = recovered
        .iter()
        .filter(|item| item.state == ImportState::Registered)
        .count();
    let failed = recovered
        .iter()
        .filter(|item| item.state == ImportState::Failed)
        .count();
    let corrupt_index_reasons = recovered
        .iter()
        .filter(|item| {
            item.reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("STORE_INDEX_INVALID"))
        })
        .count();

    if recovered.len() != expected_recovered
        || ready != expected_ready
        || indexing != expected_indexing
        || registered != 0
        || failed != 0
        || corrupt_index_reasons != expected_indexing
    {
        return Err(format!(
            "unexpected recovery: recovered={} ready={ready} indexing={indexing} registered={registered} failed={failed} corrupt_index_reasons={corrupt_index_reasons}",
            recovered.len()
        )
        .into());
    }

    let status = process_status();
    println!(
        "{}",
        serde_json::to_string(&json!({
            "duration_ns": duration_ns,
            "recovered": recovered.len(),
            "ready": ready,
            "indexing": indexing,
            "registered": registered,
            "failed": failed,
            "corrupt_index_reasons": corrupt_index_reasons,
            "rss_bytes": status.0,
            "peak_rss_bytes": status.1,
        }))?
    );
    Ok(())
}

fn process_status() -> (Option<u64>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return (None, None);
        };
        let value = |key: &str| {
            status.lines().find_map(|line| {
                let rest = line.strip_prefix(key)?;
                let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
                Some(kib.saturating_mul(1024))
            })
        };
        (value("VmRSS:"), value("VmHWM:"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        (None, None)
    }
}
