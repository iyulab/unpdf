use std::sync::Arc;

use super::layout::{ScriptShift, TextBlock, TextLine, TextSpan, SCRIPT_MAX_SIZE};
use super::link::LinkTarget;
use super::text_ranges::{push_range, TaggedText, TextRange};
use crate::model::{InlineContent, Paragraph, TextRun, TextStyle};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Owner {
    style: TextStyle,
    target: Option<Arc<LinkTarget>>,
}

pub(crate) type Text = TaggedText<Owner>;

fn insert(output: &mut Text, text: &str, style: TextStyle) {
    let start = output.text.len();
    output.insert(text);
    push_range(
        &mut output.ranges,
        start..output.text.len(),
        Owner {
            style,
            target: None,
        },
    );
}

fn append_span(output: &mut Text, span: &TextSpan, script: Option<ScriptShift>, styled: bool) {
    let base = if styled {
        TextStyle {
            bold: span.is_bold,
            italic: span.is_italic,
            strikethrough: span.is_struck,
            ..Default::default()
        }
    } else {
        TextStyle::default()
    };
    let ink_start = span.text.len() - span.text.trim_start().len();
    let ink_end = span.text.trim_end().len();
    let mut at = 0;
    let mut link = 0;
    while at < span.text.len() {
        while span.links.get(link).is_some_and(|l| l.range.end <= at) {
            link += 1;
        }
        let current = span.links.get(link);
        let mut end = span.text.len();
        let target = if let Some(current) = current {
            if current.range.start <= at {
                end = end.min(current.range.end);
                Some(current.value.target.clone())
            } else {
                end = end.min(current.range.start);
                None
            }
        } else {
            None
        };
        let mut style = base.clone();
        if let Some(current) = current.filter(|current| current.range.contains(&at)) {
            style.bold = current.value.bold;
            style.italic = current.value.italic;
            style.strikethrough = current.value.strikethrough;
        }
        if styled && script.is_some() {
            if at < ink_start {
                end = end.min(ink_start);
            } else if at < ink_end {
                end = end.min(ink_end);
                style.superscript = script == Some(ScriptShift::Raised);
                style.subscript = script == Some(ScriptShift::Lowered);
            }
        }
        let Some(piece) = span.text.get(at..end).filter(|_| end > at) else {
            insert(output, &span.text[at..], base);
            return;
        };
        let start = output.text.len();
        output.text.push_str(piece);
        push_range(
            &mut output.ranges,
            start..output.text.len(),
            Owner { style, target },
        );
        at = end;
    }
}

pub(crate) fn line(line: &TextLine, styled: bool) -> Text {
    let baseline = line
        .spans
        .iter()
        .filter(|s| !s.is_rotated() && s.font_size > line.font_size * SCRIPT_MAX_SIZE)
        .max_by_key(|s| s.text.trim().chars().count())
        .map_or(line.y, |s| s.y);
    let mut output = Text::new();
    for (i, span) in line.spans.iter().enumerate() {
        if i > 0 && super::layout::should_insert_space_between(&line.spans[i - 1], span) {
            let previous = &line.spans[i - 1];
            insert(
                &mut output,
                " ",
                if styled {
                    TextStyle {
                        bold: previous.is_bold,
                        italic: previous.is_italic,
                        strikethrough: previous.is_struck,
                        ..Default::default()
                    }
                } else {
                    TextStyle::default()
                },
            );
        }
        let shift = (!span.is_rotated())
            .then(|| super::layout::script_shift(span.font_size, span.y, line.font_size, baseline))
            .flatten();
        append_span(&mut output, span, shift, styled || !span.links.is_empty());
    }
    super::bidi::reorder_tagged(output)
}

pub(crate) fn block(block: &TextBlock) -> Paragraph {
    let plain = block.text();
    if super::bidi::contains_rtl(&plain)
        && block
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| s.links.is_empty())
    {
        return Paragraph::with_text(plain);
    }
    let breaks = block.hangul_breaks_in_words();
    let mut output = Text::new();
    let mut previous = String::new();
    for (i, source) in block.lines.iter().enumerate() {
        let current = line(source, true);
        if i > 0 {
            insert(
                &mut output,
                super::layout::line_join(&previous, &current.text, breaks[i - 1]),
                TextStyle::default(),
            );
        }
        let start = if i > 0 {
            current.text.len() - current.text.trim_start().len()
        } else {
            0
        };
        let end = if i + 1 < block.lines.len() {
            current.text.trim_end().len().max(start)
        } else {
            current.text.len()
        };
        output.copy(&current.text, &current.ranges, start..end);
        previous = current.text;
    }
    paragraph(output)
}

pub(crate) fn heading(block: &TextBlock, level: u8) -> Paragraph {
    if block
        .lines
        .iter()
        .flat_map(|l| &l.spans)
        .all(|s| s.links.is_empty())
    {
        return Paragraph::heading(block.text(), level);
    }
    let mut paragraph = self::block(block);
    paragraph.style.heading_level = Some(level.clamp(1, 6));
    paragraph
}

pub(crate) fn joined_spans(spans: &[TextSpan], separator: &str, trim_each: bool) -> Text {
    let mut output = Text::new();
    for (i, span) in spans.iter().enumerate() {
        if i > 0 {
            insert(&mut output, separator, TextStyle::default());
        }
        let mut part = Text::new();
        append_span(&mut part, span, None, !span.links.is_empty());
        let (start, end) = if trim_each {
            (
                part.text.len() - part.text.trim_start().len(),
                part.text.trim_end().len(),
            )
        } else {
            (0, part.text.len())
        };
        output.copy(&part.text, &part.ranges, start..end.max(start));
    }
    super::bidi::reorder_tagged(output)
}

pub(crate) fn joined_lines(lines: &[TextLine], separator: &str, trim_each: bool) -> Text {
    let mut output = Text::new();
    for (i, source) in lines.iter().enumerate() {
        if i > 0 {
            insert(&mut output, separator, TextStyle::default());
        }
        let current = line(source, false);
        let (start, end) = if trim_each {
            (
                current.text.len() - current.text.trim_start().len(),
                current.text.trim_end().len(),
            )
        } else {
            (0, current.text.len())
        };
        output.copy(&current.text, &current.ranges, start..end.max(start));
    }
    output
}

pub(crate) fn trim_leaders(text: Text) -> Text {
    let kept = super::layout::trim_leaders(&text.text);
    let start = kept.as_ptr() as usize - text.text.as_ptr() as usize;
    text.slice(start..start + kept.len())
}

pub(crate) fn paragraph(text: Text) -> Paragraph {
    if text.ranges.is_empty() {
        return Paragraph::with_text(text.text);
    }
    let mut paragraph = Paragraph::new();
    for TextRange { range, value } in text.ranges {
        let piece = text.text[range].to_string();
        let inline = value
            .target
            .and_then(|target| target.copy_uri())
            .map(|url| InlineContent::Link {
                text: piece.clone(),
                url,
                title: None,
                style: value.style.clone(),
            })
            .unwrap_or_else(|| {
                InlineContent::Text(TextRun {
                    text: piece,
                    style: value.style,
                })
            });
        paragraph.content.push(inline);
    }
    paragraph
}

pub(crate) fn clear_links(page: &mut crate::model::Page) {
    fn clear(paragraph: &mut Paragraph) {
        for inline in &mut paragraph.content {
            if let InlineContent::Link { text, style, .. } = inline {
                *inline = InlineContent::Text(TextRun {
                    text: std::mem::take(text),
                    style: std::mem::take(style),
                });
            }
        }
    }
    for element in &mut page.elements {
        match element {
            crate::model::Block::Paragraph(paragraph) => clear(paragraph),
            crate::model::Block::Table(table) => {
                for paragraph in table
                    .rows
                    .iter_mut()
                    .flat_map(|r| &mut r.cells)
                    .flat_map(|c| &mut c.content)
                {
                    clear(paragraph);
                }
            }
            _ => {}
        }
    }
}
