//! PDF document converter implementation.

use crate::error::Result;
use crate::parser::{ParseOptions, PdfParser};
use crate::render::{to_json, to_markdown_with_stats, to_text, JsonFormat};
use std::path::Path;

use super::{ConvertOptions, ConvertResult, DocumentConverter, OutputFormat};

/// PDF document converter.
///
/// Converts PDF documents to Markdown, plain text, or JSON.
#[derive(Debug, Clone, Default)]
pub struct PdfConverter {
    _private: (),
}

impl PdfConverter {
    /// Create a new PDF converter.
    pub fn new() -> Self {
        Self { _private: () }
    }

    fn build_parse_options(&self, options: &ConvertOptions) -> ParseOptions {
        let mut parse_opts = ParseOptions::new().with_pages(options.render.page_selection.clone());

        if let Some(ref password) = options.password {
            parse_opts = parse_opts.with_password(password);
        }

        parse_opts
    }

    fn convert_document(
        &self,
        doc: crate::model::Document,
        options: &ConvertOptions,
    ) -> Result<ConvertResult> {
        let metadata = doc.metadata.clone();

        match options.output_format {
            OutputFormat::Markdown => {
                if options.collect_stats {
                    let render_result = to_markdown_with_stats(&doc, &options.render)?;
                    Ok(ConvertResult::new(render_result.content, metadata)
                        .with_stats(render_result.stats)
                        .with_mime_type("text/markdown"))
                } else {
                    let content = crate::render::to_markdown(&doc, &options.render)?;
                    Ok(ConvertResult::new(content, metadata).with_mime_type("text/markdown"))
                }
            }
            OutputFormat::Text => {
                let content = to_text(&doc, &options.render)?;
                Ok(ConvertResult::new(content, metadata).with_mime_type("text/plain"))
            }
            OutputFormat::Json => {
                let content = to_json(&doc, JsonFormat::Pretty)?;
                Ok(ConvertResult::new(content, metadata).with_mime_type("application/json"))
            }
        }
    }
}

impl DocumentConverter for PdfConverter {
    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }

    fn name(&self) -> &str {
        "pdf"
    }

    fn convert(&self, path: &Path, options: &ConvertOptions) -> Result<ConvertResult> {
        let parse_opts = self.build_parse_options(options);
        let parser = PdfParser::open_with_options(path, parse_opts)?;
        let doc = parser.parse()?;
        self.convert_document(doc, options)
    }

    fn convert_bytes(&self, bytes: &[u8], options: &ConvertOptions) -> Result<ConvertResult> {
        let parse_opts = self.build_parse_options(options);
        let parser = PdfParser::from_bytes_with_options(bytes, parse_opts)?;
        let doc = parser.parse()?;
        self.convert_document(doc, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_converter_extensions() {
        let converter = PdfConverter::new();
        assert_eq!(converter.supported_extensions(), &["pdf"]);
        assert!(converter.supports_extension("pdf"));
        assert!(converter.supports_extension("PDF"));
        assert!(!converter.supports_extension("docx"));
    }

    #[test]
    fn test_pdf_converter_name() {
        let converter = PdfConverter::new();
        assert_eq!(converter.name(), "pdf");
    }
}
