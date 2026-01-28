//! PDF parsing module.

mod layout;
mod options;
mod pdf_parser;

pub use layout::{BlockType, Column, FontStatistics, LayoutAnalyzer, TextBlock, TextLine, TextSpan};
pub use options::{ErrorMode, ExtractMode, ParseOptions};
pub use pdf_parser::PdfParser;
