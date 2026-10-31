//! Portable ebook processing without a UI or rendering dependency.
//!
//! This crate re-exports the independently usable PerchCore modules.
//! Enable `sync` or `pagecurl` only if the corresponding algorithms are needed.
//!
//! ```
//! use perch_core::{index::Index, txt::TxtDocument};
//!
//! let document = TxtDocument::from_bytes("中文文字与书页".as_bytes());
//! let index = Index::build(document.text(), 4096);
//! assert_eq!(index.search("文字").len(), 1);
//! ```

pub use perch_epub as epub;
pub use perch_import as import;
pub use perch_index as index;
pub use perch_jobs as jobs;
pub use perch_model as model;
pub use perch_pdf as pdf;
pub use perch_pipeline as pipeline;
pub use perch_txt as txt;

#[cfg(feature = "sync")]
pub use perch_sync as sync;

#[cfg(feature = "pagecurl")]
pub use perch_pagecurl as pagecurl;
