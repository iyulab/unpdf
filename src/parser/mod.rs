//! PDF parsing module.

pub mod backend;
pub mod bidi;
pub mod cmap_table;
pub(crate) mod encoding;
pub(crate) mod font;
mod layout;
mod options;
mod pdf_parser;
pub mod raw;
pub mod stream;
pub mod xycut;
mod table_detector;

pub use layout::{
    BlockType, Column, FontStatistics, LayoutAnalyzer, TextBlock, TextLine, TextSpan,
};
pub use options::{ErrorMode, ExtractMode, ParseOptions};
pub use pdf_parser::PdfParser;
pub use table_detector::{DetectedTable, TableDetector, TableDetectorConfig, TableRowData};
