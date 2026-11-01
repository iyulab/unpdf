mod common;

use unpdf::model::{Block, InlineContent, Page, Paragraph, Table, TableCell, TableRow, TextStyle};
use unpdf::render::{collect_content, StreamingRenderer, TableFallback};
use unpdf::{Document, RenderOptions};

fn document(target: &str) -> Document {
    let mut paragraph = Paragraph::new();
    paragraph.content.push(InlineContent::Link {
        text: "Label".into(),
        url: target.into(),
        title: None,
        style: TextStyle {
            bold: true,
            ..Default::default()
        },
    });
    let mut table = Table::new();
    table.add_row(TableRow::new(vec![TableCell::with_content(vec![
        paragraph.clone(),
    ])
    .colspan(2)]));
    table.add_row(TableRow::from_strings(["A", "B"]));
    let mut page = Page::letter(1);
    page.add_paragraph(paragraph);
    page.add_block(Block::Table(table));
    let mut document = Document::new();
    document.add_page(page);
    document
}

#[test]
fn executable_unknown_and_obfuscated_targets_stay_data_and_render_styled_labels() {
    for target in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,hello",
        "vbscript:foo",
        "custom:thing",
        "file:///etc/passwd",
        " javascript:alert(1)",
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
    ] {
        let document = document(target);
        let json = serde_json::to_value(&document).unwrap();
        assert_eq!(json["pages"][0]["elements"][0]["content"][0]["url"], target);
        for escape in [false, true] {
            for fallback in [TableFallback::Markdown, TableFallback::Html] {
                let options = RenderOptions {
                    escape_special_chars: escape,
                    table_fallback: fallback,
                    cleanup: None,
                    ..Default::default()
                };
                let rendered = unpdf::render::to_markdown(&document, &options).unwrap();
                assert!(!rendered.contains("]("), "{target:?}: {rendered}");
                assert!(!rendered.contains("<a "), "{target:?}: {rendered}");
                assert!(rendered.contains("**Label**"), "{rendered}");
                if fallback == TableFallback::Html {
                    assert!(rendered.contains("<strong>Label</strong>"));
                }
                let streamed = collect_content(StreamingRenderer::new(&document, options).unwrap());
                assert_eq!(rendered.trim(), streamed.trim());
            }
        }
    }
}

#[test]
fn safe_targets_including_internal_path_spaces_remain_active_in_both_renderers() {
    for target in [
        "https://example.test/a b",
        "folder name/paper.pdf",
        "../paper.pdf",
        "#section",
        "HtTpS://example.test/paper",
        "ftp://example.test/file",
        "mailto:a@example.test",
        "tel:+12345",
    ] {
        let document = document(target);
        let markdown = unpdf::render::to_markdown(&document, &RenderOptions::default()).unwrap();
        assert!(markdown.contains("[**Label**]("), "{target}: {markdown}");
        if target.contains(' ') {
            assert!(markdown.contains(&format!("(<{target}>)")), "{markdown}");
        }
        let options = RenderOptions::default().with_table_fallback(TableFallback::Html);
        let html = unpdf::render::to_markdown(&document, &options).unwrap();
        assert!(html.contains(&format!("href=\"{target}\"")), "{html}");
    }
}

#[test]
fn html_destinations_escape_attributes_and_markdown_cells_escape_pipes() {
    let html_document = document("https://example.test/?q=\"x\"&v=<tag>");
    let options = RenderOptions::default().with_table_fallback(TableFallback::Html);
    let html = unpdf::render::to_markdown(&html_document, &options).unwrap();
    assert!(
        html.contains("href=\"https://example.test/?q=&quot;x&quot;&amp;v=&lt;tag&gt;\""),
        "{html}"
    );
    let document = document("https://example.test/?q=a|b");
    let markdown = unpdf::render::to_markdown(&document, &RenderOptions::default()).unwrap();
    let row = markdown
        .lines()
        .find(|line| line.starts_with("| ") && line.contains("Label"))
        .unwrap();
    assert!(row.contains("a\\|b"), "{row}");
}

#[test]
fn unsafe_uri_annotations_keep_the_target_in_json_without_activating_it() {
    let bytes = common::page_pdf("", "/MediaBox[0 0 612 792]/Annots[<</Subtype/Link/Rect[72 697 110 709]/A<</S/URI/URI(javascript:alert(1))>>>>]",
        b"BT /F1 10 Tf 72 700 Td (Label) Tj ET");
    let document = unpdf::parse_bytes(&bytes).unwrap();
    let value = serde_json::to_value(&document).unwrap();
    assert_eq!(
        value["pages"][0]["elements"][0]["content"][0]["url"],
        "javascript:alert(1)"
    );
    let markdown = unpdf::render::to_markdown(&document, &RenderOptions::default()).unwrap();
    assert_eq!(markdown.trim(), "Label");
    assert!(!document.pages[0].link_annotations_incomplete);
}

fn cell_texts(markdown: &str) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut cells = Vec::new();
    let mut current = None;
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::TableCell) => current = Some(String::new()),
            Event::End(TagEnd::TableCell) => cells.push(current.take().unwrap()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(current) = &mut current {
                    current.push_str(&text);
                }
            }
            _ => {}
        }
    }
    cells
}

#[test]
fn review_regression_rejected_links_coalesce_as_styled_text() {
    let style = TextStyle {
        italic: true,
        ..Default::default()
    };
    let mut paragraph = Paragraph::new();
    paragraph.add_run(unpdf::model::TextRun {
        text: "a".into(),
        style: style.clone(),
    });
    paragraph.content.push(InlineContent::Link {
        text: "b".into(),
        url: "javascript:x".into(),
        title: None,
        style,
    });
    let markdown = unpdf::render::BlockRenderer::new(RenderOptions::default())
        .render_block(&Block::Paragraph(paragraph));
    let text = pulldown_cmark::Parser::new(&markdown)
        .filter_map(|event| match event {
            pulldown_cmark::Event::Text(text) => Some(text),
            _ => None,
        })
        .fold(String::new(), |mut output, text| {
            output.push_str(&text);
            output
        });
    assert_eq!(text, "ab", "{markdown}");
    assert_eq!(
        pulldown_cmark::Parser::new(&markdown)
            .filter(|event| matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis)
            ))
            .count(),
        1
    );
}

#[test]
fn review_regression_table_literal_stars_never_become_emphasis_around_a_link() {
    let mut paragraph = Paragraph::new();
    paragraph.add_text("*");
    paragraph.content.push(InlineContent::Link {
        text: "L".into(),
        url: "https://example.test".into(),
        title: None,
        style: Default::default(),
    });
    paragraph.add_text("*");
    let mut table = Table::new();
    table.add_row(TableRow::new(vec![TableCell::with_content(vec![
        paragraph,
    ])]));
    let markdown = unpdf::render::BlockRenderer::new(RenderOptions::default())
        .render_block(&Block::Table(table));
    assert_eq!(cell_texts(&markdown), ["*L*"], "{markdown}");
    assert!(
        !pulldown_cmark::Parser::new_ext(&markdown, pulldown_cmark::Options::ENABLE_TABLES).any(
            |event| matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis)
            )
        )
    );
}

#[test]
fn review_regression_table_list_markers_survive_links_in_markdown_and_html() {
    for info in [
        unpdf::model::ListInfo::numbered(0, 3),
        unpdf::model::ListInfo::bullet(0),
    ] {
        let mut paragraph = Paragraph::new();
        paragraph.style.list_info = Some(info);
        paragraph.content.push(InlineContent::Link {
            text: "L".into(),
            url: "https://example.test".into(),
            title: None,
            style: Default::default(),
        });
        let expected = paragraph.plain_text();
        let mut table = Table::new();
        table.add_row(TableRow::new(vec![TableCell::with_content(vec![
            paragraph,
        ])
        .colspan(2)]));
        for fallback in [TableFallback::Markdown, TableFallback::Html] {
            let options = RenderOptions::default().with_table_fallback(fallback);
            let markdown = unpdf::render::BlockRenderer::new(options)
                .render_block(&Block::Table(table.clone()));
            if fallback == TableFallback::Markdown {
                assert_eq!(cell_texts(&markdown)[0], expected, "{markdown}");
            } else {
                assert!(
                    markdown.contains(&format!(">{}<a ", expected.trim_end_matches('L'))),
                    "{markdown}"
                );
            }
        }
    }
}
