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
//!     let renderer = StreamingRenderer::new(&doc, Default::default())?;
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

use crate::error::Result;
use crate::model::{Block, Document, Metadata};

use super::syntax::{paragraph_markdown, render_table};
use super::{PageMarkerStyle, RenderOptions};
use unparser_shared::markdown;

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
    /// Emitting page boundary marker before entering InPage
    PageMarker { page_index: usize },
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
    blocks: BlockRenderer,
    state: StreamState,
    current_page_number: u32,
}

impl<'a> StreamingRenderer<'a> {
    /// Create a new streaming renderer.
    ///
    /// Fails before yielding anything when `options.page_selection` names a page `doc`
    /// does not have, as [`to_markdown`](super::to_markdown) does — see
    /// [`PageSelection::check`](super::PageSelection::check). An iterator has no place to
    /// report it later, and honoured quietly the selection reads as an empty document.
    pub fn new(doc: &'a Document, options: RenderOptions) -> Result<Self> {
        options.page_selection.check_document(doc)?;
        Ok(Self {
            doc,
            blocks: BlockRenderer::new(options),
            state: StreamState::Initial,
            current_page_number: 0,
        })
    }

    fn options(&self) -> &RenderOptions {
        &self.blocks.options
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
        (start_index..self.doc.pages.len()).find(|&i| {
            self.options()
                .page_selection
                .includes(self.doc.pages[i].number)
        })
    }
}

/// Renders single blocks to Markdown fragments, without a document.
///
/// For consumers that drive their own page-by-page loop (a writer receiving pages as
/// they are parsed) and want exactly what [`StreamingRenderer`] and
/// [`to_markdown`](super::to_markdown) write for each block.
#[derive(Debug, Clone, Default)]
pub struct BlockRenderer {
    options: RenderOptions,
}

impl BlockRenderer {
    /// Create a block renderer with these options. Page-level options (selection,
    /// markers, frontmatter) are the caller's loop's to apply.
    pub fn new(options: RenderOptions) -> Self {
        Self { options }
    }

    /// Render `block` as it appears after `previous` on the same page, including the
    /// blank line that separates a list from the block that follows it. A consumer
    /// driving its own loop gets the same output as the renderer's iterator without
    /// having to know the rule.
    pub fn render_block_after(
        &self,
        previous: Option<&crate::model::Block>,
        block: &crate::model::Block,
    ) -> String {
        let mut content = self.render_block(block);
        if !content.is_empty() && super::syntax::ends_a_list(previous, block) {
            content.insert(0, '\n');
        }
        content
    }

    /// Render a single block to its Markdown fragment.
    pub fn render_block(&self, block: &Block) -> String {
        match block {
            Block::Paragraph(p) if p.is_empty() => String::new(),
            Block::Paragraph(p) => paragraph_markdown(p, &self.options),
            Block::Table(t) => render_table(t, &self.options),
            Block::Image {
                resource_id,
                alt_text,
                ..
            } => {
                let alt = alt_text.as_deref().unwrap_or("");
                let path = format!("{}{}", self.options.image_path_prefix, resource_id);
                format!("{}\n\n", markdown::image(alt, &path, false))
            }
            Block::HorizontalRule => "\n---\n\n".to_string(),
            Block::PageBreak | Block::SectionBreak => "\n\n".to_string(),
            Block::Code { content } => format!("{}\n\n", super::syntax::fenced_code(content)),
            Block::Raw { content } => format!("{}\n\n", content),
        }
    }
}

impl<'a> Iterator for StreamingRenderer<'a> {
    type Item = RenderEvent;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.state {
                StreamState::Initial => {
                    if self.options().include_frontmatter {
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
                        if self.options().page_markers == PageMarkerStyle::Comment {
                            self.state = StreamState::PageMarker {
                                page_index: page_idx,
                            };
                        } else {
                            self.state = StreamState::InPage {
                                page_index: page_idx,
                                block_index: 0,
                            };
                        }
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
                        let previous = block_index.checked_sub(1).map(|i| &page.elements[i]);
                        let content = self.blocks.render_block_after(previous, block);

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
                        if self.options().page_markers == PageMarkerStyle::Comment {
                            self.state = StreamState::PageMarker {
                                page_index: page_idx,
                            };
                        } else {
                            self.state = StreamState::InPage {
                                page_index: page_idx,
                                block_index: 0,
                            };
                        }
                        return Some(RenderEvent::PageStart {
                            number: page.number,
                        });
                    } else {
                        self.state = StreamState::PagesComplete;
                    }
                }

                StreamState::PageMarker { page_index } => {
                    let page = &self.doc.pages[page_index];
                    let marker = format!("<!-- page {} -->\n\n", page.number);
                    self.state = StreamState::InPage {
                        page_index,
                        block_index: 0,
                    };
                    return Some(RenderEvent::Block(marker));
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

    /// Link text and title are document data: a `]` in the text and a `"` in the title must
    /// not end the link — in the streaming renderer exactly as in `to_markdown`.
    #[test]
    fn test_link_text_and_title_stay_inside_the_link_in_both_renderers() {
        let mut para = Paragraph::new();
        para.content.push(crate::model::InlineContent::Link {
            text: "see [3] *now*".to_string(),
            url: "https://example.com".to_string(),
            title: Some(r#"say "hi""#.to_string()),
            style: Default::default(),
        });
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.elements.push(Block::Paragraph(para.clone()));
        doc.add_page(page);

        let expected = r#"[see \[3\] \*now\*](https://example.com "say \"hi\"")"#;
        let batch = crate::render::to_markdown(&doc, &RenderOptions::default()).unwrap();
        assert!(batch.contains(expected), "to_markdown: {batch:?}");
        let streamed =
            BlockRenderer::new(RenderOptions::default()).render_block(&Block::Paragraph(para));
        assert!(streamed.contains(expected), "streaming: {streamed:?}");
    }

    #[test]
    fn test_streaming_renderer_empty_doc() {
        let doc = Document::new();
        let renderer = StreamingRenderer::new(&doc, RenderOptions::default()).unwrap();
        let events: Vec<_> = renderer.collect();

        assert!(events.len() >= 2); // At least DocumentStart and DocumentEnd
        assert!(matches!(
            events.first(),
            Some(RenderEvent::DocumentStart { .. })
        ));
        assert!(matches!(events.last(), Some(RenderEvent::DocumentEnd)));
    }

    #[test]
    fn test_streaming_renderer_honors_html_table_fallback() {
        // Regression: streaming's own copy of the table body never looked at
        // `table_fallback` at all, so a merged-cell table rendered as HTML
        // via `to_markdown()` but as a plain table (silently losing the
        // merge) via `StreamingRenderer`/the CLI, depending only on which
        // renderer happened to be driving (cycle-26, ISSUE-unpdf-20260730-
        // 205711-*). Both now delegate to `syntax::render_table`.
        use crate::model::{Block, TableCell, TableRow};

        let mut table = crate::model::Table::new();
        table.add_row(TableRow::new(vec![TableCell::text("Merged").colspan(2)]));

        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.elements.push(Block::Table(table));
        doc.add_page(page);

        let options =
            RenderOptions::default().with_table_fallback(crate::render::TableFallback::Html);
        let renderer = StreamingRenderer::new(&doc, options).unwrap();
        let content = collect_content(renderer);

        assert!(
            content.contains("<table>") && content.contains("colspan=\"2\""),
            "expected HTML fallback for a merged-cell table, got:\n{content}"
        );
    }

    #[test]
    fn test_streaming_renderer_with_content() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello, world!"));
        doc.add_page(page);

        let renderer = StreamingRenderer::new(&doc, RenderOptions::default()).unwrap();
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
        let renderer = StreamingRenderer::new(&doc, options).unwrap();
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

        let renderer = StreamingRenderer::new(&doc, RenderOptions::default()).unwrap();
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

    #[test]
    fn test_streaming_renderer_emits_page_markers() {
        let mut doc = Document::new();
        let mut page1 = Page::letter(1);
        page1.add_paragraph(Paragraph::with_text("Page one"));
        doc.add_page(page1);
        let mut page2 = Page::letter(2);
        page2.add_paragraph(Paragraph::with_text("Page two"));
        doc.add_page(page2);

        let options = RenderOptions::default().with_page_markers(super::PageMarkerStyle::Comment);
        let renderer = StreamingRenderer::new(&doc, options).unwrap();
        let content = collect_content(renderer);

        assert!(
            content.contains("<!-- page 1 -->"),
            "page 1 marker missing:\n{}",
            content
        );
        assert!(
            content.contains("<!-- page 2 -->"),
            "page 2 marker missing:\n{}",
            content
        );
        let p1 = content.find("<!-- page 1 -->").unwrap();
        let p2 = content.find("<!-- page 2 -->").unwrap();
        assert!(p1 < p2, "page 1 marker must precede page 2 marker");
    }

    fn two_page_doc() -> Document {
        let mut doc = Document::new();
        for (number, text) in [(1, "Page one"), (2, "Page two")] {
            let mut page = Page::letter(number);
            page.add_paragraph(Paragraph::with_text(text));
            doc.add_page(page);
        }
        doc
    }

    #[test]
    fn streaming_renders_the_selected_pages() {
        let doc = two_page_doc();
        let options = RenderOptions::default().with_page_list(vec![2]);
        let content = collect_content(StreamingRenderer::new(&doc, options).unwrap());
        assert_eq!(content, "Page two");
    }

    /// The selection is checked before the first event, as `to_markdown` checks it — an
    /// iterator that honoured it quietly yielded a document with no pages.
    #[test]
    fn streaming_rejects_a_selection_naming_a_missing_page() {
        let doc = two_page_doc();
        for options in [
            RenderOptions::default().with_page_list(vec![1, 5]),
            RenderOptions::default().with_page_range(3..=4),
        ] {
            let batch = crate::render::to_markdown(&doc, &options).map(|_| ());
            let streamed = StreamingRenderer::new(&doc, options).map(|_| ());
            assert!(
                matches!(streamed, Err(crate::Error::PageOutOfRange(..))),
                "{streamed:?}"
            );
            assert_eq!(format!("{streamed:?}"), format!("{batch:?}"));
        }
    }

    #[test]
    fn test_streaming_renderer_no_markers_by_default() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        doc.add_page(page);

        let renderer = StreamingRenderer::new(&doc, RenderOptions::default()).unwrap();
        let content = collect_content(renderer);
        assert!(
            !content.contains("<!-- page "),
            "unexpected marker:\n{}",
            content
        );
    }
}
