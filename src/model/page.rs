//! Page-level types.

use super::{Paragraph, Table};
use serde::{Deserialize, Serialize};

/// A single page in the document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// Page number (1-indexed)
    pub number: u32,

    /// Page width in points (1 point = 1/72 inch)
    pub width: f32,

    /// Page height in points
    pub height: f32,

    /// Content blocks on the page
    pub elements: Vec<Block>,

    /// Page rotation in degrees (0, 90, 180, 270)
    pub rotation: u16,
}

impl Page {
    /// Create a new page with the given dimensions.
    pub fn new(number: u32, width: f32, height: f32) -> Self {
        Self {
            number,
            width,
            height,
            elements: Vec::new(),
            rotation: 0,
        }
    }

    /// Create a new page with standard Letter size (8.5 x 11 inches).
    pub fn letter(number: u32) -> Self {
        Self::new(number, 612.0, 792.0) // 8.5 * 72, 11 * 72
    }

    /// Create a new page with standard A4 size (210 x 297 mm).
    pub fn a4(number: u32) -> Self {
        Self::new(number, 595.0, 842.0) // 210mm * 2.834, 297mm * 2.834
    }

    /// Add a block to the page.
    pub fn add_block(&mut self, block: Block) {
        self.elements.push(block);
    }

    /// Add a paragraph to the page.
    pub fn add_paragraph(&mut self, paragraph: Paragraph) {
        self.elements.push(Block::Paragraph(paragraph));
    }

    /// Add a table to the page.
    pub fn add_table(&mut self, table: Table) {
        self.elements.push(Block::Table(table));
    }

    /// Get plain text content of the page.
    pub fn plain_text(&self) -> String {
        self.elements
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(p) => Some(p.plain_text()),
                Block::Table(t) => Some(t.plain_text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Check if the page is empty (no content blocks).
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get the number of blocks on the page.
    pub fn block_count(&self) -> usize {
        self.elements.len()
    }

    /// Get page dimensions as (width, height) tuple.
    pub fn dimensions(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    /// Check if the page is in landscape orientation.
    pub fn is_landscape(&self) -> bool {
        self.width > self.height
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::letter(1)
    }
}

/// A content block on a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    /// A paragraph of text
    Paragraph(Paragraph),

    /// A table
    Table(Table),

    /// An image reference
    Image {
        /// Resource ID for the image
        resource_id: String,
        /// Alternative text
        alt_text: Option<String>,
        /// Image width in points
        width: Option<f32>,
        /// Image height in points
        height: Option<f32>,
        /// X position on page
        x: Option<f32>,
        /// Y position on page
        y: Option<f32>,
    },

    /// A horizontal rule / separator
    HorizontalRule,

    /// A page break marker
    PageBreak,

    /// A section break marker
    SectionBreak,

    /// Raw/unstructured content
    Raw {
        /// Raw content text
        content: String,
    },
}

impl Block {
    /// Create an image block.
    pub fn image(resource_id: impl Into<String>) -> Self {
        Block::Image {
            resource_id: resource_id.into(),
            alt_text: None,
            width: None,
            height: None,
            x: None,
            y: None,
        }
    }

    /// Create an image block with dimensions.
    pub fn image_with_size(resource_id: impl Into<String>, width: f32, height: f32) -> Self {
        Block::Image {
            resource_id: resource_id.into(),
            alt_text: None,
            width: Some(width),
            height: Some(height),
            x: None,
            y: None,
        }
    }

    /// Check if this block is a paragraph.
    pub fn is_paragraph(&self) -> bool {
        matches!(self, Block::Paragraph(_))
    }

    /// Check if this block is a table.
    pub fn is_table(&self) -> bool {
        matches!(self, Block::Table(_))
    }

    /// Check if this block is an image.
    pub fn is_image(&self) -> bool {
        matches!(self, Block::Image { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_new() {
        let page = Page::new(1, 612.0, 792.0);
        assert_eq!(page.number, 1);
        assert_eq!(page.width, 612.0);
        assert_eq!(page.height, 792.0);
        assert!(page.is_empty());
    }

    #[test]
    fn test_page_letter_a4() {
        let letter = Page::letter(1);
        assert!(!letter.is_landscape());

        let a4 = Page::a4(1);
        assert!(!a4.is_landscape());
    }

    #[test]
    fn test_block_variants() {
        let img = Block::image("img1");
        assert!(img.is_image());
        assert!(!img.is_paragraph());
    }
}
