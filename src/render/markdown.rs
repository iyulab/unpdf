//! Markdown rendering for PDF documents.

use crate::error::Result;
use crate::model::{
    Alignment, Block, Document, InlineContent, ListInfo, ListStyle, NumberStyle, Page, Paragraph,
    Table, TextRun, TextStyle,
};

use super::{CleanupPipeline, ExtractionStats, RenderOptions, RenderResult, TableFallback};

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

        // Apply cleanup if configured
        if let Some(ref cleanup_options) = self.options.cleanup {
            let pipeline = CleanupPipeline::new(cleanup_options.clone());
            output = pipeline.process(&output);
        }

        Ok(output.trim().to_string())
    }

    fn render_page(&mut self, output: &mut String, page: &Page) {
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
                // Optionally add page break marker
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
                    if let Some(ref t) = title {
                        output.push_str(&format!("[{}]({} \"{}\")", text, url, t));
                    } else {
                        output.push_str(&format!("[{}]({})", text, url));
                    }
                }
                InlineContent::Image {
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
        if table.is_empty() {
            return;
        }

        // Use HTML for complex tables
        if table.has_merged_cells() && self.options.table_fallback == TableFallback::Html {
            self.render_table_html(output, table);
            return;
        }

        // Standard Markdown table
        self.render_table_markdown(output, table);
    }

    fn render_table_markdown(&self, output: &mut String, table: &Table) {
        let col_count = table.column_count();
        if col_count == 0 {
            return;
        }

        // Render rows
        for (i, row) in table.rows.iter().enumerate() {
            output.push('|');
            for cell in &row.cells {
                let content = cell.plain_text().replace('\n', " ");
                output.push_str(&format!(" {} |", content.trim()));
            }
            output.push('\n');

            // Add separator after header row
            if i == 0 || (table.header_rows > 0 && i == table.header_rows as usize - 1) {
                output.push('|');
                for cell in &row.cells {
                    let align_marker = match cell.alignment {
                        Alignment::Left => " --- |",
                        Alignment::Center => " :---: |",
                        Alignment::Right => " ---: |",
                        Alignment::Justify => " --- |",
                    };
                    output.push_str(align_marker);
                }
                output.push('\n');
            }
        }

        output.push('\n');
    }

    fn render_table_html(&self, output: &mut String, table: &Table) {
        output.push_str("<table>\n");

        // Header
        if table.header_rows > 0 {
            output.push_str("<thead>\n");
            for row in table.header() {
                self.render_html_row(output, row, true);
            }
            output.push_str("</thead>\n");
        }

        // Body
        output.push_str("<tbody>\n");
        for row in table.body() {
            self.render_html_row(output, row, false);
        }
        output.push_str("</tbody>\n");

        output.push_str("</table>\n\n");
    }

    fn render_html_row(&self, output: &mut String, row: &crate::model::TableRow, is_header: bool) {
        let tag = if is_header { "th" } else { "td" };
        output.push_str("<tr>");

        for cell in &row.cells {
            let mut attrs = String::new();
            if cell.rowspan > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
            }
            if cell.colspan > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
            }

            let content = cell.plain_text();
            output.push_str(&format!("<{}{}>", tag, attrs));
            output.push_str(&content);
            output.push_str(&format!("</{}>", tag));
        }

        output.push_str("</tr>\n");
    }

    fn render_image(&self, output: &mut String, resource_id: &str, alt_text: Option<&str>) {
        let alt = alt_text.unwrap_or("");
        let path = format!("{}{}", self.options.image_path_prefix, resource_id);
        output.push_str(&format!("![{}]({})\n\n", alt, path));
    }
}

/// Escape special Markdown characters.
/// Only escape characters that could be misinterpreted as Markdown syntax.
/// We minimize escaping to improve readability of extracted text.
fn escape_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            // Core formatting that must be escaped
            '\\' | '`' | '*' | '_' |
            // Brackets for links/images, pipe for tables
            '[' | ']' | '|' => {
                result.push('\\');
                result.push(c);
            }
            // NOT escaped (only special at line start or in specific contexts):
            // '.' '-' '!' '#' '+' '>' '(' ')' '{' '}'
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
}
