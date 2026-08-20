//! Markdown rendering for PDF documents.

use crate::error::Result;
use crate::model::{
    Block, Document, InlineContent, ListInfo, ListStyle, NumberStyle, Page, Paragraph, Table,
    TextRun, TextStyle,
};

use super::syntax::{escape_markdown, format_link_destination, render_table, to_roman};
use super::{CleanupPipeline, ExtractionStats, PageMarkerStyle, RenderOptions, RenderResult};

/// Convert a document to Markdown.
pub fn to_markdown(doc: &Document, options: &RenderOptions) -> Result<String> {
    let renderer = MarkdownRenderer::new(options.clone());
    renderer.render(doc)
}

/// Convert a document to Markdown with statistics.
pub fn to_markdown_with_stats(doc: &Document, options: &RenderOptions) -> Result<RenderResult> {
    let mut options = options.clone();
    options.collect_stats = true;
    let renderer = MarkdownRenderer::new(options);
    renderer.render_with_stats(doc)
}

/// Markdown renderer.
pub struct MarkdownRenderer {
    options: RenderOptions,
    stats: ExtractionStats,
}

impl MarkdownRenderer {
    /// Create a new Markdown renderer.
    pub fn new(options: RenderOptions) -> Self {
        Self {
            options,
            stats: ExtractionStats::new(),
        }
    }

    /// Render a document to Markdown.
    pub fn render(mut self, doc: &Document) -> Result<String> {
        let result = self.render_internal(doc)?;
        Ok(result)
    }

    /// Render a document to Markdown with extraction statistics.
    pub fn render_with_stats(mut self, doc: &Document) -> Result<RenderResult> {
        self.options.collect_stats = true;
        let content = self.render_internal(doc)?;

        // Count words and characters in final content
        self.stats.count_text(&content);

        Ok(RenderResult::new(content, doc.metadata.clone(), self.stats))
    }

    fn render_internal(&mut self, doc: &Document) -> Result<String> {
        let mut output = String::new();

        // Add frontmatter if requested
        if self.options.include_frontmatter {
            output.push_str(&doc.metadata.to_yaml_frontmatter());
        }

        // Render selected pages
        for page in &doc.pages {
            if self.options.page_selection.includes(page.number) {
                self.render_page(&mut output, page);
            }
        }

        // Render form fields section
        if !doc.form_fields.is_empty() {
            output.push_str("\n---\n\n");
            output.push_str("## Form Fields\n\n");
            for field in &doc.form_fields {
                let value = field.display_value();
                if value.is_empty() {
                    output.push_str(&format!("- **{}**: _(empty)_\n", field.name));
                } else {
                    output.push_str(&format!("- **{}**: {}\n", field.name, value));
                }
            }
        }

        // Apply cleanup if configured
        if let Some(ref cleanup_options) = self.options.cleanup {
            let pipeline = CleanupPipeline::new(cleanup_options.clone());
            output = pipeline.process(&output);
        }

        Ok(output.trim().to_string())
    }

    fn render_page(&mut self, output: &mut String, page: &Page) {
        if self.options.page_markers == PageMarkerStyle::Comment {
            if !output.is_empty() && !output.ends_with("\n\n") {
                output.push('\n');
            }
            output.push_str(&format!("<!-- page {} -->\n\n", page.number));
        }
        if self.options.collect_stats {
            self.stats.add_page();
        }
        for block in &page.elements {
            self.render_block(output, block);
        }
    }

    fn render_block(&mut self, output: &mut String, block: &Block) {
        match block {
            Block::Paragraph(p) => self.render_paragraph(output, p),
            Block::Table(t) => {
                if self.options.collect_stats {
                    self.stats.add_table();
                }
                self.render_table(output, t);
            }
            Block::Image {
                resource_id,
                alt_text,
                ..
            } => {
                if self.options.collect_stats {
                    self.stats.add_image();
                }
                self.render_image(output, resource_id, alt_text.as_deref());
            }
            Block::HorizontalRule => {
                if self.options.collect_stats {
                    self.stats.add_horizontal_rule();
                }
                output.push_str("\n---\n\n");
            }
            Block::PageBreak | Block::SectionBreak => {
                if !output.ends_with("\n\n") {
                    output.push_str("\n\n");
                }
            }
            Block::Raw { content } => {
                output.push_str(content);
                output.push_str("\n\n");
            }
        }
    }

    fn render_paragraph(&mut self, output: &mut String, para: &Paragraph) {
        if para.is_empty() {
            return;
        }

        // Handle headings
        if let Some(level) = para.style.heading_level {
            if self.options.collect_stats {
                self.stats.add_heading();
            }
            let level = level.min(self.options.max_heading_level);
            let prefix = "#".repeat(level as usize);
            output.push_str(&prefix);
            output.push(' ');
            self.render_inline_content(output, &para.content);
            output.push_str("\n\n");
            return;
        }

        // Handle list items
        if let Some(ref list_info) = para.style.list_info {
            if self.options.collect_stats {
                self.stats.add_list_item();
            }
            self.render_list_item(output, para, list_info);
            return;
        }

        // Normal paragraph
        if self.options.collect_stats {
            self.stats.add_paragraph();
        }
        self.render_inline_content(output, &para.content);
        output.push_str("\n\n");
    }

    fn render_list_item(&self, output: &mut String, para: &Paragraph, list_info: &ListInfo) {
        let indent = "  ".repeat(list_info.level as usize);

        let marker = match &list_info.style {
            ListStyle::Unordered { marker: _ } => {
                format!("{}", self.options.list_marker)
            }
            ListStyle::Ordered { number_style, .. } => {
                let num = list_info.item_number.unwrap_or(1);
                match number_style {
                    NumberStyle::Decimal => format!("{}.", num),
                    NumberStyle::LowerAlpha => {
                        format!("{}.", char::from_u32('a' as u32 + num - 1).unwrap_or('a'))
                    }
                    NumberStyle::UpperAlpha => {
                        format!("{}.", char::from_u32('A' as u32 + num - 1).unwrap_or('A'))
                    }
                    NumberStyle::LowerRoman => format!("{}.", to_roman(num).to_lowercase()),
                    NumberStyle::UpperRoman => format!("{}.", to_roman(num)),
                }
            }
        };

        output.push_str(&indent);
        output.push_str(&marker);
        output.push(' ');
        self.render_inline_content(output, &para.content);
        output.push('\n');
    }

    fn render_inline_content(&self, output: &mut String, content: &[InlineContent]) {
        for item in content {
            match item {
                InlineContent::Text(run) => {
                    self.render_text_run(output, run);
                }
                InlineContent::LineBreak => {
                    if self.options.preserve_line_breaks {
                        output.push_str("  \n");
                    } else {
                        output.push(' ');
                    }
                }
                InlineContent::Link { text, url, title } => {
                    let dest = format_link_destination(url);
                    if let Some(ref t) = title {
                        output.push_str(&format!("[{}]({} \"{}\")", text, dest, t));
                    } else {
                        output.push_str(&format!("[{}]({})", text, dest));
                    }
                }
                InlineContent::Image {
                    resource_id,
                    alt_text,
                } => {
                    let alt = alt_text.as_deref().unwrap_or("");
                    let path = format!("{}{}", self.options.image_path_prefix, resource_id);
                    output.push_str(&format!("![{}]({})", alt, format_link_destination(&path)));
                }
            }
        }
    }

    fn render_text_run(&self, output: &mut String, run: &TextRun) {
        let text = if self.options.escape_special_chars {
            escape_markdown(&run.text)
        } else {
            run.text.clone()
        };

        let styled = self.apply_text_style(&text, &run.style);
        output.push_str(&styled);
    }

    fn apply_text_style(&self, text: &str, style: &TextStyle) -> String {
        let mut result = text.to_string();

        // Apply styles (innermost first)
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

    fn render_table(&self, output: &mut String, table: &Table) {
        output.push_str(&render_table(table, &self.options));
    }

    fn render_image(&self, output: &mut String, resource_id: &str, alt_text: Option<&str>) {
        let alt = alt_text.unwrap_or("");
        let path = format!("{}{}", self.options.image_path_prefix, resource_id);
        output.push_str(&format!(
            "![{}]({})\n\n",
            alt,
            format_link_destination(&path)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_markdown() {
        assert_eq!(escape_markdown("Hello *world*"), "Hello \\*world\\*");
        assert_eq!(escape_markdown("[link]"), "\\[link\\]");
    }

    #[test]
    fn test_to_roman() {
        assert_eq!(to_roman(1), "I");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(9), "IX");
        assert_eq!(to_roman(14), "XIV");
        assert_eq!(to_roman(2024), "MMXXIV");
    }

    #[test]
    fn test_render_simple_paragraph() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello, world!"));
        doc.add_page(page);

        let options = RenderOptions::new();
        let result = to_markdown(&doc, &options).unwrap();
        // Exclamation mark is NOT escaped (only special with brackets for images)
        assert!(result.contains("Hello, world!"));
    }

    #[test]
    fn test_render_block_image_emits_a_real_markdown_link() {
        // Regression: `render_image` used to ignore `resource_id` (it was
        // even underscore-prefixed) and emit only an HTML comment, so a
        // library consumer calling `to_markdown()` directly never got a
        // usable image reference -- `StreamingRenderer::render_block`
        // (used by both `collect_content` and the CLI writer) already built
        // a real `![alt](path)` link from `image_path_prefix` + resource id;
        // batch had silently diverged from it.
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.elements.push(Block::Image {
            resource_id: "page1_Im1.jpg".to_string(),
            alt_text: Some("A photo".to_string()),
            width: None,
            height: None,
            x: None,
            y: None,
        });
        doc.add_page(page);

        let options = RenderOptions::new().with_image_prefix("images/");
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            result.contains("![A photo](images/page1_Im1.jpg)"),
            "expected a real image link, got:\n{result}"
        );
    }

    #[test]
    fn test_link_destination_with_a_space_is_angle_wrapped() {
        // Regression: a bare `(dest with space)` is not valid CommonMark syntax at all --
        // pulldown-cmark never produces a Link event for it, so a consumer sees the
        // brackets as literal text instead of a link (unrefine cycle-23 finding).
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        let mut para = Paragraph::new();
        para.content.push(InlineContent::Link {
            text: "doc".to_string(),
            url: "my folder/file.png".to_string(),
            title: None,
        });
        page.elements.push(Block::Paragraph(para));
        doc.add_page(page);

        let options = RenderOptions::new();
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            result.contains("[doc](<my folder/file.png>)"),
            "destination not angle-wrapped: {result:?}"
        );
    }

    #[test]
    fn test_render_heading() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::heading("Chapter 1", 1));
        doc.add_page(page);

        let options = RenderOptions::new();
        let result = to_markdown(&doc, &options).unwrap();
        assert!(result.contains("# Chapter 1"));
    }

    #[test]
    fn test_render_with_frontmatter() {
        let mut doc = Document::new();
        doc.metadata.title = Some("Test Doc".to_string());
        let page = Page::letter(1);
        doc.add_page(page);

        let options = RenderOptions::new().with_frontmatter(true);
        let result = to_markdown(&doc, &options).unwrap();
        assert!(result.contains("---"));
        assert!(result.contains("title:"));
    }

    #[test]
    fn test_page_markers_comment_inserted() {
        let mut doc = Document::new();
        let mut page1 = Page::letter(1);
        page1.add_paragraph(Paragraph::with_text("First page content"));
        doc.add_page(page1);
        let mut page2 = Page::letter(2);
        page2.add_paragraph(Paragraph::with_text("Second page content"));
        doc.add_page(page2);

        let options = RenderOptions::new().with_page_markers(PageMarkerStyle::Comment);
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            result.contains("<!-- page 1 -->"),
            "marker for page 1 missing in:\n{}",
            result
        );
        assert!(
            result.contains("<!-- page 2 -->"),
            "marker for page 2 missing in:\n{}",
            result
        );
    }

    #[test]
    fn test_page_markers_none_by_default() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        doc.add_page(page);

        let options = RenderOptions::new();
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            !result.contains("<!-- page "),
            "unexpected page marker in output:\n{}",
            result
        );
    }

    #[test]
    fn test_page_markers_precede_content() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::heading("Chapter 1", 1));
        doc.add_page(page);

        let options = RenderOptions::new().with_page_markers(PageMarkerStyle::Comment);
        let result = to_markdown(&doc, &options).unwrap();
        let marker_pos = result.find("<!-- page 1 -->").expect("marker missing");
        let heading_pos = result.find("# Chapter 1").expect("heading missing");
        assert!(marker_pos < heading_pos, "marker must precede page content");
    }

    #[test]
    fn test_page_markers_survive_cleanup() {
        use crate::render::{CleanupOptions, CleanupPreset};
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        doc.add_page(page);

        for preset in [
            CleanupPreset::Minimal,
            CleanupPreset::Standard,
            CleanupPreset::Aggressive,
        ] {
            let options = RenderOptions::new()
                .with_page_markers(PageMarkerStyle::Comment)
                .with_cleanup(CleanupOptions::from_preset(preset));
            let result = to_markdown(&doc, &options).unwrap();
            assert!(
                result.contains("<!-- page 1 -->"),
                "marker stripped by {:?} cleanup preset",
                preset
            );
        }
    }

    #[test]
    fn test_page_markers_respect_page_selection() {
        let mut doc = Document::new();
        for i in 1..=3u32 {
            let mut page = Page::letter(i);
            page.add_paragraph(Paragraph::with_text(format!("Page {}", i)));
            doc.add_page(page);
        }

        // Only render page 2
        let options = RenderOptions::new()
            .with_page_markers(PageMarkerStyle::Comment)
            .with_page_list(vec![2]);
        let result = to_markdown(&doc, &options).unwrap();

        assert!(
            !result.contains("<!-- page 1 -->"),
            "page 1 marker must be absent when filtered"
        );
        assert!(
            result.contains("<!-- page 2 -->"),
            "page 2 marker must be present"
        );
        assert!(
            !result.contains("<!-- page 3 -->"),
            "page 3 marker must be absent when filtered"
        );
    }

    #[test]
    fn test_page_markers_after_frontmatter() {
        let mut doc = Document::new();
        doc.metadata.title = Some("Test".to_string());
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Content"));
        doc.add_page(page);

        let options = RenderOptions::new()
            .with_frontmatter(true)
            .with_page_markers(PageMarkerStyle::Comment);
        let result = to_markdown(&doc, &options).unwrap();
        assert!(
            result.contains("<!-- page 1 -->"),
            "marker missing:\n{}",
            result
        );
        // marker must appear after frontmatter closing ---
        let second_dashes = result
            .find("---")
            .and_then(|i| result[i + 3..].find("---").map(|j| i + 3 + j + 3))
            .expect("frontmatter not found");
        let marker_pos = result.find("<!-- page 1 -->").expect("marker not found");
        assert!(
            marker_pos > second_dashes,
            "marker must appear after frontmatter"
        );
    }
}
