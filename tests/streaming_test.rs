//! Integration tests for streaming renderer.

use unpdf::model::{Document, Page, Paragraph};
use unpdf::render::streaming::{collect_content, RenderEvent, StreamingRenderer};
use unpdf::render::{PageSelection, RenderOptions};

fn create_sample_document() -> Document {
    let mut doc = Document::new();
    doc.metadata.title = Some("Test Document".to_string());
    doc.metadata.author = Some("Test Author".to_string());

    // Page 1
    let mut page1 = Page::letter(1);
    page1.add_paragraph(Paragraph::heading("Introduction", 1));
    page1.add_paragraph(Paragraph::with_text("This is the introduction."));
    doc.add_page(page1);

    // Page 2
    let mut page2 = Page::letter(2);
    page2.add_paragraph(Paragraph::heading("Chapter 1", 1));
    page2.add_paragraph(Paragraph::with_text("Chapter content goes here."));
    doc.add_page(page2);

    // Page 3
    let mut page3 = Page::letter(3);
    page3.add_paragraph(Paragraph::with_text("Conclusion text."));
    doc.add_page(page3);

    doc
}

#[test]
fn test_streaming_renderer_basic() {
    let doc = create_sample_document();
    let renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    let events: Vec<_> = renderer.collect();

    // Should have document start and end
    assert!(matches!(
        events.first(),
        Some(RenderEvent::DocumentStart { .. })
    ));
    assert!(matches!(events.last(), Some(RenderEvent::DocumentEnd)));
}

#[test]
fn test_streaming_renderer_page_events() {
    let doc = create_sample_document();
    let renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    let events: Vec<_> = renderer.collect();

    // Count page start events
    let page_starts: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RenderEvent::PageStart { .. }))
        .collect();

    // Count page end events
    let page_ends: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RenderEvent::PageEnd { .. }))
        .collect();

    assert_eq!(page_starts.len(), 3);
    assert_eq!(page_ends.len(), 3);
}

#[test]
fn test_streaming_renderer_content() {
    let doc = create_sample_document();
    let renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    let content = collect_content(renderer);

    assert!(content.contains("Introduction"));
    assert!(content.contains("Chapter 1"));
    assert!(content.contains("Conclusion text"));
}

#[test]
fn test_streaming_renderer_with_frontmatter() {
    let doc = create_sample_document();
    let options = RenderOptions::default().with_frontmatter(true);
    let renderer = StreamingRenderer::new(&doc, options);

    let events: Vec<_> = renderer.collect();

    // First event should be frontmatter
    assert!(matches!(events.first(), Some(RenderEvent::Frontmatter(_))));

    // Check frontmatter content
    if let Some(RenderEvent::Frontmatter(content)) = events.first() {
        assert!(content.contains("title:"));
        assert!(content.contains("Test Document"));
    }
}

#[test]
fn test_streaming_renderer_page_selection() {
    let doc = create_sample_document();
    let options = RenderOptions::default().with_pages(PageSelection::Range(1..=2));
    let renderer = StreamingRenderer::new(&doc, options.clone());

    let events: Vec<_> = renderer.collect();

    // Should only have 2 page starts
    let page_starts: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RenderEvent::PageStart { .. }))
        .collect();

    assert_eq!(page_starts.len(), 2);

    // Should not contain page 3 content
    let content = collect_content(StreamingRenderer::new(&doc, options));
    assert!(content.contains("Introduction"));
    assert!(content.contains("Chapter 1"));
    assert!(!content.contains("Conclusion text"));
}

#[test]
fn test_streaming_renderer_empty_document() {
    let doc = Document::new();
    let renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    let events: Vec<_> = renderer.collect();

    // Should still have document start and end
    assert!(matches!(
        events.first(),
        Some(RenderEvent::DocumentStart { .. })
    ));
    assert!(matches!(events.last(), Some(RenderEvent::DocumentEnd)));

    // Should have no page events
    let page_events: Vec<_> = events.iter().filter(|e| e.is_page_boundary()).collect();
    assert!(page_events.is_empty());
}

#[test]
fn test_render_event_methods() {
    // Content events
    let block = RenderEvent::Block("test".to_string());
    assert!(block.has_content());
    assert_eq!(block.content(), Some("test"));
    assert!(!block.is_document_boundary());
    assert!(!block.is_page_boundary());

    let frontmatter = RenderEvent::Frontmatter("---\n---".to_string());
    assert!(frontmatter.has_content());
    assert!(frontmatter.content().is_some());

    // Boundary events
    let doc_start = RenderEvent::DocumentStart {
        metadata: Default::default(),
        page_count: 0,
    };
    assert!(!doc_start.has_content());
    assert!(doc_start.content().is_none());
    assert!(doc_start.is_document_boundary());
    assert!(!doc_start.is_page_boundary());

    let page_start = RenderEvent::PageStart { number: 1 };
    assert!(!page_start.is_document_boundary());
    assert!(page_start.is_page_boundary());
}

#[test]
fn test_streaming_renderer_is_done() {
    let doc = create_sample_document();
    let mut renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    assert!(!renderer.is_done());

    // Exhaust the iterator
    while renderer.next().is_some() {}

    assert!(renderer.is_done());
}

#[test]
fn test_streaming_renderer_page_count() {
    let doc = create_sample_document();
    let renderer = StreamingRenderer::new(&doc, RenderOptions::default());

    assert_eq!(renderer.page_count(), 3);
}
