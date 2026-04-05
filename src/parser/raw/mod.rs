//! Custom PDF parser — lightweight, purpose-built for text extraction.

pub mod content;
pub mod crypt;
pub mod document;
pub mod stream;
pub mod tokenizer;
pub mod xref;

pub use document::RawDocument;
pub use tokenizer::{PdfDict, PdfObject, PdfStream};
