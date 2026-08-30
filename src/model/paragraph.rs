//! Paragraph and text-level types.

use serde::{Deserialize, Serialize};

/// A paragraph of text content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    /// Text runs in the paragraph
    pub content: Vec<InlineContent>,

    /// Paragraph style
    pub style: ParagraphStyle,
}

impl Paragraph {
    /// Create a new empty paragraph.
    pub fn new() -> Self {
        Self {
            content: Vec::new(),
            style: ParagraphStyle::default(),
        }
    }

    /// Create a paragraph with plain text.
    pub fn with_text(text: impl Into<String>) -> Self {
        let mut p = Self::new();
        p.add_text(text);
        p
    }

    /// Create a heading paragraph.
    pub fn heading(text: impl Into<String>, level: u8) -> Self {
        let mut p = Self::with_text(text);
        p.style.heading_level = Some(level.clamp(1, 6));
        p
    }

    /// Add plain text to the paragraph.
    pub fn add_text(&mut self, text: impl Into<String>) {
        self.content.push(InlineContent::Text(TextRun {
            text: text.into(),
            style: TextStyle::default(),
        }));
    }

    /// Add a styled text run.
    pub fn add_run(&mut self, run: TextRun) {
        self.content.push(InlineContent::Text(run));
    }

    /// Add a line break.
    pub fn add_line_break(&mut self) {
        self.content.push(InlineContent::LineBreak);
    }

    /// Get plain text content of the paragraph.
    ///
    /// A list item's marker is re-attached here (`"- "`, `"3. "`) rather than
    /// kept inside `content` — the parser strips it from the source text once,
    /// and every renderer that cares (this one, Markdown) re-derives its own
    /// marker spelling from `style.list_info` instead of carrying two
    /// independently-formatted copies of the same fact.
    pub fn plain_text(&self) -> String {
        let text: String = self
            .content
            .iter()
            .map(|c| match c {
                InlineContent::Text(run) => run.text.clone(),
                InlineContent::LineBreak => "\n".to_string(),
                InlineContent::Link { text, .. } => text.clone(),
                InlineContent::Image { alt_text, .. } => alt_text.clone().unwrap_or_default(),
            })
            .collect();

        match &self.style.list_info {
            Some(info) => {
                let marker = match &info.style {
                    ListStyle::Unordered { marker } => format!("{marker} "),
                    ListStyle::Ordered { number_style, .. } => {
                        format!(
                            "{}. ",
                            number_style.format_number(info.item_number.unwrap_or(1))
                        )
                    }
                };
                format!("{marker}{text}")
            }
            None => text,
        }
    }

    /// Check if the paragraph is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty() || self.plain_text().trim().is_empty()
    }

    /// Check if this is a heading.
    pub fn is_heading(&self) -> bool {
        self.style.heading_level.is_some()
    }

    /// Get the heading level (1-6) or None.
    pub fn heading_level(&self) -> Option<u8> {
        self.style.heading_level
    }

    /// Check if this is a list item.
    pub fn is_list_item(&self) -> bool {
        self.style.list_info.is_some()
    }
}

impl Default for Paragraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Inline content within a paragraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InlineContent {
    /// A text run with styling
    Text(TextRun),

    /// A line break
    LineBreak,

    /// A hyperlink
    Link {
        /// Link text
        text: String,
        /// Link URL
        url: String,
        /// Link title (tooltip)
        title: Option<String>,
    },

    /// An inline image
    Image {
        /// Resource ID
        resource_id: String,
        /// Alternative text
        alt_text: Option<String>,
    },
}

/// A run of text with consistent styling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    /// The text content
    pub text: String,

    /// Text styling
    pub style: TextStyle,
}

impl TextRun {
    /// Create a new text run with default style.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::default(),
        }
    }

    /// Create a bold text run.
    pub fn bold(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle {
                bold: true,
                ..Default::default()
            },
        }
    }

    /// Create an italic text run.
    pub fn italic(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle {
                italic: true,
                ..Default::default()
            },
        }
    }

    /// Check if this run is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Text styling properties.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextStyle {
    /// Bold text
    pub bold: bool,

    /// Italic text
    pub italic: bool,

    /// Underlined text
    pub underline: bool,

    /// Strikethrough text
    pub strikethrough: bool,

    /// Superscript
    pub superscript: bool,

    /// Subscript
    pub subscript: bool,

    /// Font name
    pub font_name: Option<String>,

    /// Font size in points
    pub font_size: Option<f32>,

    /// Text color (hex format, e.g., "#FF0000")
    pub color: Option<String>,

    /// Background/highlight color
    pub background_color: Option<String>,
}

impl TextStyle {
    /// Check if any styling is applied.
    pub fn has_styling(&self) -> bool {
        self.bold
            || self.italic
            || self.underline
            || self.strikethrough
            || self.superscript
            || self.subscript
    }
}

/// Paragraph styling properties.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParagraphStyle {
    /// Heading level (1-6) or None for normal paragraph
    pub heading_level: Option<u8>,

    /// Text alignment
    pub alignment: Alignment,

    /// Indentation level (0 = no indent)
    pub indent_level: u8,

    /// List information if this is a list item
    pub list_info: Option<ListInfo>,

    /// Line spacing multiplier (1.0 = single, 2.0 = double)
    pub line_spacing: Option<f32>,

    /// Space before paragraph in points
    pub space_before: Option<f32>,

    /// Space after paragraph in points
    pub space_after: Option<f32>,

    /// First line indent in points
    pub first_line_indent: Option<f32>,
}

/// Text alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    /// Left alignment (default)
    #[default]
    Left,
    /// Center alignment
    Center,
    /// Right alignment
    Right,
    /// Justified alignment
    Justify,
}

/// Information about a list item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListInfo {
    /// List style (ordered or unordered)
    pub style: ListStyle,

    /// Nesting level (0 = top level)
    pub level: u8,

    /// Item number for ordered lists
    pub item_number: Option<u32>,
}

impl ListInfo {
    /// Create a new bulleted list item.
    pub fn bullet(level: u8) -> Self {
        Self {
            style: ListStyle::Unordered { marker: '•' },
            level,
            item_number: None,
        }
    }

    /// Create a new numbered list item.
    pub fn numbered(level: u8, number: u32) -> Self {
        Self {
            style: ListStyle::Ordered {
                start: 1,
                number_style: NumberStyle::Decimal,
            },
            level,
            item_number: Some(number),
        }
    }
}

/// List style.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ListStyle {
    /// Ordered (numbered) list
    Ordered {
        /// Starting number
        start: u32,
        /// Number style
        number_style: NumberStyle,
    },
    /// Unordered (bulleted) list
    Unordered {
        /// Bullet character
        marker: char,
    },
}

/// Number style for ordered lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberStyle {
    /// 1, 2, 3, ...
    #[default]
    Decimal,
    /// a, b, c, ...
    LowerAlpha,
    /// A, B, C, ...
    UpperAlpha,
    /// i, ii, iii, ...
    LowerRoman,
    /// I, II, III, ...
    UpperRoman,
}

impl NumberStyle {
    /// Render `num` (1-based) as this style's label, with no trailing
    /// punctuation — `"3"`, `"c"`, `"iii"`. Shared by every renderer that
    /// needs an ordered-list label so Decimal/Alpha/Roman formatting can't
    /// drift between them.
    pub(crate) fn format_number(&self, num: u32) -> String {
        match self {
            NumberStyle::Decimal => num.to_string(),
            NumberStyle::LowerAlpha => char::from_u32('a' as u32 + num.saturating_sub(1))
                .map_or_else(|| num.to_string(), |c| c.to_string()),
            NumberStyle::UpperAlpha => char::from_u32('A' as u32 + num.saturating_sub(1))
                .map_or_else(|| num.to_string(), |c| c.to_string()),
            NumberStyle::LowerRoman => to_roman(num).to_lowercase(),
            NumberStyle::UpperRoman => to_roman(num),
        }
    }
}

/// Spell a number as an uppercase Roman numeral, for Roman-numbered list markers.
pub(crate) fn to_roman(mut num: u32) -> String {
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
    fn test_paragraph_plain_text() {
        let mut p = Paragraph::new();
        p.add_text("Hello ");
        p.add_run(TextRun::bold("world"));
        p.add_text("!");

        assert_eq!(p.plain_text(), "Hello world!");
    }

    #[test]
    fn test_plain_text_reattaches_a_bullet_marker() {
        let mut p = Paragraph::with_text("Buy milk");
        p.style.list_info = Some(ListInfo::bullet(0));
        assert_eq!(p.plain_text(), "• Buy milk");
    }

    #[test]
    fn test_plain_text_reattaches_an_ordered_marker() {
        let mut p = Paragraph::with_text("Third step");
        p.style.list_info = Some(ListInfo::numbered(0, 3));
        assert_eq!(p.plain_text(), "3. Third step");
    }

    #[test]
    fn test_format_number_decimal_alpha_and_roman() {
        assert_eq!(NumberStyle::Decimal.format_number(3), "3");
        assert_eq!(NumberStyle::LowerAlpha.format_number(3), "c");
        assert_eq!(NumberStyle::UpperAlpha.format_number(3), "C");
        assert_eq!(NumberStyle::LowerRoman.format_number(14), "xiv");
        assert_eq!(NumberStyle::UpperRoman.format_number(14), "XIV");
    }

    /// A malformed `item_number` of 0 must not panic — `format_number` is
    /// reachable from parsed (untrusted) PDF content via `detect_list_marker`,
    /// which accepts a printed "0." marker. `num - 1` would underflow a u32
    /// there; saturating instead just reuses the first letter.
    #[test]
    fn test_format_number_alpha_does_not_underflow_on_zero() {
        assert_eq!(NumberStyle::LowerAlpha.format_number(0), "a");
        assert_eq!(NumberStyle::UpperAlpha.format_number(0), "A");
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

    #[test]
    fn test_heading() {
        let h1 = Paragraph::heading("Title", 1);
        assert!(h1.is_heading());
        assert_eq!(h1.heading_level(), Some(1));
    }

    #[test]
    fn test_text_style() {
        let style = TextStyle::default();
        assert!(!style.has_styling());

        let bold_style = TextStyle {
            bold: true,
            ..Default::default()
        };
        assert!(bold_style.has_styling());
    }

    #[test]
    fn test_list_info() {
        let bullet = ListInfo::bullet(0);
        assert_eq!(bullet.level, 0);

        let numbered = ListInfo::numbered(1, 5);
        assert_eq!(numbered.item_number, Some(5));
    }
}
