//! Visitor pattern for customizing document rendering.
//!
//! The visitor pattern allows users to customize how different document
//! elements are rendered without modifying the core rendering logic.
//!
//! # Example
//!
//! ```
//! use unpdf::render::visitor::{DocumentVisitor, VisitorAction};
//! use unpdf::model::Table;
//!
//! struct CustomTableVisitor;
//!
//! impl DocumentVisitor for CustomTableVisitor {
//!     fn visit_table(&mut self, _table: &Table) -> VisitorAction {
//!         // Convert all tables to a custom format
//!         VisitorAction::Replace("<!-- table omitted -->".to_string())
//!     }
//! }
//! ```

use crate::model::{Paragraph, Table};

/// Action returned by visitor methods to control rendering behavior.
#[derive(Debug, Clone, Default)]
pub enum VisitorAction {
    /// Continue with default rendering.
    #[default]
    Continue,

    /// Replace the element with custom output.
    Replace(String),

    /// Skip this element entirely (produce no output).
    Skip,
}

impl VisitorAction {
    /// Check if this action indicates the element should be skipped.
    pub fn should_skip(&self) -> bool {
        matches!(self, VisitorAction::Skip)
    }

    /// Check if this action provides replacement content.
    pub fn is_replace(&self) -> bool {
        matches!(self, VisitorAction::Replace(_))
    }

    /// Get replacement content if available.
    pub fn replacement(&self) -> Option<&str> {
        match self {
            VisitorAction::Replace(s) => Some(s),
            _ => None,
        }
    }
}

/// Trait for visiting document elements during rendering.
///
/// Implement this trait to customize how specific elements are rendered.
/// All methods return `VisitorAction::Continue` by default.
pub trait DocumentVisitor: Send + Sync {
    /// Called before rendering a paragraph.
    ///
    /// # Arguments
    /// * `para` - The paragraph about to be rendered
    ///
    /// # Returns
    /// Action indicating how to handle this paragraph
    fn visit_paragraph(&mut self, para: &Paragraph) -> VisitorAction {
        let _ = para;
        VisitorAction::Continue
    }

    /// Called before rendering a table.
    ///
    /// # Arguments
    /// * `table` - The table about to be rendered
    ///
    /// # Returns
    /// Action indicating how to handle this table
    fn visit_table(&mut self, table: &Table) -> VisitorAction {
        let _ = table;
        VisitorAction::Continue
    }

    /// Called before rendering an image.
    ///
    /// # Arguments
    /// * `id` - Resource ID of the image
    /// * `alt` - Optional alt text for the image
    ///
    /// # Returns
    /// Action indicating how to handle this image
    fn visit_image(&mut self, id: &str, alt: Option<&str>) -> VisitorAction {
        let _ = (id, alt);
        VisitorAction::Continue
    }

    /// Called before rendering a heading.
    ///
    /// # Arguments
    /// * `text` - The heading text content
    /// * `level` - Heading level (1-6)
    ///
    /// # Returns
    /// Action indicating how to handle this heading
    fn visit_heading(&mut self, text: &str, level: u8) -> VisitorAction {
        let _ = (text, level);
        VisitorAction::Continue
    }

    /// Called before rendering a horizontal rule.
    ///
    /// # Returns
    /// Action indicating how to handle this horizontal rule
    fn visit_horizontal_rule(&mut self) -> VisitorAction {
        VisitorAction::Continue
    }

    /// Called before rendering a list item.
    ///
    /// # Arguments
    /// * `para` - The paragraph containing the list item content
    /// * `level` - Nesting level of the list item
    /// * `ordered` - Whether this is an ordered list
    ///
    /// # Returns
    /// Action indicating how to handle this list item
    fn visit_list_item(&mut self, para: &Paragraph, level: u8, ordered: bool) -> VisitorAction {
        let _ = (para, level, ordered);
        VisitorAction::Continue
    }

    /// Called before rendering raw content.
    ///
    /// # Arguments
    /// * `content` - The raw content string
    ///
    /// # Returns
    /// Action indicating how to handle this raw content
    fn visit_raw(&mut self, content: &str) -> VisitorAction {
        let _ = content;
        VisitorAction::Continue
    }

    /// Called at the start of rendering a new page.
    ///
    /// # Arguments
    /// * `page_number` - The 1-indexed page number
    fn on_page_start(&mut self, page_number: u32) {
        let _ = page_number;
    }

    /// Called at the end of rendering a page.
    ///
    /// # Arguments
    /// * `page_number` - The 1-indexed page number
    fn on_page_end(&mut self, page_number: u32) {
        let _ = page_number;
    }
}

/// Default visitor that performs no customization.
///
/// All visit methods return `VisitorAction::Continue`.
#[derive(Debug, Clone, Default)]
pub struct DefaultVisitor;

impl DefaultVisitor {
    /// Create a new default visitor.
    pub fn new() -> Self {
        Self
    }
}

impl DocumentVisitor for DefaultVisitor {}

/// Visitor that skips all images.
#[derive(Debug, Clone, Default)]
pub struct SkipImagesVisitor;

impl DocumentVisitor for SkipImagesVisitor {
    fn visit_image(&mut self, _id: &str, _alt: Option<&str>) -> VisitorAction {
        VisitorAction::Skip
    }
}

/// Visitor that converts tables to simple text representation.
#[derive(Debug, Clone, Default)]
pub struct SimpleTableVisitor;

impl DocumentVisitor for SimpleTableVisitor {
    fn visit_table(&mut self, table: &Table) -> VisitorAction {
        // Convert table to simple pipe-separated text
        let mut output = String::new();
        for row in &table.rows {
            let cells: Vec<String> = row.cells.iter().map(|c| c.plain_text()).collect();
            output.push_str(&cells.join(" | "));
            output.push('\n');
        }
        output.push('\n');
        VisitorAction::Replace(output)
    }
}

/// Visitor that limits heading depth.
#[derive(Debug, Clone)]
pub struct MaxHeadingDepthVisitor {
    max_level: u8,
}

impl MaxHeadingDepthVisitor {
    /// Create a visitor that limits headings to the specified max level.
    pub fn new(max_level: u8) -> Self {
        Self {
            max_level: max_level.clamp(1, 6),
        }
    }
}

impl DocumentVisitor for MaxHeadingDepthVisitor {
    fn visit_heading(&mut self, text: &str, level: u8) -> VisitorAction {
        let effective_level = level.min(self.max_level);
        let prefix = "#".repeat(effective_level as usize);
        VisitorAction::Replace(format!("{} {}\n\n", prefix, text))
    }
}

/// Composite visitor that chains multiple visitors.
///
/// Visitors are called in order. The first visitor that returns
/// a non-Continue action determines the result.
pub struct CompositeVisitor {
    visitors: Vec<Box<dyn DocumentVisitor>>,
}

impl CompositeVisitor {
    /// Create a new composite visitor.
    pub fn new() -> Self {
        Self {
            visitors: Vec::new(),
        }
    }

    /// Add a visitor to the chain.
    pub fn with_visitor<V: DocumentVisitor + 'static>(mut self, visitor: V) -> Self {
        self.visitors.push(Box::new(visitor));
        self
    }
}

impl Default for CompositeVisitor {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentVisitor for CompositeVisitor {
    fn visit_paragraph(&mut self, para: &Paragraph) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_paragraph(para);
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn visit_table(&mut self, table: &Table) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_table(table);
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn visit_image(&mut self, id: &str, alt: Option<&str>) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_image(id, alt);
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn visit_heading(&mut self, text: &str, level: u8) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_heading(text, level);
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn visit_horizontal_rule(&mut self) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_horizontal_rule();
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn visit_list_item(&mut self, para: &Paragraph, level: u8, ordered: bool) -> VisitorAction {
        for visitor in &mut self.visitors {
            let action = visitor.visit_list_item(para, level, ordered);
            if !matches!(action, VisitorAction::Continue) {
                return action;
            }
        }
        VisitorAction::Continue
    }

    fn on_page_start(&mut self, page_number: u32) {
        for visitor in &mut self.visitors {
            visitor.on_page_start(page_number);
        }
    }

    fn on_page_end(&mut self, page_number: u32) {
        for visitor in &mut self.visitors {
            visitor.on_page_end(page_number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visitor_action_default() {
        let action = VisitorAction::default();
        assert!(matches!(action, VisitorAction::Continue));
    }

    #[test]
    fn test_visitor_action_should_skip() {
        assert!(!VisitorAction::Continue.should_skip());
        assert!(!VisitorAction::Replace("test".into()).should_skip());
        assert!(VisitorAction::Skip.should_skip());
    }

    #[test]
    fn test_visitor_action_replacement() {
        assert!(VisitorAction::Continue.replacement().is_none());
        assert!(VisitorAction::Skip.replacement().is_none());
        assert_eq!(
            VisitorAction::Replace("hello".into()).replacement(),
            Some("hello")
        );
    }

    #[test]
    fn test_default_visitor() {
        let mut visitor = DefaultVisitor::new();
        let para = Paragraph::new();
        let action = visitor.visit_paragraph(&para);
        assert!(matches!(action, VisitorAction::Continue));
    }

    #[test]
    fn test_skip_images_visitor() {
        let mut visitor = SkipImagesVisitor;
        let action = visitor.visit_image("img1", Some("alt text"));
        assert!(action.should_skip());
    }

    #[test]
    fn test_max_heading_depth_visitor() {
        let mut visitor = MaxHeadingDepthVisitor::new(2);
        let action = visitor.visit_heading("Deep Heading", 4);
        assert!(action.is_replace());
        let replacement = action.replacement().unwrap();
        assert!(replacement.starts_with("## "));
    }

    #[test]
    fn test_composite_visitor() {
        let mut composite = CompositeVisitor::new()
            .with_visitor(SkipImagesVisitor)
            .with_visitor(DefaultVisitor);

        // Images should be skipped
        let action = composite.visit_image("img1", None);
        assert!(action.should_skip());

        // Other elements should continue
        let para = Paragraph::new();
        let action = composite.visit_paragraph(&para);
        assert!(matches!(action, VisitorAction::Continue));
    }
}
