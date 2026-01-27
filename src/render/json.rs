//! JSON rendering for PDF documents.

use crate::error::{Error, Result};
use crate::model::Document;

/// JSON output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JsonFormat {
    /// Pretty-printed JSON with indentation
    #[default]
    Pretty,
    /// Compact JSON without extra whitespace
    Compact,
}

/// Convert a document to JSON.
pub fn to_json(doc: &Document, format: JsonFormat) -> Result<String> {
    let result = match format {
        JsonFormat::Pretty => serde_json::to_string_pretty(doc),
        JsonFormat::Compact => serde_json::to_string(doc),
    };

    result.map_err(|e| Error::Render(format!("JSON serialization error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Paragraph};

    #[test]
    fn test_to_json_pretty() {
        let mut doc = Document::new();
        doc.metadata.title = Some("Test".to_string());
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello"));
        doc.add_page(page);

        let json = to_json(&doc, JsonFormat::Pretty).unwrap();
        assert!(json.contains("\"title\""));
        assert!(json.contains("Test"));
        assert!(json.contains('\n')); // Pretty has newlines
    }

    #[test]
    fn test_to_json_compact() {
        let mut doc = Document::new();
        let page = Page::letter(1);
        doc.add_page(page);

        let json = to_json(&doc, JsonFormat::Compact).unwrap();
        assert!(!json.contains('\n')); // Compact has no newlines
    }
}
