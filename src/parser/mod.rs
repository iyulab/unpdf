//! PDF parsing module.

mod options;
mod pdf_parser;

pub use options::{ErrorMode, ExtractMode, ParseOptions};
pub use pdf_parser::PdfParser;
