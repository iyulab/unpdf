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
use crate::model::{
    Alignment, Block, InlineContent, ListStyle, Paragraph, Table, TableCell, TableRow, TextStyle,
};
use unparser_shared::markdown::{self, emphasis_span};

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
    let style = &written_style(style);
    if !style.has_styling() {
        return text.to_string();
    }

    // The delimiters land against the neighbours only when nothing is wrapped around them;
    // inside a tag they touch `>` and `<`, which CommonMark always accepts.
    let outer_delimiter = (style.bold || style.italic || style.strikethrough) && !style.underline;
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
    if style.underline {
        styled = format!("<u>{}</u>", styled);
    }

    format!("{leading}{styled}{trailing}")
}

/// A paragraph as Markdown with the separator after it: a heading (its `#`s, then a blank
/// line), a list item (its marker, then one line break) or a paragraph (a blank line). The
/// whole-document and the streaming renderer write a paragraph through this one function, so
/// the two cannot drift.
pub(super) fn paragraph_markdown(para: &Paragraph, options: &RenderOptions) -> String {
    let mut output = String::new();
    if let Some(level) = para.style.heading_level {
        output.push_str(&"#".repeat(level.min(options.max_heading_level) as usize));
        output.push(' ');
        push_inline_content(&mut output, &para.content, options);
        output.push_str("\n\n");
    } else if let Some(list_info) = &para.style.list_info {
        output.push_str(&"  ".repeat(list_info.level as usize));
        match &list_info.style {
            ListStyle::Unordered { .. } => output.push(options.list_marker),
            ListStyle::Ordered { number_style, .. } => {
                let number = list_info.item_number.unwrap_or(1);
                output.push_str(&number_style.format_number(number));
                output.push('.');
            }
        }
        output.push(' ');
        push_body(&mut output, &para.content, options);
        output.push('\n');
    } else {
        push_body(&mut output, &para.content, options);
        output.push_str("\n\n");
    }
    output
}

/// Append the inline content of a paragraph or a list item. The block's own markers are the
/// renderer's; text that merely starts like one (`1. `, `* `, `# ` on a line of a paragraph
/// the layout did not take for a list or a heading) is the document's and stays text.
fn push_body(output: &mut String, content: &[InlineContent], options: &RenderOptions) {
    if !options.escape_special_chars {
        return push_inline_content(output, content, options);
    }
    let mut body = String::new();
    push_inline_content(&mut body, content, options);
    output.push_str(&markdown::escape_block_starts(&body));
}

/// The part of `style` Markdown writes.
///
/// A PDF says nothing of scripts: a superscript or subscript is read off the geometry of a
/// smaller, raised or lowered run, and the model keeps that reading (the JSON output carries
/// it). Written into the text as `<sup>`/`<sub>` it splits words that every reference reader
/// extracts whole -- `S<sub>K</sub>` for `SK`, `기준금리<sup>1)</sup>` -- and both public
/// benchmarks this crate is measured on lose matches by it. So Markdown writes a script's text
/// as the line's.
fn written_style(style: &TextStyle) -> TextStyle {
    TextStyle {
        superscript: false,
        subscript: false,
        ..style.clone()
    }
}

/// The marks Markdown writes for `style` -- what two runs must share to be written as one.
fn written_marks(style: &TextStyle) -> (bool, bool, bool, bool) {
    (
        style.bold,
        style.italic,
        style.underline,
        style.strikethrough,
    )
}

/// `content` with each run of text joined to the one before it when Markdown writes both the
/// same way: an italic `c` and an italic subscript `p` are one italic `cp`, not `*c**p*`, which
/// CommonMark does not read as two emphases.
fn coalesce_written_runs(content: &[InlineContent]) -> Vec<InlineContent> {
    let mut out: Vec<InlineContent> = Vec::with_capacity(content.len());
    for item in content {
        let plain = match item {
            InlineContent::Link {
                text, url, style, ..
            } if !active_link(url) => Some(InlineContent::Text(crate::model::TextRun {
                text: text.clone(),
                style: style.clone(),
            })),
            _ => None,
        };
        let item = plain.as_ref().unwrap_or(item);
        if let (InlineContent::Text(run), Some(InlineContent::Text(last))) = (item, out.last_mut())
        {
            if written_marks(&run.style) == written_marks(&last.style) {
                last.text.push_str(&run.text);
                continue;
            }
        }
        out.push(item.clone());
    }
    out
}

/// Append a paragraph's inline content.
fn push_inline_content(output: &mut String, content: &[InlineContent], options: &RenderOptions) {
    push_inline_content_in(output, content, options, false);
}

fn push_inline_content_in(
    output: &mut String,
    content: &[InlineContent],
    options: &RenderOptions,
    cell: bool,
) {
    let content = &coalesce_written_runs(content);
    let mut i = 0;
    while i < content.len() {
        if let Some(count) = linked_emphasis_group(content, i) {
            let before = output.chars().next_back();
            let after = content.get(i + count).and_then(leading_char);
            if before.is_none_or(|c| !c.is_alphanumeric())
                && after.is_none_or(|c| !c.is_alphanumeric())
            {
                output.push_str(&linked_emphasis(
                    &content[i..i + count],
                    before,
                    after,
                    options,
                    cell,
                ));
                i += count;
                continue;
            }
        }
        let item = &content[i];
        match item {
            InlineContent::Text(run) => {
                let text = if options.escape_special_chars {
                    escape_markdown(&run.text)
                } else {
                    run.text.clone()
                };
                let after = content.get(i + 1).and_then(leading_char);
                output.push_str(&apply_text_style(
                    &text,
                    &run.style,
                    output.chars().next_back(),
                    after,
                ));
            }
            InlineContent::LineBreak => {
                if options.preserve_line_breaks {
                    output.push_str("  \n");
                } else {
                    output.push(' ');
                }
            }
            InlineContent::Link {
                text,
                url,
                title,
                style,
            } => {
                let label = if options.escape_special_chars {
                    escape_markdown(text)
                } else {
                    text.clone()
                };
                if active_link(url) {
                    let label = apply_text_style(&label, style, None, None);
                    output.push_str(&markdown::link(&label, url, title.as_deref(), cell));
                } else {
                    let after = content.get(i + 1).and_then(leading_char);
                    output.push_str(&apply_text_style(
                        &label,
                        style,
                        output.chars().next_back(),
                        after,
                    ));
                }
            }
            InlineContent::Image {
                resource_id,
                alt_text,
            } => {
                let alt = alt_text.as_deref().unwrap_or("");
                let path = format!("{}{}", options.image_path_prefix, resource_id);
                output.push_str(&markdown::image(alt, &path, cell));
            }
        }
        i += 1;
    }
}

/// How many items from `start` are one emphasis with a link inside it: text, a link, text, ...
/// written in the same style, beginning and ending in text. `(` , a linked citation and `)` in
/// one italic is `*([Name 2007](url))*`; written run by run it is `*(*[*Name 2007*](url)*)*`,
/// three emphases for one. `None` when the items are not such a group.
fn linked_emphasis_group(content: &[InlineContent], start: usize) -> Option<usize> {
    let marks_of = |item: &InlineContent| match item {
        InlineContent::Text(run) => Some(written_marks(&run.style)),
        InlineContent::Link { style, .. } => Some(written_marks(style)),
        _ => None,
    };
    if !matches!(content.get(start)?, InlineContent::Text(_)) {
        return None;
    }
    let marks = marks_of(&content[start])?;
    if marks == (false, false, false, false) {
        return None;
    }
    let mut count = 1;
    while content.get(start + count).and_then(marks_of) == Some(marks) {
        count += 1;
    }
    // The group ends in text: a link at the edge would put the delimiter against `)`.
    while count > 0 && !matches!(content[start + count - 1], InlineContent::Text(_)) {
        count -= 1;
    }
    let active_links = content[start..start + count]
        .iter()
        .filter(|item| matches!(item, InlineContent::Link { url, .. } if active_link(url)))
        .count();
    (active_links > 0).then_some(count)
}

/// The items of a [`linked_emphasis_group`] as Markdown: the text and links written plain, the
/// whole wrapped once in the group's style.
fn linked_emphasis(
    group: &[InlineContent],
    before: Option<char>,
    after: Option<char>,
    options: &RenderOptions,
    cell: bool,
) -> String {
    let escaped = |text: &str| {
        if options.escape_special_chars {
            escape_markdown(text)
        } else {
            text.to_string()
        }
    };
    let mut inner = String::new();
    let mut style = TextStyle::default();
    for (n, item) in group.iter().enumerate() {
        match item {
            InlineContent::Text(run) => {
                if n == 0 {
                    style = run.style.clone();
                }
                inner.push_str(&escaped(&run.text));
            }
            InlineContent::Link {
                text, url, title, ..
            } => {
                let label = escaped(text);
                if active_link(url) {
                    inner.push_str(&markdown::link(&label, url, title.as_deref(), cell));
                } else {
                    inner.push_str(&label);
                }
            }
            // Never in a group (`linked_emphasis_group` stops at them).
            InlineContent::LineBreak | InlineContent::Image { .. } => {}
        }
    }
    apply_text_style(&inner, &style, before, after)
}

/// The first character `item` writes — what a styled run just before it lands against.
pub(super) fn leading_char(item: &InlineContent) -> Option<char> {
    match item {
        InlineContent::Text(run) => run.text.chars().next(),
        InlineContent::LineBreak => Some('\n'),
        InlineContent::Link { text, url, .. } => {
            if active_link(url) {
                Some('[')
            } else {
                text.chars().next()
            }
        }
        InlineContent::Image { .. } => Some('!'),
    }
}

fn active_link(target: &str) -> bool {
    if target.is_empty()
        || target.starts_with("\\\\")
        || target.starts_with("//")
        || target.trim() != target
        || target.chars().any(char::is_control)
    {
        return false;
    }
    let Some(colon) = target
        .find(':')
        .filter(|&colon| target.find(['/', '?', '#']).is_none_or(|end| colon < end))
    else {
        return true;
    };
    ["http", "https", "ftp", "mailto", "tel"]
        .into_iter()
        .any(|scheme| target[..colon].eq_ignore_ascii_case(scheme))
}

fn cell_has_links(cell: &TableCell) -> bool {
    cell.content
        .iter()
        .flat_map(|p| &p.content)
        .any(|i| matches!(i, InlineContent::Link { .. }))
}

fn markdown_cell(cell: &TableCell, options: &RenderOptions) -> String {
    let mut out = String::new();
    for (i, paragraph) in cell.content.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if let Some(marker) = paragraph.list_marker() {
            out.push_str(&if options.escape_special_chars {
                escape_markdown(&marker)
            } else {
                marker
            });
        }
        push_inline_content_in(&mut out, &paragraph.content, options, true);
    }
    out.replace('\n', " ").trim().to_string()
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
        render_table_markdown(table, options)
    }
}

/// A table as Markdown. Markdown has no merged cells: a merged cell's text is in the position
/// it starts at and the positions it covers are empty, as in [`Table::to_csv`], so every row
/// is as wide as the table and each value stays under its heading.
fn render_table_markdown(table: &Table, options: &RenderOptions) -> String {
    let width = table.column_count();
    if width == 0 {
        return String::new();
    }

    let mut output = String::new();
    for (i, (row, starts)) in table.rows.iter().zip(table.cell_columns()).enumerate() {
        let mut slots: Vec<Option<&TableCell>> = vec![None; width];
        for (cell, c) in row.cells.iter().zip(starts) {
            if let Some(slot) = slots.get_mut(c) {
                *slot = Some(cell);
            }
        }
        output.push('|');
        for slot in &slots {
            if let Some(cell) = slot.filter(|cell| cell_has_links(cell)) {
                let content = markdown_cell(cell, options);
                output.push_str(&format!(" {content} |"));
                continue;
            }
            let content = slot.map_or(String::new(), |cell| cell.plain_text().replace('\n', " "));
            // A cell's text is the document's: a `|` in it would end the cell, a `*` start
            // emphasis. Escaped like a paragraph's (`escape_special_chars`), `|` included.
            let content = content.trim();
            let content = if options.escape_special_chars {
                escape_cell(content)
            } else {
                content.to_string()
            };
            output.push_str(&format!(" {content} |"));
        }
        output.push('\n');

        // Add separator after header row
        if i == 0 || (table.header_rows > 0 && i == table.header_rows as usize - 1) {
            output.push('|');
            for slot in &slots {
                let align_marker = match slot.map_or(Alignment::Left, |cell| cell.alignment) {
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

        // HTML, whatever `escape_special_chars` says: a `<` or `&` in a cell's text is markup
        // to the reader of an HTML table, and the text is not.
        let content = if cell_has_links(cell) {
            html_cell(cell)
        } else {
            escape_html(&cell.plain_text())
        };
        output.push_str(&format!("<{}{}>", tag, attrs));
        output.push_str(&content);
        output.push_str(&format!("</{}>", tag));
    }

    output.push_str("</tr>\n");
}

fn html_style(text: &str, style: &TextStyle) -> String {
    let mut text = escape_html(text);
    for (enabled, tag) in [
        (style.strikethrough, "del"),
        (style.italic, "em"),
        (style.bold, "strong"),
        (style.underline, "u"),
        (style.superscript, "sup"),
        (style.subscript, "sub"),
    ] {
        if enabled {
            text = format!("<{tag}>{text}</{tag}>");
        }
    }
    text
}

fn html_attribute(text: &str) -> String {
    escape_html(text).replace('"', "&quot;")
}

fn html_cell(cell: &TableCell) -> String {
    let mut output = String::new();
    for (i, paragraph) in cell.content.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        if let Some(marker) = paragraph.list_marker() {
            output.push_str(&escape_html(&marker));
        }
        for inline in &paragraph.content {
            match inline {
                InlineContent::Text(run) => output.push_str(&html_style(&run.text, &run.style)),
                InlineContent::Link {
                    text,
                    url,
                    title,
                    style,
                } => {
                    let label = html_style(text, style);
                    if active_link(url) {
                        output.push_str(&format!("<a href=\"{}\"", html_attribute(url)));
                        if let Some(title) = title {
                            output.push_str(&format!(" title=\"{}\"", html_attribute(title)));
                        }
                        output.push('>');
                        output.push_str(&label);
                        output.push_str("</a>");
                    } else {
                        output.push_str(&label);
                    }
                }
                InlineContent::LineBreak => output.push_str("<br>"),
                InlineContent::Image { alt_text, .. } => {
                    output.push_str(&escape_html(alt_text.as_deref().unwrap_or("")))
                }
            }
        }
    }
    output
}

/// A cell's text in a Markdown table, escaped like a paragraph's -- except `*` and `_`. A cell
/// is a whole inline context, so they are escaped only when some of them could pair into
/// emphasis ([`markdown::emphasis_can_pair`]): the significance stars of `0.039***` are read as
/// stars as they are, while `x*y*z` would lose its stars to an italic `y`.
fn escape_cell(text: &str) -> String {
    let star = markdown::emphasis_can_pair(text, '*');
    let underscore = markdown::emphasis_can_pair(text, '_');
    // Brackets make a link only as `[text](url)` or `[text][label]`: a shortcut reference
    // `[text]` needs a definition, and the renderer writes none (a paragraph's `[` is always
    // escaped). So `1 [Reference]` stays as it is.
    let link = text.contains("](") || text.contains("][");
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let escape = match c {
            '\\' | '`' | '|' => true,
            '[' | ']' => link,
            '*' => star,
            '_' => underscore,
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    markdown::escape_html_syntax(&out)
}

/// A cell's text in an HTML table: `&`, `<` and `>` as character references.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape the characters that would otherwise be read as Markdown syntax -- and, through
/// [`markdown::escape_html_syntax`], what would be read as inline HTML or a character
/// reference.
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
    markdown::escape_html_syntax(&result)
}

/// `content` as a fenced code block: a fence of backticks longer than any run of backticks in
/// it, so no line of the code closes it (CommonMark §4.5).
pub(crate) fn fenced_code(content: &str) -> String {
    let longest = content.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{content}\n{fence}")
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
        let underline = TextStyle {
            underline: true,
            ..TextStyle::default()
        };
        assert_eq!(
            apply_text_style(",1", &underline, Some('a'), Some('b')),
            "<u>,1</u>"
        );
    }

    #[test]
    fn an_italic_script_joins_the_italic_text_beside_it() {
        let run = |text: &str, subscript: bool| {
            InlineContent::Text(crate::model::TextRun {
                text: text.to_string(),
                style: TextStyle {
                    italic: true,
                    subscript,
                    ..TextStyle::default()
                },
            })
        };
        let mut out = String::new();
        push_inline_content(
            &mut out,
            &[run("c", false), run("p", true), run("h", false)],
            &RenderOptions::default(),
        );
        assert_eq!(out, "*cph*");
    }

    fn styled_text(text: &str, italic: bool) -> InlineContent {
        InlineContent::Text(crate::model::TextRun {
            text: text.to_string(),
            style: TextStyle {
                italic,
                ..TextStyle::default()
            },
        })
    }

    fn styled_link(text: &str, url: &str, italic: bool) -> InlineContent {
        InlineContent::Link {
            text: text.to_string(),
            url: url.to_string(),
            title: None,
            style: TextStyle {
                italic,
                ..TextStyle::default()
            },
        }
    }

    fn written(content: &[InlineContent]) -> String {
        let mut out = String::new();
        push_inline_content(&mut out, content, &RenderOptions::default());
        out
    }

    #[test]
    fn a_linked_citation_in_italic_is_one_emphasis() {
        let content = [
            styled_text("as shown ", false),
            styled_text("(", true),
            styled_link("Name 2007", "https://example.org/a", true),
            styled_text(")", true),
            styled_text(" later", false),
        ];
        assert_eq!(
            written(&content),
            "as shown *([Name 2007](https://example.org/a))* later"
        );
    }

    #[test]
    fn a_link_beside_text_of_another_style_keeps_its_own_emphasis() {
        // The link is italic, the text around it is not: nothing to join.
        let content = [
            styled_text("see ", false),
            styled_link("Name", "https://example.org/a", true),
            styled_text(" now", false),
        ];
        assert_eq!(written(&content), "see [*Name*](https://example.org/a) now");
    }

    #[test]
    fn an_italic_link_at_the_edge_of_italic_text_is_not_joined() {
        // Text, link: the group would end in the link, putting the closing delimiter against `)`.
        let content = [
            styled_text("see ", true),
            styled_link("Name", "https://example.org/a", true),
        ];
        assert_eq!(written(&content), "*see* [*Name*](https://example.org/a)");
    }

    #[test]
    fn a_linked_group_against_a_word_is_left_run_by_run() {
        // A delimiter that touches a letter outside would move punctuation across the link's
        // brackets, so a group beside a word keeps the old, safe form.
        let content = [
            styled_text("word", false),
            styled_text("(", true),
            styled_link("Name", "https://example.org/a", true),
            styled_text(")", true),
        ];
        let out = written(&content);
        assert!(out.contains("[*Name*](https://example.org/a)"), "{out}");
    }

    #[test]
    fn a_script_is_written_as_its_text() {
        let sup = TextStyle {
            superscript: true,
            italic: true,
            ..TextStyle::default()
        };
        assert_eq!(apply_text_style("p", &sup, Some('h'), None), "*p*");
        let sub = TextStyle {
            subscript: true,
            ..TextStyle::default()
        };
        assert_eq!(apply_text_style("K", &sub, Some('S'), None), "K");
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

    /// A merged cell takes one position of its row and leaves the ones it covers empty: every
    /// row is as wide as the table, and the values after a merge stay under their headings.
    #[test]
    fn test_render_table_markdown_keeps_merged_rows_full_width() {
        let mut table = Table::new();
        table.header_rows = 1;
        table.add_row(TableRow::header(vec![
            TableCell::text("Item").colspan(2),
            TableCell::text("2020"),
        ]));
        table.add_row(TableRow::new(vec![
            TableCell::text("GMV").rowspan(2),
            TableCell::text("A"),
            TableCell::text("1"),
        ]));
        table.add_row(TableRow::new(vec![
            TableCell::text("B"),
            TableCell::text("2"),
        ]));
        let output = render_table(&table, &RenderOptions::default());
        assert_eq!(
            output,
            "| Item |  | 2020 |\n| --- | --- | --- |\n| GMV | A | 1 |\n|  | B | 2 |\n\n"
        );
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

    /// A cell's text is escaped like a paragraph's, its `|` included, so it stays one cell; in
    /// an HTML table, `<` and `&` are character references whatever escaping says. Text that a
    /// Markdown reader would take for HTML is escaped in a paragraph too.
    #[test]
    fn table_cells_and_html_like_text_stay_text() {
        let mut table = Table::new();
        table.add_row(TableRow::header(vec![
            TableCell::text("a | b".to_string()),
            TableCell::text("x*y*z".to_string()),
        ]));
        table.add_row(TableRow::new(vec![
            TableCell::text("Vec<String>".to_string()),
            TableCell::text("AT&amp;T".to_string()),
        ]));
        let md = render_table(&table, &RenderOptions::default());
        assert!(md.contains(r"| a \| b | x\*y\*z |"), "{md}");
        assert!(md.contains(r"| Vec\<String> | AT\&amp;T |"), "{md}");
        let mut stars = Table::new();
        stars.add_row(TableRow::new(vec![
            TableCell::text("0.039***".to_string()),
            TableCell::text("*8 ml".to_string()),
            TableCell::text("1 [Reference]".to_string()),
            TableCell::text("[x](y)".to_string()),
        ]));
        let stars_md = render_table(&stars, &RenderOptions::default());
        assert!(
            stars_md.contains(r"| 0.039*** | *8 ml | 1 [Reference] | \[x\](y) |"),
            "{stars_md}"
        );

        let mut merged = table.clone();
        merged.rows[1].cells[0].colspan = 2;
        merged.rows[1].cells.truncate(1);
        let html = render_table(
            &merged,
            &RenderOptions::default().with_table_fallback(TableFallback::Html),
        );
        assert!(
            html.contains("<td colspan=\"2\">Vec&lt;String&gt;</td>"),
            "{html}"
        );
        assert!(html.contains(">a | b</t"), "{html}");

        let options = RenderOptions::default();
        assert_eq!(
            paragraph_markdown(&Paragraph::with_text("a <b>bold</b>"), &options),
            "a \\<b>bold\\</b>\n\n"
        );
    }

    /// ... and at the start of a paragraph's line the renderer does not put a marker there:
    /// text that starts like one is the document's, and stays text. A heading's text opens
    /// no block and is left alone; with escaping off the text is written as it is.
    #[test]
    fn a_paragraph_that_starts_like_a_block_marker_stays_a_paragraph() {
        let options = RenderOptions::default();
        let md = |para: &Paragraph| paragraph_markdown(para, &options);
        assert_eq!(md(&Paragraph::with_text("1. Scope")), "1\\. Scope\n\n");
        assert_eq!(md(&Paragraph::with_text("# 5 remain")), "\\# 5 remain\n\n");
        assert_eq!(md(&Paragraph::with_text("> quoted")), "\\> quoted\n\n");
        assert_eq!(
            md(&Paragraph::with_text("2024. 10. 14")),
            "2024\\. 10. 14\n\n"
        );

        let mut heading = Paragraph::with_text("1. Introduction");
        heading.style.heading_level = Some(2);
        assert_eq!(md(&heading), "## 1. Introduction\n\n");

        let raw = RenderOptions {
            escape_special_chars: false,
            ..RenderOptions::default()
        };
        assert_eq!(
            paragraph_markdown(&Paragraph::with_text("1. Scope"), &raw),
            "1. Scope\n\n"
        );
    }
}
