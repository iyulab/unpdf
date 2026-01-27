//! Document converter module providing a plugin architecture for multiple formats.
//!
//! This module defines a flexible converter system that allows registering
//! converters for different file formats and dispatching conversions based
//! on file extensions.
//!
//! # Example
//!
//! ```no_run
//! use unpdf::convert::{ConverterRegistry, ConvertOptions, PdfConverter};
//! use std::sync::Arc;
//! use std::path::Path;
//!
//! fn main() -> unpdf::Result<()> {
//!     let mut registry = ConverterRegistry::new();
//!     registry.register(Arc::new(PdfConverter::new()));
//!
//!     let result = registry.convert(Path::new("document.pdf"), &ConvertOptions::default())?;
//!     println!("{}", result.content);
//!     Ok(())
//! }
//! ```

mod pdf;

pub use pdf::PdfConverter;

use crate::error::{Error, Result};
use crate::model::Metadata;
use crate::render::{ExtractionStats, RenderOptions};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Options for document conversion.
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Rendering options
    pub render: RenderOptions,

    /// Password for encrypted documents
    pub password: Option<String>,

    /// Whether to collect statistics during conversion
    pub collect_stats: bool,

    /// Output format
    pub output_format: OutputFormat,
}

impl ConvertOptions {
    /// Create new conversion options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set rendering options.
    pub fn with_render_options(mut self, options: RenderOptions) -> Self {
        self.render = options;
        self
    }

    /// Set document password.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Enable statistics collection.
    pub fn with_stats(mut self, collect: bool) -> Self {
        self.collect_stats = collect;
        self
    }

    /// Set output format.
    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.output_format = format;
        self
    }
}

/// Output format for conversion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Markdown format
    #[default]
    Markdown,

    /// Plain text
    Text,

    /// JSON structure
    Json,
}

/// Result of document conversion.
#[derive(Debug, Clone)]
pub struct ConvertResult {
    /// Converted content
    pub content: String,

    /// Source document metadata
    pub metadata: Metadata,

    /// Extraction statistics (if collected)
    pub stats: Option<ExtractionStats>,

    /// MIME type of the output
    pub mime_type: &'static str,
}

impl ConvertResult {
    /// Create a new conversion result.
    pub fn new(content: String, metadata: Metadata) -> Self {
        Self {
            content,
            metadata,
            stats: None,
            mime_type: "text/markdown",
        }
    }

    /// Set extraction statistics.
    pub fn with_stats(mut self, stats: ExtractionStats) -> Self {
        self.stats = Some(stats);
        self
    }

    /// Set MIME type.
    pub fn with_mime_type(mut self, mime_type: &'static str) -> Self {
        self.mime_type = mime_type;
        self
    }

    /// Get content length in bytes.
    pub fn content_len(&self) -> usize {
        self.content.len()
    }
}

/// Trait for document converters.
///
/// Implement this trait to add support for a new document format.
pub trait DocumentConverter: Send + Sync {
    /// Get the supported file extensions for this converter.
    ///
    /// Extensions should be lowercase without the leading dot (e.g., `["pdf"]`).
    fn supported_extensions(&self) -> &[&str];

    /// Get the name of this converter.
    fn name(&self) -> &str;

    /// Convert a file at the given path.
    fn convert(&self, path: &Path, options: &ConvertOptions) -> Result<ConvertResult>;

    /// Convert from bytes.
    fn convert_bytes(&self, bytes: &[u8], options: &ConvertOptions) -> Result<ConvertResult>;

    /// Check if this converter supports the given extension.
    fn supports_extension(&self, ext: &str) -> bool {
        let ext_lower = ext.to_lowercase();
        self.supported_extensions().iter().any(|e| *e == ext_lower)
    }
}

/// Registry for document converters.
///
/// The registry maps file extensions to converters and provides
/// convenient methods for converting documents.
pub struct ConverterRegistry {
    converters: HashMap<String, Arc<dyn DocumentConverter>>,
    by_name: HashMap<String, Arc<dyn DocumentConverter>>,
}

impl ConverterRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            converters: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    /// Create a registry with default converters (PDF).
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(PdfConverter::new()));
        registry
    }

    /// Register a converter.
    ///
    /// The converter will be registered for all its supported extensions.
    pub fn register(&mut self, converter: Arc<dyn DocumentConverter>) {
        for ext in converter.supported_extensions() {
            self.converters
                .insert(ext.to_lowercase(), converter.clone());
        }
        self.by_name
            .insert(converter.name().to_lowercase(), converter);
    }

    /// Get a converter by file extension.
    pub fn get_by_extension(&self, ext: &str) -> Option<Arc<dyn DocumentConverter>> {
        self.converters.get(&ext.to_lowercase()).cloned()
    }

    /// Get a converter by name.
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn DocumentConverter>> {
        self.by_name.get(&name.to_lowercase()).cloned()
    }

    /// Check if an extension is supported.
    pub fn supports(&self, ext: &str) -> bool {
        self.converters.contains_key(&ext.to_lowercase())
    }

    /// Get all supported extensions.
    pub fn supported_extensions(&self) -> Vec<&str> {
        self.converters.keys().map(|s| s.as_str()).collect()
    }

    /// Convert a file using the appropriate converter.
    pub fn convert(&self, path: &Path, options: &ConvertOptions) -> Result<ConvertResult> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| Error::Other("File has no extension".into()))?;

        let converter = self
            .get_by_extension(ext)
            .ok_or_else(|| Error::Other(format!("No converter for extension: {}", ext)))?;

        converter.convert(path, options)
    }

    /// Convert bytes using the specified extension to determine the converter.
    pub fn convert_bytes(
        &self,
        bytes: &[u8],
        ext: &str,
        options: &ConvertOptions,
    ) -> Result<ConvertResult> {
        let converter = self
            .get_by_extension(ext)
            .ok_or_else(|| Error::Other(format!("No converter for extension: {}", ext)))?;

        converter.convert_bytes(bytes, options)
    }
}

impl Default for ConverterRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_options_builder() {
        let options = ConvertOptions::new()
            .with_password("secret")
            .with_stats(true)
            .with_format(OutputFormat::Text);

        assert_eq!(options.password, Some("secret".to_string()));
        assert!(options.collect_stats);
        assert_eq!(options.output_format, OutputFormat::Text);
    }

    #[test]
    fn test_registry_with_defaults() {
        let registry = ConverterRegistry::with_defaults();
        assert!(registry.supports("pdf"));
        assert!(registry.supports("PDF"));
        assert!(!registry.supports("docx"));
    }

    #[test]
    fn test_registry_get_by_extension() {
        let registry = ConverterRegistry::with_defaults();
        let converter = registry.get_by_extension("pdf");
        assert!(converter.is_some());
        assert_eq!(converter.unwrap().name(), "pdf");
    }

    #[test]
    fn test_registry_get_by_name() {
        let registry = ConverterRegistry::with_defaults();
        let converter = registry.get_by_name("pdf");
        assert!(converter.is_some());
    }
}
