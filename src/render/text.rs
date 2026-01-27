//! Plain text rendering for PDF documents.

use crate::error::Result;
use crate::model::Document;

use super::{CleanupPipeline, RenderOptions};

/// Convert a document to plain text.
pub fn to_text(doc: &Document, options: &RenderOptions) -> Result<String> {
    let mut output = doc.plain_text();

    // Apply cleanup if configured
    if let Some(ref cleanup_options) = options.cleanup {
        let pipeline = CleanupPipeline::new(cleanup_options.clone());
        output = pipeline.process(&output);
    }

    Ok(output.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Paragraph};

    #[test]
    fn test_to_text() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello, world!"));
        page.add_paragraph(Paragraph::with_text("Second paragraph."));
        doc.add_page(page);

        let options = RenderOptions::default();
        let result = to_text(&doc, &options).unwrap();

        assert!(result.contains("Hello, world!"));
        assert!(result.contains("Second paragraph."));
    }
}
