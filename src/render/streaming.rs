//! Streaming renderer for memory-efficient processing of large documents.
//!
//! The streaming renderer provides an iterator-based interface that yields
//! rendering events one at a time, allowing for processing large PDFs without
//! loading the entire rendered output into memory.
//!
//! # Example
//!
//! ```no_run
//! use unpdf::{parse_file, render::{StreamingRenderer, RenderEvent}};
//! use std::io::Write;
//!
//! fn main() -> unpdf::Result<()> {
//!     let doc = parse_file("large-document.pdf")?;
//!     let renderer = StreamingRenderer::new(&doc, Default::default());
//!
//!     for event in renderer {
//!         match event {
//!             RenderEvent::Block(content) => {
//!                 // Process content incrementally
//!                 println!("{}", content);
//!             }
//!             RenderEvent::PageStart { number } => {
//!                 println!("Processing page {}", number);
//!             }
//!             _ => {}
//!         }
//!     }
//!     Ok(())
//! }
//! ```

use crate::model::{Block, Document, Metadata};

use super::RenderOptions;

/// Events emitted during streaming rendering.
#[derive(Debug, Clone)]
pub enum RenderEvent {
    /// Document rendering has started.
    DocumentStart {
        /// Document metadata
        metadata: Metadata,
        /// Total number of pages
        page_count: u32,
    },

    /// A new page is starting.
    PageStart {
        /// 1-indexed page number
        number: u32,
    },

    /// A block of rendered content.
    Block(String),

    /// A page has finished rendering.
    PageEnd {
        /// 1-indexed page number
        number: u32,
    },

    /// Document rendering has completed.
    DocumentEnd,

    /// YAML frontmatter (if enabled).
    Frontmatter(String),
}

impl RenderEvent {
    /// Check if this is a content-bearing event.
    pub fn has_content(&self) -> bool {
        matches!(self, RenderEvent::Block(_) | RenderEvent::Frontmatter(_))
    }

    /// Get the content if this is a content event.
    pub fn content(&self) -> Option<&str> {
        match self {
            RenderEvent::Block(s) | RenderEvent::Frontmatter(s) => Some(s),
            _ => None,
        }
    }

    /// Check if this is a document boundary event.
    pub fn is_document_boundary(&self) -> bool {
        matches!(
            self,
            RenderEvent::DocumentStart { .. } | RenderEvent::DocumentEnd
        )
    }

    /// Check if this is a page boundary event.
    pub fn is_page_boundary(&self) -> bool {
        matches!(
            self,
            RenderEvent::PageStart { .. } | RenderEvent::PageEnd { .. }
        )
    }
}

/// Internal state for the streaming renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamState {
    /// Before any output
    Initial,
    /// Emitted frontmatter (if configured)
    Frontmatter,
    /// Emitted document start
    DocumentStarted,
    /// Currently rendering pages
    InPage {
        page_index: usize,
        block_index: usize,
    },
    /// Between pages
    BetweenPages { next_page: usize },
    /// All pages rendered, waiting to emit document end
    PagesComplete,
    /// Rendering complete
    Done,
}

/// Streaming renderer that yields rendering events as an iterator.
///
/// This is useful for processing large documents without keeping the
/// entire output in memory.
pub struct StreamingRenderer<'a> {
    doc: &'a Document,
    options: RenderOptions,
    state: StreamState,
    current_page_number: u32,
}

impl<'a> StreamingRenderer<'a> {
    /// Create a new streaming renderer.
    pub fn new(doc: &'a Document, options: RenderOptions) -> Self {
        Self {
            doc,
            options,
            state: StreamState::Initial,
            current_page_number: 0,
        }
    }

    /// Get the number of pages in the document.
    pub fn page_count(&self) -> u32 {
        self.doc.page_count()
    }

    /// Check if rendering is complete.
    pub fn is_done(&self) -> bool {
        self.state == StreamState::Done
    }

    /// Get the current page number being processed.
    pub fn current_page(&self) -> u32 {
        self.current_page_number
    }

    /// Find the next page that should be rendered (respecting page selection).
    fn find_next_page(&self, start_index: usize) -> Option<usize> {
        for i in start_index..self.doc.pages.len() {
            let page_num = self.doc.pages[i].number;
            if self.options.page_selection.includes(page_num) {
                return Some(i);
            }
        }
        None
    }

    /// Render a single block to string.
    fn render_block(&self, block: &Block) -> String {
        match block {
            Block::Paragraph(p) => {
                if p.is_empty() {
                    return String::new();
                }

                let mut output = String::new();

                // Handle headings
                if let Some(level) = p.style.heading_level {
                    let level = level.min(self.options.max_heading_level);
                    let prefix = "#".repeat(level as usize);
                    output.push_str(&prefix);
                    output.push(' ');
                    self.render_inline_content(&mut output, &p.content);
                    output.push_str("\n\n");
                    return output;
                }

                // Handle list items
                if let Some(ref list_info) = p.style.list_info {
                    self.render_list_item(&mut output, p, list_info);
                    return output;
                }

                // Normal paragraph
                self.render_inline_content(&mut output, &p.content);
                output.push_str("\n\n");
                output
            }
            Block::Table(t) => {
                if t.is_empty() {
                    return String::new();
                }

                let mut output = String::new();
                let col_count = t.column_count();
                if col_count == 0 {
                    return output;
                }

                // Render rows
                for (i, row) in t.rows.iter().enumerate() {
                    output.push('|');
                    for cell in &row.cells {
                        let content = cell.plain_text().replace('\n', " ");
                        output.push_str(&format!(" {} |", content.trim()));
                    }
                    output.push('\n');

                    // Add separator after header row
                    if i == 0 || (t.header_rows > 0 && i == t.header_rows as usize - 1) {
                        output.push('|');
                        for cell in &row.cells {
                            let align_marker = match cell.alignment {
                                crate::model::Alignment::Left => " --- |",
                                crate::model::Alignment::Center => " :---: |",
                                crate::model::Alignment::Right => " ---: |",
                                crate::model::Alignment::Justify => " --- |",
                            };
                            output.push_str(align_marker);
                        }
                        output.push('\n');
                    }
                }
                output.push('\n');
                output
            }
            Block::Image {
                resource_id,
                alt_text,
                ..
            } => {
                let alt = alt_text.as_deref().unwrap_or("");
                let path = format!("{}{}", self.options.image_path_prefix, resource_id);
                format!("![{}]({})\n\n", alt, path)
            }
            Block::HorizontalRule => "\n---\n\n".to_string(),
            Block::PageBreak | Block::SectionBreak => "\n\n".to_string(),
            Block::Raw { content } => format!("{}\n\n", content),
        }
    }

    fn render_inline_content(&self, output: &mut String, content: &[crate::model::InlineContent]) {
        for item in content {
            match item {
                crate::model::InlineContent::Text(run) => {
                    self.render_text_run(output, run);
                }
                crate::model::InlineContent::LineBreak => {
                    if self.options.preserve_line_breaks {
                        output.push_str("  \n");
                    } else {
                        output.push(' ');
                    }
                }
                crate::model::InlineContent::Link { text, url, title } => {
                    if let Some(t) = title {
                        output.push_str(&format!("[{}]({} \"{}\")", text, url, t));
                    } else {
                        output.push_str(&format!("[{}]({})", text, url));
                    }
                }
                crate::model::InlineContent::Image {
                    resource_id,
                    alt_text,
                } => {
                    let alt = alt_text.as_deref().unwrap_or("");
                    let path = format!("{}{}", self.options.image_path_prefix, resource_id);
                    output.push_str(&format!("![{}]({})", alt, path));
                }
            }
        }
    }

    fn render_text_run(&self, output: &mut String, run: &crate::model::TextRun) {
        let text = if self.options.escape_special_chars {
            escape_markdown(&run.text)
        } else {
            run.text.clone()
        };

        let styled = self.apply_text_style(&text, &run.style);
        output.push_str(&styled);
    }

    fn apply_text_style(&self, text: &str, style: &crate::model::TextStyle) -> String {
        let mut result = text.to_string();

        if style.strikethrough {
            result = format!("~~{}~~", result);
        }
        if style.italic {
            result = format!("*{}*", result);
        }
        if style.bold {
            result = format!("**{}**", result);
        }
        if style.superscript {
            result = format!("<sup>{}</sup>", result);
        }
        if style.subscript {
            result = format!("<sub>{}</sub>", result);
        }
        if style.underline {
            result = format!("<u>{}</u>", result);
        }

        result
    }

    fn render_list_item(
        &self,
        output: &mut String,
        para: &crate::model::Paragraph,
        list_info: &crate::model::ListInfo,
    ) {
        let indent = "  ".repeat(list_info.level as usize);

        let marker = match &list_info.style {
            crate::model::ListStyle::Unordered { .. } => {
                format!("{}", self.options.list_marker)
            }
            crate::model::ListStyle::Ordered { number_style, .. } => {
                let num = list_info.item_number.unwrap_or(1);
                match number_style {
                    crate::model::NumberStyle::Decimal => format!("{}.", num),
                    crate::model::NumberStyle::LowerAlpha => {
                        format!("{}.", char::from_u32('a' as u32 + num - 1).unwrap_or('a'))
                    }
                    crate::model::NumberStyle::UpperAlpha => {
                        format!("{}.", char::from_u32('A' as u32 + num - 1).unwrap_or('A'))
                    }
                    crate::model::NumberStyle::LowerRoman => {
                        format!("{}.", to_roman(num).to_lowercase())
                    }
                    crate::model::NumberStyle::UpperRoman => format!("{}.", to_roman(num)),
                }
            }
        };

        output.push_str(&indent);
        output.push_str(&marker);
        output.push(' ');
        self.render_inline_content(output, &para.content);
        output.push('\n');
    }
}

impl<'a> Iterator for StreamingRenderer<'a> {
    type Item = RenderEvent;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.state {
                StreamState::Initial => {
                    if self.options.include_frontmatter {
                        self.state = StreamState::Frontmatter;
                        return Some(RenderEvent::Frontmatter(
                            self.doc.metadata.to_yaml_frontmatter(),
                        ));
                    }
                    self.state = StreamState::DocumentStarted;
                    return Some(RenderEvent::DocumentStart {
                        metadata: self.doc.metadata.clone(),
                        page_count: self.doc.page_count(),
                    });
                }

                StreamState::Frontmatter => {
                    self.state = StreamState::DocumentStarted;
                    return Some(RenderEvent::DocumentStart {
                        metadata: self.doc.metadata.clone(),
                        page_count: self.doc.page_count(),
                    });
                }

                StreamState::DocumentStarted => {
                    // Find first page to render
                    if let Some(page_idx) = self.find_next_page(0) {
                        let page = &self.doc.pages[page_idx];
                        self.current_page_number = page.number;
                        self.state = StreamState::InPage {
                            page_index: page_idx,
                            block_index: 0,
                        };
                        return Some(RenderEvent::PageStart {
                            number: page.number,
                        });
                    } else {
                        self.state = StreamState::PagesComplete;
                    }
                }

                StreamState::InPage {
                    page_index,
                    block_index,
                } => {
                    let page = &self.doc.pages[page_index];

                    if block_index < page.elements.len() {
                        let block = &page.elements[block_index];
                        let content = self.render_block(block);
                        self.state = StreamState::InPage {
                            page_index,
                            block_index: block_index + 1,
                        };

                        // Skip empty content
                        if content.is_empty() {
                            continue;
                        }

                        return Some(RenderEvent::Block(content));
                    } else {
                        // Page complete
                        let page_num = page.number;
                        self.state = StreamState::BetweenPages {
                            next_page: page_index + 1,
                        };
                        return Some(RenderEvent::PageEnd { number: page_num });
                    }
                }

                StreamState::BetweenPages { next_page } => {
                    if let Some(page_idx) = self.find_next_page(next_page) {
                        let page = &self.doc.pages[page_idx];
                        self.current_page_number = page.number;
                        self.state = StreamState::InPage {
                            page_index: page_idx,
                            block_index: 0,
                        };
                        return Some(RenderEvent::PageStart {
                            number: page.number,
                        });
                    } else {
                        self.state = StreamState::PagesComplete;
                    }
                }

                StreamState::PagesComplete => {
                    self.state = StreamState::Done;
                    return Some(RenderEvent::DocumentEnd);
                }

                StreamState::Done => {
                    return None;
                }
            }
        }
    }
}

/// Escape special Markdown characters.
fn escape_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '|' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

/// Convert number to Roman numerals.
fn to_roman(mut num: u32) -> String {
    let numerals = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];

    let mut result = String::new();
    for (value, symbol) in numerals {
        while num >= value {
            result.push_str(symbol);
            num -= value;
        }
    }
    result
}

/// Collect all content from a streaming renderer into a single string.
pub fn collect_content(renderer: StreamingRenderer<'_>) -> String {
    let mut output = String::new();
    for event in renderer {
        if let Some(content) = event.content() {
            output.push_str(content);
        }
    }
    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Paragraph};

    #[test]
    fn test_streaming_renderer_empty_doc() {
        let doc = Document::new();
        let renderer = StreamingRenderer::new(&doc, RenderOptions::default());
        let events: Vec<_> = renderer.collect();

        assert!(events.len() >= 2); // At least DocumentStart and DocumentEnd
        assert!(matches!(
            events.first(),
            Some(RenderEvent::DocumentStart { .. })
        ));
        assert!(matches!(events.last(), Some(RenderEvent::DocumentEnd)));
    }

    #[test]
    fn test_streaming_renderer_with_content() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello, world!"));
        doc.add_page(page);

        let renderer = StreamingRenderer::new(&doc, RenderOptions::default());
        let events: Vec<_> = renderer.collect();

        // Should have: DocumentStart, PageStart, Block, PageEnd, DocumentEnd
        assert!(events.len() >= 5);

        // Check for block content
        let has_content = events.iter().any(|e| {
            if let RenderEvent::Block(s) = e {
                s.contains("Hello, world!")
            } else {
                false
            }
        });
        assert!(has_content);
    }

    #[test]
    fn test_streaming_renderer_with_frontmatter() {
        let mut doc = Document::new();
        doc.metadata.title = Some("Test".to_string());
        let page = Page::letter(1);
        doc.add_page(page);

        let options = RenderOptions::default().with_frontmatter(true);
        let renderer = StreamingRenderer::new(&doc, options);
        let events: Vec<_> = renderer.collect();

        // Should start with frontmatter
        assert!(matches!(events.first(), Some(RenderEvent::Frontmatter(_))));
    }

    #[test]
    fn test_collect_content() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Test content"));
        doc.add_page(page);

        let renderer = StreamingRenderer::new(&doc, RenderOptions::default());
        let content = collect_content(renderer);

        assert!(content.contains("Test content"));
    }

    #[test]
    fn test_render_event_content() {
        let event = RenderEvent::Block("hello".to_string());
        assert!(event.has_content());
        assert_eq!(event.content(), Some("hello"));

        let event = RenderEvent::PageStart { number: 1 };
        assert!(!event.has_content());
        assert!(event.content().is_none());
    }
}
