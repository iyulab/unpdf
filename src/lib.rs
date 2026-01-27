//! # unpdf
//!
//! High-performance PDF content extraction library for Rust.
//!
//! This library extracts content from PDF documents and converts it to
//! structured formats like Markdown, plain text, and JSON.
//!
//! ## Quick Start
//!
//! ```no_run
//! use unpdf::{parse_file, render};
//!
//! fn main() -> unpdf::Result<()> {
//!     // Parse a PDF file
//!     let doc = parse_file("document.pdf")?;
//!
//!     // Convert to Markdown
//!     let options = render::RenderOptions::default();
//!     let markdown = render::to_markdown(&doc, &options)?;
//!     println!("{}", markdown);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - **Multiple output formats**: Markdown, plain text, JSON
//! - **Structure preservation**: Headings, paragraphs, tables, lists
//! - **Asset extraction**: Images and embedded resources
//! - **CJK support**: Korean, Chinese, Japanese text handling
//! - **Parallel processing**: Uses Rayon for multi-page documents
//! - **Cleanup pipeline**: Text normalization for LLM training data

pub mod convert;
pub mod detect;
pub mod error;
pub mod model;
pub mod parser;
pub mod render;

#[cfg(feature = "ffi")]
pub mod ffi;

// Re-export commonly used types
pub use convert::{
    ConvertOptions, ConvertResult, ConverterRegistry, DocumentConverter, OutputFormat,
};
pub use detect::{detect_format_from_bytes, detect_format_from_path, is_pdf, PdfFormat};
pub use error::{Error, Result};
pub use model::{
    Alignment, Block, Document, InlineContent, ListInfo, Metadata, Outline, Page, Paragraph,
    ParagraphStyle, Resource, ResourceType, Table, TableCell, TableRow, TextRun, TextStyle,
};
pub use parser::{ParseOptions, PdfParser};
pub use render::{
    CleanupOptions, CleanupPreset, HeadingConfig, JsonFormat, PageSelection, RenderOptions,
    TableFallback,
};

use std::io::Read;
use std::path::Path;

/// Parse a PDF file and return a structured document.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
///
/// # Returns
///
/// A `Result` containing the parsed `Document` or an error.
///
/// # Example
///
/// ```no_run
/// use unpdf::parse_file;
///
/// let doc = parse_file("document.pdf").unwrap();
/// println!("Pages: {}", doc.page_count());
/// ```
pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<Document> {
    let parser = PdfParser::open(path)?;
    parser.parse()
}

/// Parse a PDF file with custom options.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
/// * `options` - Parsing options
///
/// # Example
///
/// ```no_run
/// use unpdf::{parse_file_with_options, ParseOptions};
///
/// let options = ParseOptions::new()
///     .lenient()
///     .text_only();
/// let doc = parse_file_with_options("document.pdf", options).unwrap();
/// ```
pub fn parse_file_with_options<P: AsRef<Path>>(path: P, options: ParseOptions) -> Result<Document> {
    let parser = PdfParser::open_with_options(path, options)?;
    parser.parse()
}

/// Parse a PDF from bytes.
///
/// # Arguments
///
/// * `data` - PDF file content as bytes
///
/// # Example
///
/// ```no_run
/// use unpdf::parse_bytes;
///
/// let data = std::fs::read("document.pdf").unwrap();
/// let doc = parse_bytes(&data).unwrap();
/// ```
pub fn parse_bytes(data: &[u8]) -> Result<Document> {
    let parser = PdfParser::from_bytes(data)?;
    parser.parse()
}

/// Parse a PDF from bytes with custom options.
pub fn parse_bytes_with_options(data: &[u8], options: ParseOptions) -> Result<Document> {
    let parser = PdfParser::from_bytes_with_options(data, options)?;
    parser.parse()
}

/// Parse a PDF from a reader.
///
/// # Arguments
///
/// * `reader` - Any type implementing `Read`
///
/// # Example
///
/// ```no_run
/// use unpdf::parse_reader;
/// use std::fs::File;
///
/// let file = File::open("document.pdf").unwrap();
/// let doc = parse_reader(file).unwrap();
/// ```
pub fn parse_reader<R: Read>(reader: R) -> Result<Document> {
    let parser = PdfParser::from_reader(reader)?;
    parser.parse()
}

/// Parse a PDF from a reader with custom options.
pub fn parse_reader_with_options<R: Read>(reader: R, options: ParseOptions) -> Result<Document> {
    let parser = PdfParser::from_reader_with_options(reader, options)?;
    parser.parse()
}

/// Parse a password-protected PDF file.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
/// * `password` - Document password
///
/// # Example
///
/// ```no_run
/// use unpdf::parse_file_with_password;
///
/// let doc = parse_file_with_password("encrypted.pdf", "secret").unwrap();
/// ```
pub fn parse_file_with_password<P: AsRef<Path>>(path: P, password: &str) -> Result<Document> {
    let options = ParseOptions::new().with_password(password);
    parse_file_with_options(path, options)
}

/// Extract plain text from a PDF file.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
///
/// # Example
///
/// ```no_run
/// use unpdf::extract_text;
///
/// let text = extract_text("document.pdf").unwrap();
/// println!("{}", text);
/// ```
pub fn extract_text<P: AsRef<Path>>(path: P) -> Result<String> {
    let doc = parse_file(path)?;
    Ok(doc.plain_text())
}

/// Convert a PDF to Markdown.
///
/// # Arguments
///
/// * `path` - Path to the PDF file
///
/// # Example
///
/// ```no_run
/// use unpdf::to_markdown;
///
/// let markdown = to_markdown("document.pdf").unwrap();
/// std::fs::write("output.md", markdown).unwrap();
/// ```
pub fn to_markdown<P: AsRef<Path>>(path: P) -> Result<String> {
    let doc = parse_file(path)?;
    let options = RenderOptions::default();
    render::to_markdown(&doc, &options)
}

/// Convert a PDF to Markdown with custom options.
///
/// # Example
///
/// ```no_run
/// use unpdf::{to_markdown_with_options, RenderOptions, CleanupPreset};
///
/// let options = RenderOptions::new()
///     .with_frontmatter(true)
///     .with_cleanup_preset(CleanupPreset::Aggressive);
/// let markdown = to_markdown_with_options("document.pdf", &options).unwrap();
/// ```
pub fn to_markdown_with_options<P: AsRef<Path>>(
    path: P,
    options: &RenderOptions,
) -> Result<String> {
    let doc = parse_file(path)?;
    render::to_markdown(&doc, options)
}

/// Convert a PDF to plain text with cleanup.
///
/// # Example
///
/// ```no_run
/// use unpdf::{to_text, RenderOptions, CleanupPreset};
///
/// let options = RenderOptions::new()
///     .with_cleanup_preset(CleanupPreset::Standard);
/// let text = to_text("document.pdf", &options).unwrap();
/// ```
pub fn to_text<P: AsRef<Path>>(path: P, options: &RenderOptions) -> Result<String> {
    let doc = parse_file(path)?;
    render::to_text(&doc, options)
}

/// Convert a PDF to JSON.
///
/// # Example
///
/// ```no_run
/// use unpdf::{to_json, JsonFormat};
///
/// let json = to_json("document.pdf", JsonFormat::Pretty).unwrap();
/// std::fs::write("output.json", json).unwrap();
/// ```
pub fn to_json<P: AsRef<Path>>(path: P, format: JsonFormat) -> Result<String> {
    let doc = parse_file(path)?;
    render::to_json(&doc, format)
}

/// Builder for parsing and converting PDF documents.
///
/// # Example
///
/// ```no_run
/// use unpdf::Unpdf;
///
/// let markdown = Unpdf::new()
///     .with_images(true)
///     .with_image_dir("./images")
///     .with_frontmatter()
///     .lenient()
///     .parse("document.pdf")?
///     .to_markdown()?;
/// # Ok::<(), unpdf::Error>(())
/// ```
pub struct Unpdf {
    parse_options: ParseOptions,
    render_options: RenderOptions,
}

impl Unpdf {
    /// Create a new Unpdf builder.
    pub fn new() -> Self {
        Self {
            parse_options: ParseOptions::default(),
            render_options: RenderOptions::default(),
        }
    }

    /// Enable lenient parsing mode.
    pub fn lenient(mut self) -> Self {
        self.parse_options = self.parse_options.lenient();
        self
    }

    /// Extract text only (no structure).
    pub fn text_only(mut self) -> Self {
        self.parse_options = self.parse_options.text_only();
        self
    }

    /// Disable parallel processing.
    pub fn sequential(mut self) -> Self {
        self.parse_options = self.parse_options.sequential();
        self
    }

    /// Set memory limit in MB.
    pub fn with_memory_limit_mb(mut self, mb: u32) -> Self {
        self.parse_options = self.parse_options.with_memory_limit(mb);
        self
    }

    /// Enable image extraction.
    pub fn with_images(mut self, extract: bool) -> Self {
        self.parse_options = self.parse_options.with_resources(extract);
        self
    }

    /// Set image output directory.
    pub fn with_image_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.render_options = self.render_options.with_image_dir(dir);
        self
    }

    /// Enable frontmatter in output.
    pub fn with_frontmatter(mut self) -> Self {
        self.render_options = self.render_options.with_frontmatter(true);
        self
    }

    /// Set table fallback mode.
    pub fn with_table_fallback(mut self, fallback: TableFallback) -> Self {
        self.render_options = self.render_options.with_table_fallback(fallback);
        self
    }

    /// Set cleanup preset.
    pub fn with_cleanup(mut self, preset: CleanupPreset) -> Self {
        self.render_options = self.render_options.with_cleanup_preset(preset);
        self
    }

    /// Set document password.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.parse_options = self.parse_options.with_password(password);
        self
    }

    /// Set page selection.
    pub fn with_pages(mut self, pages: PageSelection) -> Self {
        self.parse_options = self.parse_options.with_pages(pages.clone());
        self.render_options = self.render_options.with_pages(pages);
        self
    }

    /// Parse a PDF file and return a result wrapper.
    pub fn parse<P: AsRef<Path>>(self, path: P) -> Result<UnpdfResult> {
        let parser = PdfParser::open_with_options(path, self.parse_options)?;
        let document = parser.parse()?;
        Ok(UnpdfResult {
            document,
            render_options: self.render_options,
        })
    }

    /// Parse a PDF from bytes.
    pub fn parse_bytes(self, data: &[u8]) -> Result<UnpdfResult> {
        let parser = PdfParser::from_bytes_with_options(data, self.parse_options)?;
        let document = parser.parse()?;
        Ok(UnpdfResult {
            document,
            render_options: self.render_options,
        })
    }
}

impl Default for Unpdf {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of parsing a PDF document.
pub struct UnpdfResult {
    /// The parsed document
    pub document: Document,
    /// Render options to use
    render_options: RenderOptions,
}

impl UnpdfResult {
    /// Convert to Markdown.
    pub fn to_markdown(&self) -> Result<String> {
        render::to_markdown(&self.document, &self.render_options)
    }

    /// Convert to plain text.
    pub fn to_text(&self) -> Result<String> {
        render::to_text(&self.document, &self.render_options)
    }

    /// Convert to JSON.
    pub fn to_json(&self, format: JsonFormat) -> Result<String> {
        render::to_json(&self.document, format)
    }

    /// Get plain text without cleanup.
    pub fn plain_text(&self) -> String {
        self.document.plain_text()
    }

    /// Get the document.
    pub fn document(&self) -> &Document {
        &self.document
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unpdf_builder() {
        let unpdf = Unpdf::new()
            .lenient()
            .with_frontmatter()
            .with_cleanup(CleanupPreset::Standard);

        assert!(matches!(
            unpdf.parse_options.error_mode,
            parser::ErrorMode::Lenient
        ));
        assert!(unpdf.render_options.include_frontmatter);
    }
}
