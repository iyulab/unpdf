//! Rendering result with metadata and statistics.

use crate::model::Metadata;
use serde::{Deserialize, Serialize};

/// Result of rendering a document, including content and statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderResult {
    /// The rendered content (Markdown, text, etc.)
    pub content: String,

    /// Document metadata (copied from source document)
    pub metadata: Metadata,

    /// Extraction statistics
    pub stats: ExtractionStats,
}

impl RenderResult {
    /// Create a new render result.
    pub fn new(content: String, metadata: Metadata, stats: ExtractionStats) -> Self {
        Self {
            content,
            metadata,
            stats,
        }
    }

    /// Create a simple result with just content.
    pub fn content_only(content: String) -> Self {
        Self {
            content,
            metadata: Metadata::default(),
            stats: ExtractionStats::default(),
        }
    }

    /// Get the content length in bytes.
    pub fn content_len(&self) -> usize {
        self.content.len()
    }
}

/// Statistics collected during content extraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionStats {
    /// Total number of pages processed
    pub page_count: u32,

    /// Number of paragraphs extracted
    pub paragraph_count: u32,

    /// Number of tables extracted
    pub table_count: u32,

    /// Number of images found
    pub image_count: u32,

    /// Number of list items extracted
    pub list_item_count: u32,

    /// Approximate word count (whitespace-separated tokens)
    pub word_count: u32,

    /// Character count (excluding whitespace)
    pub char_count: u32,

    /// Number of headings extracted
    pub heading_count: u32,

    /// Number of horizontal rules
    pub horizontal_rule_count: u32,
}

impl ExtractionStats {
    /// Create new empty statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment paragraph count.
    pub fn add_paragraph(&mut self) {
        self.paragraph_count += 1;
    }

    /// Increment table count.
    pub fn add_table(&mut self) {
        self.table_count += 1;
    }

    /// Increment image count.
    pub fn add_image(&mut self) {
        self.image_count += 1;
    }

    /// Increment list item count.
    pub fn add_list_item(&mut self) {
        self.list_item_count += 1;
    }

    /// Increment heading count.
    pub fn add_heading(&mut self) {
        self.heading_count += 1;
    }

    /// Increment horizontal rule count.
    pub fn add_horizontal_rule(&mut self) {
        self.horizontal_rule_count += 1;
    }

    /// Increment page count.
    pub fn add_page(&mut self) {
        self.page_count += 1;
    }

    /// Add word and character counts from text.
    pub fn count_text(&mut self, text: &str) {
        // Word count: whitespace-separated tokens
        self.word_count += text.split_whitespace().count() as u32;

        // Character count: non-whitespace characters
        self.char_count += text.chars().filter(|c| !c.is_whitespace()).count() as u32;
    }

    /// Merge another stats instance into this one.
    pub fn merge(&mut self, other: &ExtractionStats) {
        self.page_count += other.page_count;
        self.paragraph_count += other.paragraph_count;
        self.table_count += other.table_count;
        self.image_count += other.image_count;
        self.list_item_count += other.list_item_count;
        self.word_count += other.word_count;
        self.char_count += other.char_count;
        self.heading_count += other.heading_count;
        self.horizontal_rule_count += other.horizontal_rule_count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_stats_count_text() {
        let mut stats = ExtractionStats::new();
        stats.count_text("Hello, world! This is a test.");

        assert_eq!(stats.word_count, 6);
        // "Helloworld!Thisisatest." = 23 non-whitespace chars
        assert_eq!(stats.char_count, 24);
    }

    #[test]
    fn test_extraction_stats_merge() {
        let mut stats1 = ExtractionStats::new();
        stats1.paragraph_count = 5;
        stats1.table_count = 2;

        let stats2 = ExtractionStats {
            paragraph_count: 3,
            table_count: 1,
            image_count: 4,
            ..Default::default()
        };

        stats1.merge(&stats2);

        assert_eq!(stats1.paragraph_count, 8);
        assert_eq!(stats1.table_count, 3);
        assert_eq!(stats1.image_count, 4);
    }

    #[test]
    fn test_render_result_content_only() {
        let result = RenderResult::content_only("# Hello".to_string());
        assert_eq!(result.content, "# Hello");
        assert_eq!(result.stats.paragraph_count, 0);
    }
}
