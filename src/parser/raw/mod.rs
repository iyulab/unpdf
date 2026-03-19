//! Custom PDF parser — lightweight, purpose-built for text extraction.

pub mod tokenizer;
pub mod xref;
pub mod document;
pub mod stream;
pub mod content;

pub use document::RawDocument;
pub use tokenizer::{PdfObject, PdfDict, PdfStream};
