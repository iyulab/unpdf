//! Markdown syntax helpers shared by the batch and streaming renderers.
//!
//! Both renderers produce Markdown from the same document model, so the rules for what has
//! to be escaped and how a number is spelled are properties of the output format, not of
//! either renderer. Kept in one place, a change to an escaping rule cannot reach one path
//! and miss the other — a divergence that is hard to notice, since it shows up only in table
//! cells and around emphasis characters.
//!
//! [`render_table`] joined this module for the same reason (found while investigating
//! `ISSUE-unpdf-20260730-205711-*`, cycle-26): `StreamingRenderer` had its own copy of the
//! plain-Markdown table body, but never looked at [`TableFallback`] at all — a document with
//! merged cells rendered as HTML via `to_markdown()` and as a plain table (silently losing the
//! merge) via `StreamingRenderer`/the CLI, depending only on which renderer happened to be
//! driving. `has_merged_cells` reads flags already on the parsed `Table`, so there is no
//! streaming-specific reason (buffering, backpressure) for the two paths to disagree — the
//! Table block is already fully materialized by the time either renderer sees it.

use super::{RenderOptions, TableFallback};
use crate::model::{Alignment, Block, InlineContent, Table, TableRow, TextStyle};
use unparser_shared::markdown::emphasis_span;

/// Whether `block` is a list item.
fn is_list_item(block: &Block) -> bool {
    matches!(block, Block::Paragraph(p) if p.style.list_info.is_some())
}

/// Whether `block` closes a list and so needs a blank line before it.
///
/// List items render with a single trailing newline (a tight list); a paragraph or heading
/// right after the last item would otherwise be read as part of it. Every Markdown writer
/// asks this one question, so the rule cannot drift between them.
pub(super) fn ends_a_list(previous: Option<&Block>, block: &Block) -> bool {
    previous.is_some_and(is_list_item) && !is_list_item(block)
}

/// Wrap `text` in the Markdown emphasis markers for `style`, keeping any
/// leading/trailing whitespace *outside* the markers. A run like `"Southampton "`
/// would otherwise render as `**Southampton **` and, abutting an italic run,
/// produce the ambiguous `***` sequence — CommonMark reads that as a single
/// bold+italic opener, corrupting both runs. Kept here so the batch and
/// streaming renderers can't drift on the rule.
///
/// `before` and `after` are the characters the run lands between (`None` at either end of
/// the inline content). A delimiter that touches punctuation on its inside and a letter on
/// its outside is not a delimiter at all in CommonMark (§6.2 flanking rules): an italic
/// `", s"` after `32` written as `32*, s*` prints its asterisks. Such punctuation, and the
/// space behind it, moves outside the markers (`32, *s*`); a run that is nothing but
/// punctuation there is written plain.
pub(super) fn apply_text_style(
    text: &str,
    style: &TextStyle,
    before: Option<char>,
    after: Option<char>,
) -> String {
    if !style.has_styling() {
        return text.to_string();
    }

    // The delimiters land against the neighbours only when nothing is wrapped around them;
    // inside a tag they touch `>` and `<`, which CommonMark always accepts.
    let outer_delimiter = (style.bold || style.italic || style.strikethrough)
        && !(style.superscript || style.subscript || style.underline);
    let span = if outer_delimiter {
        emphasis_span(text, before, after)
    } else {
        emphasis_span(text, None, None)
    };
    let Some(span) = span else {
        // Nothing to emphasise: whitespace, or punctuation between words.
        return text.to_string();
    };
    let leading = &text[..span.start];
    let trailing = &text[span.end..];
    let core = &text[span];

    // Apply styles (innermost first), matching the historical nesting order.
    let mut styled = core.to_string();
    if style.strikethrough {
        styled = format!("~~{}~~", styled);
    }
    if style.italic {
        styled = format!("*{}*", styled);
    }
    if style.bold {
        styled = format!("**{}**", styled);
    }
    if style.superscript {
        styled = format!("<sup>{}</sup>", styled);
    }
    if style.subscript {
        styled = format!("<sub>{}</sub>", styled);
    }
    if style.underline {
        styled = format!("<u>{}</u>", styled);
    }

    format!("{leading}{styled}{trailing}")
}

/// The first character `item` writes — what a styled run just before it lands against.
pub(super) fn leading_char(item: &InlineContent) -> Option<char> {
    match item {
        InlineContent::Text(run) => run.text.chars().next(),
        InlineContent::LineBreak => Some('\n'),
        InlineContent::Link { .. } => Some('['),
        InlineContent::Image { .. } => Some('!'),
    }
}

/// Render a table to Markdown, honoring [`RenderOptions::table_fallback`] for a table
/// [`Table::has_merged_cells`]. Returns an empty string for an empty table.
pub(super) fn render_table(table: &Table, options: &RenderOptions) -> String {
    if table.is_empty() {
        return String::new();
    }
    if table.has_merged_cells() && options.table_fallback == TableFallback::Html {
        render_table_html(table)
    } else {
        render_table_markdown(table)
    }
}

fn render_table_markdown(table: &Table) -> String {
    let col_count = table.column_count();
    if col_count == 0 {
        return String::new();
    }

    let mut output = String::new();
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
    output
}

fn render_table_html(table: &Table) -> String {
    let mut output = String::new();
    output.push_str("<table>\n");

    if table.header_rows > 0 {
        output.push_str("<thead>\n");
        for row in table.header() {
            render_html_row(&mut output, row, true);
        }
        output.push_str("</thead>\n");
    }

    output.push_str("<tbody>\n");
    for row in table.body() {
        render_html_row(&mut output, row, false);
    }
    output.push_str("</tbody>\n");

    output.push_str("</table>\n\n");
    output
}

fn render_html_row(output: &mut String, row: &TableRow, is_header: bool) {
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

/// Escape the characters that would otherwise be read as Markdown syntax.
pub(super) fn escape_markdown(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TableCell, TableRow};

    fn italic() -> TextStyle {
        TextStyle {
            italic: true,
            ..TextStyle::default()
        }
    }

    #[test]
    fn test_apply_text_style_moves_leading_punctuation_off_a_word() {
        assert_eq!(
            apply_text_style(", s", &italic(), Some('2'), Some(' ')),
            ", *s*"
        );
    }

    #[test]
    fn test_apply_text_style_moves_trailing_punctuation_off_a_word() {
        assert_eq!(
            apply_text_style("word.", &italic(), Some(' '), Some('x')),
            "*word*."
        );
    }

    #[test]
    fn test_apply_text_style_writes_lone_punctuation_between_words_plain() {
        assert_eq!(apply_text_style(",", &italic(), Some('8'), Some('a')), ",");
    }

    #[test]
    fn test_apply_text_style_leaves_punctuation_that_touches_space_or_the_edge_inside() {
        assert_eq!(
            apply_text_style("i.e.,", &italic(), Some(' '), Some(' ')),
            "*i.e.,*"
        );
        assert_eq!(apply_text_style("(h)", &italic(), None, None), "*(h)*");
    }

    #[test]
    fn test_apply_text_style_never_splits_a_backslash_escape() {
        assert_eq!(
            apply_text_style(r"\*x\_", &italic(), Some('a'), Some('b')),
            r"\**x*\_"
        );
    }

    #[test]
    fn test_apply_text_style_leaves_html_styles_alone() {
        let sup = TextStyle {
            superscript: true,
            ..TextStyle::default()
        };
        assert_eq!(
            apply_text_style(",1", &sup, Some('a'), Some('b')),
            "<sup>,1</sup>"
        );
    }

    #[test]
    fn test_render_table_empty_is_empty_string() {
        assert_eq!(render_table(&Table::new(), &RenderOptions::default()), "");
    }

    #[test]
    fn test_render_table_defaults_to_markdown() {
        let mut table = Table::new();
        table.add_row(TableRow::from_strings(["a", "b"]));
        let output = render_table(&table, &RenderOptions::default());
        assert!(output.contains("| a | b |"));
        assert!(!output.contains("<table>"));
    }

    #[test]
    fn test_render_table_uses_html_only_for_merged_cells_with_html_fallback() {
        let options = RenderOptions::default().with_table_fallback(TableFallback::Html);

        let mut plain = Table::new();
        plain.add_row(TableRow::from_strings(["a", "b"]));
        assert!(
            !render_table(&plain, &options).contains("<table>"),
            "a table without merged cells should stay plain Markdown even with Html fallback set"
        );

        let mut merged = Table::new();
        merged.add_row(TableRow::new(vec![TableCell::text("Merged").colspan(2)]));
        let output = render_table(&merged, &options);
        assert!(output.contains("<table>") && output.contains("colspan=\"2\""));
    }

    #[test]
    fn test_escape_markdown() {
        assert_eq!(escape_markdown("Hello *world*"), "Hello \\*world\\*");
        assert_eq!(escape_markdown("[link]"), "\\[link\\]");
        assert_eq!(escape_markdown("a | b"), "a \\| b");
        assert_eq!(escape_markdown("snake_case"), "snake\\_case");
    }

    #[test]
    fn test_escape_markdown_leaves_line_start_syntax_alone() {
        // These are only special in a position the renderer controls, and escaping them
        // mid-sentence would put backslashes into ordinary prose.
        assert_eq!(escape_markdown("1. 2 - 3 # 4 > 5"), "1. 2 - 3 # 4 > 5");
    }
}
