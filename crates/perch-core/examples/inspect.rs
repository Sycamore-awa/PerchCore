//! Minimal standalone client: inspect EPUB/TXT/PDF and search EPUB/TXT text.

use std::{error::Error, io, path::Path};

use perch_core::{jobs, model::Format, pdf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run -p perch-core --example inspect -- <book.epub|txt|pdf> [query]",
        )
        .into());
    };
    let query = args.next();
    let bytes = std::fs::read(Path::new(&path))?;
    let format = jobs::sniff_format(&bytes);

    if format == Format::Pdf {
        let info = pdf::analyze(&bytes)?;
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    let options = jobs::ExtractOptions::default();
    let metadata = jobs::refine(&bytes, format, options)?;
    println!("{}", serde_json::to_string_pretty(&metadata)?);

    if let Some(query) = query {
        let text = jobs::extract_text(&bytes, format, options)?;
        let index = jobs::index_book(&text, 4096);
        for hit in index.search(&query) {
            println!("{}..{}: {}", hit.char_start, hit.char_end, hit.snippet);
        }
        if text.truncated {
            eprintln!("Search covers only the extracted prefix (max_chars limit).");
        }
    }
    Ok(())
}
