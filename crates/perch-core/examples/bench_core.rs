//! BASE-02 worker. The Python runner owns fixture selection, process isolation and reports.

use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File},
    hint::black_box,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    time::Instant,
};

use perch_core::{
    epub::{EpubArchive, Package},
    import::{Candidate, Fingerprint, SAMPLE_BYTES, plan_import},
    index::Index,
    jobs::{self, BookStore, BookText, ExtractOptions, PathStore, Pipeline},
    model::{BookId, Format, ImportState},
    pdf,
    pipeline::{Gate, Policy, Priority},
    txt::{self, TxtDocument},
};
use serde::Deserialize;
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Config {
    stage: String,
    path: PathBuf,
    format: Format,
    iterations: usize,
    warmups: usize,
    scratch: PathBuf,
    decoded_path: Option<PathBuf>,
    indexed_path: Option<PathBuf>,
    query: String,
    expected_ranges: Vec<[u64; 2]>,
    max_chars: u64,
    chunk_chars: u64,
    slice_start: u64,
    slice_chars: u64,
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}

fn measure<T>(
    config: &Config,
    mut operation: impl FnMut() -> Result<T>,
    mut validate: impl FnMut(&T) -> Result<Value>,
) -> Result<Value> {
    let mut durations = Vec::with_capacity(config.iterations);
    let mut stats = Value::Null;
    for iteration in 0..config.warmups + config.iterations {
        let start = Instant::now();
        let output = operation()?;
        black_box(&output);
        let elapsed = u64::try_from(start.elapsed().as_nanos())?;
        // Validation and destruction of the output are outside the timed interval.
        stats = validate(&output)?;
        if iteration >= config.warmups {
            durations.push(elapsed);
        }
    }
    Ok(json!({"durations_ns": durations, "stats": stats}))
}

fn fingerprint(path: &PathBuf) -> Result<Fingerprint> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let count = usize::try_from(size.min(SAMPLE_BYTES as u64))?;
    let mut head = vec![0; count];
    let mut tail = vec![0; count];
    file.read_exact(&mut head)?;
    file.seek(SeekFrom::Start(size.saturating_sub(count as u64)))?;
    file.read_exact(&mut tail)?;
    Ok(Fingerprint::compute(size, &head, &tail))
}

fn validate_book(book: &BookText, expected: &str) -> Result<Value> {
    require(
        if book.truncated {
            expected.starts_with(&book.text)
        } else {
            book.text == expected
        },
        "extracted text differs from corpus",
    )?;
    Ok(
        json!({"chars": book.char_count(), "chapters": book.chapters.len(), "truncated": book.truncated}),
    )
}

fn save(index: &Index, book: &BookText, config: &Config) -> Result<()> {
    index.save_to(config.scratch.join("index.bin"))?;
    fs::write(config.scratch.join("text.tmp"), serde_json::to_vec(book)?)?;
    fs::rename(
        config.scratch.join("text.tmp"),
        config.scratch.join("text.json"),
    )?;
    Ok(())
}

fn run(config: &Config) -> Result<Value> {
    require(config.iterations > 0, "iterations must be positive")?;
    require(config.slice_chars > 0, "slice_chars must be positive")?;
    fs::create_dir_all(&config.scratch)?;
    let options = ExtractOptions {
        max_chars: config.max_chars,
    };

    if config.stage == "register" {
        let empty = HashSet::new();
        return measure(
            config,
            || {
                let candidate = Candidate {
                    path: config.path.to_string_lossy().into_owned(),
                    fingerprint: fingerprint(&config.path)?,
                    size: fs::metadata(&config.path)?.len(),
                };
                Ok(plan_import(&[candidate], &empty))
            },
            |plan| {
                require(plan.register_count() == 1, "registration failed")?;
                Ok(json!({"registered": 1}))
            },
        );
    }

    if config.stage == "open" {
        return match config.format {
            Format::Txt => {
                let expected = fs::read_to_string(
                    config.decoded_path.as_ref().ok_or("missing decoded text")?,
                )?;
                measure(
                    config,
                    || Ok(TxtDocument::open(&config.path)?),
                    |document| {
                        require(document.text() == expected, "TXT open mismatch")?;
                        Ok(
                            json!({"chars": document.char_count(), "encoding": document.meta().encoding}),
                        )
                    },
                )
            }
            Format::Epub => measure(
                config,
                || {
                    let mut archive = EpubArchive::open(&config.path)?;
                    let path = archive.opf_path()?;
                    let opf = archive.read_to_string(&path)?;
                    Ok(Package::parse(&path, &opf)?)
                },
                |package| {
                    require(!package.spine_items().is_empty(), "EPUB has no spine")?;
                    Ok(json!({"chapters": package.spine_items().len()}))
                },
            ),
            Format::Pdf => measure(
                config,
                || Ok(pdf::analyze(&fs::read(&config.path)?)?),
                |info| {
                    require(
                        info.page_count == 3 && info.outline_count() == 3,
                        "PDF structure mismatch",
                    )?;
                    Ok(json!({"pages": info.page_count, "outline": info.outline_count()}))
                },
            ),
        };
    }

    let bytes = fs::read(&config.path)?;
    if config.stage == "decode" {
        let expected =
            fs::read_to_string(config.decoded_path.as_ref().ok_or("missing decoded text")?)?;
        return measure(
            config,
            || Ok(txt::decode(&bytes)),
            |decoded| {
                require(decoded.text() == expected, "TXT decode mismatch")?;
                Ok(
                    json!({"encoding": decoded.encoding(), "had_errors": decoded.had_decode_errors()}),
                )
            },
        );
    }

    if config.stage == "slice" {
        let document = TxtDocument::from_bytes(&bytes);
        require(
            config.slice_start <= document.char_count(),
            "slice starts past EOF",
        )?;
        let end = config
            .slice_start
            .saturating_add(config.slice_chars)
            .min(document.char_count());
        let expected_slice: String = document
            .text()
            .chars()
            .skip(config.slice_start as usize)
            .take((end - config.slice_start) as usize)
            .collect();
        return measure(
            config,
            || {
                Ok(document
                    .text_range(config.slice_start..end)
                    .ok_or("slice out of range")?)
            },
            |text| {
                require(*text == expected_slice, "slice mismatch")?;
                Ok(json!({"chars": end - config.slice_start, "utf8_bytes": text.len()}))
            },
        );
    }

    let expected = fs::read_to_string(config.indexed_path.as_ref().ok_or("missing indexed text")?)?;
    if config.stage == "refine" {
        return measure(
            config,
            || Ok(jobs::refine(&bytes, config.format, options)?),
            |refinement| {
                require(refinement.char_count > 0, "empty refinement")?;
                Ok(
                    json!({"visible_chars": refinement.char_count, "chapters": refinement.spine_count}),
                )
            },
        );
    }
    if config.stage == "extract" {
        return measure(
            config,
            || Ok(jobs::extract_text(&bytes, config.format, options)?),
            |book| validate_book(book, &expected),
        );
    }

    if config.stage == "pipeline" {
        let id = BookId::parse("00000000-0000-4000-8000-000000000001")?;
        let mut iteration = 0;
        return measure(
            config,
            || {
                let directory = config.scratch.join(format!("iteration-{iteration}"));
                iteration += 1;
                let mut store = PathStore::new(&directory)?;
                store.register(id, &config.path, config.format);
                store.set_state(id, ImportState::Registered)?;
                let mut pipeline =
                    Pipeline::new(store, Policy::default(), options, config.chunk_chars)?;
                pipeline.enqueue(id, ImportState::Registered, Priority::Normal);
                let summary = pipeline.run_until_idle(0, Gate::foreground(), 10);
                let state = pipeline
                    .store()
                    .state(id)
                    .ok_or("pipeline lost state")?
                    .state;
                Ok((summary, state, pipeline.store().book_dir(id)))
            },
            |(summary, state, directory)| {
                require(
                    summary.refined == 1
                        && summary.indexed == 1
                        && summary.failed == 0
                        && *state == ImportState::Ready,
                    "pipeline did not finish successfully",
                )?;
                let book: BookText =
                    serde_json::from_slice(&fs::read(directory.join("text.json"))?)?;
                let index = Index::load_from(directory.join("index.bin"))?;
                require(index.text() == book.text, "pipeline index/text mismatch")?;
                validate_book(&book, &expected)
            },
        );
    }

    let book = jobs::extract_text(&bytes, config.format, options)?;
    validate_book(&book, &expected)?;
    if config.stage == "index" {
        return measure(
            config,
            || Ok(jobs::index_book(&book, config.chunk_chars)),
            |index| {
                require(index.text() == book.text, "index changed source text")?;
                let stats = index.stats();
                Ok(
                    json!({"chars": stats.chars, "chunks": stats.chunks, "tokens": stats.tokens, "postings": stats.postings, "truncated": book.truncated}),
                )
            },
        );
    }
    let index = jobs::index_book(&book, config.chunk_chars);
    match config.stage.as_str() {
        "query.hit" | "query.miss" => measure(
            config,
            || Ok(index.search(&config.query)),
            |hits| {
                let ranges: Vec<_> = hits
                    .iter()
                    .map(|hit| [hit.char_start, hit.char_end])
                    .collect();
                let expected_ranges: Vec<_> = config
                    .expected_ranges
                    .iter()
                    .copied()
                    .filter(|range| range[1] <= book.char_count())
                    .collect();
                require(ranges == expected_ranges, "query ranges differ from corpus")?;
                Ok(json!({"hits": hits.len(), "truncated": book.truncated}))
            },
        ),
        "save" => measure(
            config,
            || save(&index, &book, config),
            |()| {
                let index_bytes = fs::metadata(config.scratch.join("index.bin"))?.len();
                let text_bytes = fs::metadata(config.scratch.join("text.json"))?.len();
                Ok(
                    json!({"index_bytes": index_bytes, "text_bytes": text_bytes, "truncated": book.truncated}),
                )
            },
        ),
        "reopen" => {
            save(&index, &book, config)?;
            measure(
                config,
                || {
                    let index = Index::load_from(config.scratch.join("index.bin"))?;
                    let book: BookText =
                        serde_json::from_slice(&fs::read(config.scratch.join("text.json"))?)?;
                    Ok((index, book))
                },
                |(index, book)| {
                    require(index.text() == book.text, "reopened index/text mismatch")?;
                    let mut stats = validate_book(book, &expected)?;
                    stats["artifact_bytes"] = json!(
                        fs::metadata(config.scratch.join("index.bin"))?.len()
                            + fs::metadata(config.scratch.join("text.json"))?.len()
                    );
                    Ok(stats)
                },
            )
        }
        stage => Err(io::Error::other(format!("unknown stage: {stage}")).into()),
    }
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: bench_core <case.json>")?;
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    let mut result = run(&config)?;
    // Read the post-exec address space. A parent's wait/getrusage can include
    // the forked Python image before exec, inflating small Rust measurements.
    let peak = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
                .map(|kib| kib * 1024)
        });
    result["peak_rss"] = match peak {
        Some(bytes) => {
            json!({"bytes": bytes, "method": "/proc/self/status VmHWM", "scope": "post-exec process: setup, warmups, operations and validation"})
        }
        None => {
            json!({"bytes": null, "reason": "native peak RSS implemented for Linux /proc only"})
        }
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
