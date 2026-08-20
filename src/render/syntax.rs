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
use crate::model::{Alignment, Table, TableRow};

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

/// Format a link/image destination for `[text](destination)` syntax, wrapping it in
/// `<...>` when it contains a character CommonMark's bare-parenthesis destination form
/// forbids. A destination with a raw space is not valid CommonMark outside `<...>` at
/// all -- `pulldown-cmark` does not even produce a `Link`/`Image` event for it, so a
/// consumer sees the brackets as literal text instead of a link (found while building
/// `unrefine`, cycle-23; `<`/`>` themselves are the only other characters the bare form
/// forbids). Both renderers build a destination from data the document held (a
/// hyperlink target, a resource id) that can legitimately contain either.
pub(super) fn format_link_destination(url: &str) -> String {
    if url.contains(' ') || url.contains(['<', '>']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
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

/// Spell a number as an uppercase Roman numeral, for Roman-numbered list markers.
pub(super) fn to_roman(mut num: u32) -> String {
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
    use crate::model::{TableCell, TableRow};

    #[test]
    fn test_format_link_destination_leaves_a_clean_destination_alone() {
        assert_eq!(format_link_destination("images/a.png"), "images/a.png");
    }

    #[test]
    fn test_format_link_destination_wraps_a_destination_containing_a_space() {
        assert_eq!(
            format_link_destination("my folder/file.png"),
            "<my folder/file.png>"
        );
    }

    #[test]
    fn test_format_link_destination_escapes_angle_brackets_inside_the_wrapper() {
        assert_eq!(format_link_destination("a<b>c d"), "<a%3Cb%3Ec d>");
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

    #[test]
    fn test_to_roman() {
        assert_eq!(to_roman(1), "I");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(9), "IX");
        assert_eq!(to_roman(14), "XIV");
        assert_eq!(to_roman(2024), "MMXXIV");
    }

    #[test]
    fn test_to_roman_zero_is_empty() {
        // No Roman numeral exists for zero; the caller numbers list items from one.
        assert_eq!(to_roman(0), "");
    }
}
