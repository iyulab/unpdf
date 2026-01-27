//! Integration tests for the converter module.

use std::path::Path;
use std::sync::Arc;
use unpdf::convert::{
    ConvertOptions, ConvertResult, ConverterRegistry, DocumentConverter, OutputFormat, PdfConverter,
};
use unpdf::error::Result;

/// Mock converter for testing.
struct MockConverter {
    extensions: Vec<&'static str>,
    name: &'static str,
}

impl MockConverter {
    fn new(extensions: Vec<&'static str>, name: &'static str) -> Self {
        Self { extensions, name }
    }
}

impl DocumentConverter for MockConverter {
    fn supported_extensions(&self) -> &[&str] {
        &self.extensions
    }

    fn name(&self) -> &str {
        self.name
    }

    fn convert(&self, _path: &Path, _options: &ConvertOptions) -> Result<ConvertResult> {
        Ok(ConvertResult::new(
            format!("Converted by {}", self.name),
            Default::default(),
        ))
    }

    fn convert_bytes(&self, _bytes: &[u8], _options: &ConvertOptions) -> Result<ConvertResult> {
        Ok(ConvertResult::new(
            format!("Converted bytes by {}", self.name),
            Default::default(),
        ))
    }
}

#[test]
fn test_convert_options_builder() {
    let options = ConvertOptions::new()
        .with_password("secret123")
        .with_stats(true)
        .with_format(OutputFormat::Text);

    assert_eq!(options.password, Some("secret123".to_string()));
    assert!(options.collect_stats);
    assert_eq!(options.output_format, OutputFormat::Text);
}

#[test]
fn test_converter_registry_new() {
    let registry = ConverterRegistry::new();

    // Empty registry should support nothing
    assert!(!registry.supports("pdf"));
    assert!(!registry.supports("docx"));
}

#[test]
fn test_converter_registry_with_defaults() {
    let registry = ConverterRegistry::with_defaults();

    // Should have PDF support
    assert!(registry.supports("pdf"));
    assert!(registry.supports("PDF")); // Case insensitive
    assert!(!registry.supports("docx"));
}

#[test]
fn test_converter_registry_register() {
    let mut registry = ConverterRegistry::new();
    let converter = Arc::new(MockConverter::new(vec!["txt", "text"], "text"));

    registry.register(converter);

    assert!(registry.supports("txt"));
    assert!(registry.supports("text"));
    assert!(registry.supports("TXT")); // Case insensitive
}

#[test]
fn test_converter_registry_get_by_extension() {
    let registry = ConverterRegistry::with_defaults();

    let converter = registry.get_by_extension("pdf");
    assert!(converter.is_some());
    assert_eq!(converter.unwrap().name(), "pdf");

    let converter = registry.get_by_extension("docx");
    assert!(converter.is_none());
}

#[test]
fn test_converter_registry_get_by_name() {
    let registry = ConverterRegistry::with_defaults();

    let converter = registry.get_by_name("pdf");
    assert!(converter.is_some());

    let converter = registry.get_by_name("PDF"); // Case insensitive
    assert!(converter.is_some());

    let converter = registry.get_by_name("unknown");
    assert!(converter.is_none());
}

#[test]
fn test_converter_registry_multiple_converters() {
    let mut registry = ConverterRegistry::new();

    registry.register(Arc::new(PdfConverter::new()));
    registry.register(Arc::new(MockConverter::new(vec!["doc", "docx"], "word")));
    registry.register(Arc::new(MockConverter::new(vec!["xls", "xlsx"], "excel")));

    assert!(registry.supports("pdf"));
    assert!(registry.supports("doc"));
    assert!(registry.supports("docx"));
    assert!(registry.supports("xls"));
    assert!(registry.supports("xlsx"));

    // Check we get the right converter
    let converter = registry.get_by_name("word");
    assert!(converter.is_some());
    assert!(converter.unwrap().supports_extension("docx"));
}

#[test]
fn test_supported_extensions() {
    let registry = ConverterRegistry::with_defaults();
    let extensions = registry.supported_extensions();

    assert!(extensions.contains(&"pdf"));
}

#[test]
fn test_pdf_converter_extensions() {
    let converter = PdfConverter::new();

    assert_eq!(converter.supported_extensions(), &["pdf"]);
    assert!(converter.supports_extension("pdf"));
    assert!(converter.supports_extension("PDF"));
    assert!(!converter.supports_extension("doc"));
}

#[test]
fn test_pdf_converter_name() {
    let converter = PdfConverter::new();
    assert_eq!(converter.name(), "pdf");
}

#[test]
fn test_convert_result_methods() {
    let result = ConvertResult::new("# Hello".to_string(), Default::default());

    assert_eq!(result.content, "# Hello");
    assert_eq!(result.content_len(), 7);
    assert!(result.stats.is_none());
    assert_eq!(result.mime_type, "text/markdown");
}

#[test]
fn test_convert_result_with_stats() {
    use unpdf::render::ExtractionStats;

    let stats = ExtractionStats {
        page_count: 5,
        paragraph_count: 20,
        ..Default::default()
    };

    let result = ConvertResult::new("content".to_string(), Default::default()).with_stats(stats);

    assert!(result.stats.is_some());
    let stats = result.stats.unwrap();
    assert_eq!(stats.page_count, 5);
    assert_eq!(stats.paragraph_count, 20);
}

#[test]
fn test_convert_result_mime_types() {
    let md_result = ConvertResult::new("# Title".to_string(), Default::default())
        .with_mime_type("text/markdown");
    assert_eq!(md_result.mime_type, "text/markdown");

    let json_result =
        ConvertResult::new("{}".to_string(), Default::default()).with_mime_type("application/json");
    assert_eq!(json_result.mime_type, "application/json");

    let text_result = ConvertResult::new("plain text".to_string(), Default::default())
        .with_mime_type("text/plain");
    assert_eq!(text_result.mime_type, "text/plain");
}

#[test]
fn test_output_format_default() {
    let format = OutputFormat::default();
    assert_eq!(format, OutputFormat::Markdown);
}

#[test]
fn test_mock_converter() {
    let converter = MockConverter::new(vec!["mock"], "mock-converter");

    assert_eq!(converter.name(), "mock-converter");
    assert!(converter.supports_extension("mock"));

    let result = converter
        .convert(Path::new("test.mock"), &ConvertOptions::default())
        .unwrap();
    assert!(result.content.contains("mock-converter"));
}

#[test]
fn test_registry_convert_no_extension_error() {
    let registry = ConverterRegistry::with_defaults();

    // Path without extension
    let result = registry.convert(Path::new("noextension"), &ConvertOptions::default());
    assert!(result.is_err());
}

#[test]
fn test_registry_convert_unsupported_extension_error() {
    let registry = ConverterRegistry::with_defaults();

    // Unsupported extension
    let result = registry.convert(Path::new("test.xyz"), &ConvertOptions::default());
    assert!(result.is_err());
}

#[test]
fn test_registry_convert_bytes_unsupported() {
    let registry = ConverterRegistry::with_defaults();

    let result = registry.convert_bytes(b"test", "xyz", &ConvertOptions::default());
    assert!(result.is_err());
}
