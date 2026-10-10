//! Layout analysis for PDF documents.
//!
//! This module provides text extraction with position and font information,
//! enabling proper heading detection, paragraph separation, and structure analysis.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

use super::backend::{get_number_from_value, ContentOp, PdfBackend, PdfValue, ResourceScope};
use super::clip::{Bounds, ClipTracker};
use super::text_state::Shift;
use crate::error::{Error, Result};
use crate::model::UnreadableFont;

/// A text span with position and style information.
#[derive(Debug, Clone)]
pub struct TextSpan {
    /// The text content
    pub text: String,
    /// X position (left edge)
    pub x: f32,
    /// Y position (baseline)
    pub y: f32,
    /// Width of the text
    pub width: f32,
    /// Whether `width` is the run's advance from the font's glyph widths. Otherwise it is an
    /// estimate from the characters and the font size — a plausible box, too coarse to
    /// measure gaps with.
    pub width_measured: bool,
    /// Font size in points
    pub font_size: f32,
    /// Font name (e.g., "Helvetica-Bold")
    pub font_name: String,
    /// Whether the font appears to be bold
    pub is_bold: bool,
    /// Whether the font appears to be italic
    pub is_italic: bool,
}

impl TextSpan {
    /// Create a new text span.
    pub fn new(text: String, x: f32, y: f32, font_size: f32, font_name: String) -> Self {
        let is_bold = font_name.to_lowercase().contains("bold")
            || font_name.to_lowercase().contains("black")
            || font_name.to_lowercase().contains("heavy");
        let is_italic = font_name.to_lowercase().contains("italic")
            || font_name.to_lowercase().contains("oblique");

        Self {
            text,
            x,
            y,
            width: 0.0, // Will be calculated later if needed
            width_measured: false,
            font_size,
            font_name,
            is_bold,
            is_italic,
        }
    }

    /// Get the bottom Y coordinate (approximate, based on font size).
    pub fn bottom(&self) -> f32 {
        self.y - self.font_size * 0.2 // Approximate descender
    }

    /// Get the top Y coordinate (approximate, based on font size).
    pub fn top(&self) -> f32 {
        self.y + self.font_size * 0.8 // Approximate ascender
    }
}

/// A gap between two runs of one line wider than this fraction of the font size separates
/// words; narrower ones are kerning, or a change of font inside a word.
const WORD_GAP_EM: f32 = 0.15;

/// How much more room than a syllable and a space, in ems of the line's font size, a line
/// of a producer that breaks at any syllable may show — the slack of reading glyph extents.
const SYLLABLE_ROOM_SLACK: f32 = 0.25;

/// A line ending within this fraction of an em of its block's right edge runs to it.
const FLUSH_EM: f32 = 0.25;

/// Below this much room, in ems (a Hangul syllable's width), a justified column's line that
/// breaks between two syllables ends inside a word more often than not.
const IN_WORD_ROOM_EM: f32 = 0.8;

/// A text line composed of multiple spans on the same baseline.
#[derive(Debug, Clone)]
pub struct TextLine {
    /// The spans in this line, sorted by X position
    pub spans: Vec<TextSpan>,
    /// Y position (baseline)
    pub y: f32,
    /// Leftmost X position
    pub x: f32,
    /// Dominant font size in this line
    pub font_size: f32,
    /// Whether this line appears to be a heading
    pub is_heading: bool,
    /// Detected heading level (1-6, or 0 for non-heading)
    pub heading_level: u8,
}

/// True if a span's text is nothing but a run of `.` characters (a PDF-rendered
/// TOC dot leader), long enough not to be a truncated prose ellipsis. Unlike the
/// text-level regex in `render::cleanup`, this checks a single span in isolation —
/// a span with *only* dots is a far stronger signal than a dot run inside otherwise
/// mixed text, so a lower, more inclusive threshold is safe here.
fn is_dot_leader_span(span: &TextSpan) -> bool {
    let t = span.text.trim();
    t.len() >= 4 && t.chars().all(|c| c == '.')
}

/// Strip PDF-rendered TOC dot leaders directly from a line's spans, before line-
/// joining collapses the exact span boundaries a text-level regex would otherwise
/// have to guess at from whitespace alone. A run of one or more consecutive
/// dot-leader spans is dropped; if a purely-numeric span immediately follows the
/// run, it is kept and reformatted as `(p.N)` (matching `render::cleanup`'s output
/// shape) instead of being left as a bare trailing digit.
///
/// This only catches the case where the dot leader is its own span(s) — a PDF
/// backend that emits an entire "Title .... N" line as one merged span is still
/// caught downstream by the (optional) `render::cleanup` regex pass; the two are
/// complementary, not redundant.
///
/// `pub(crate)`: also used by `pdf_parser`'s low-confidence table-row fallback,
/// which builds paragraph text directly from spans without going through
/// `TextLine`.
pub(crate) fn normalize_dot_leaders(spans: Vec<TextSpan>) -> Vec<TextSpan> {
    let mut out: Vec<TextSpan> = Vec::with_capacity(spans.len());
    let mut i = 0;
    while i < spans.len() {
        if !is_dot_leader_span(&spans[i]) {
            out.push(spans[i].clone());
            i += 1;
            continue;
        }

        // Consume the whole run of consecutive dot-leader spans.
        let mut j = i + 1;
        while j < spans.len() && is_dot_leader_span(&spans[j]) {
            j += 1;
        }

        // A purely-numeric span immediately after the run is the page number.
        if let Some(page_span) = spans.get(j) {
            let digits = page_span.text.trim();
            let is_page_number = !digits.is_empty()
                && digits.len() <= 5
                && digits.chars().all(|c| c.is_ascii_digit());
            if is_page_number {
                let mut renumbered = page_span.clone();
                renumbered.text = format!("(p.{})", digits);
                out.push(renumbered);
                i = j + 1;
                continue;
            }
        }

        i = j;
    }
    out
}

/// The pitch of the lines around the gap above line `i`: the smallest of the two spacings before
/// that gap and the two after it (`spacings[j]` is the distance from line `j - 1` to line `j`),
/// or `None` when there are none.
///
/// A spacing under 0.8 of the type size is not a line pitch -- a superscript set on a line of
/// its own, a run nudged off its baseline -- and is passed over.
fn local_line_pitch(spacings: &[f32], sizes: &[f32], i: usize) -> Option<f32> {
    let floor = sizes[i] * 0.8;
    let neighbours = [i.checked_sub(2), i.checked_sub(1), Some(i + 1), Some(i + 2)];
    neighbours
        .into_iter()
        .flatten()
        .filter(|&j| j >= 1 && j != i)
        .filter_map(|j| spacings.get(j).copied())
        .filter(|&s| s >= floor)
        .min_by(f32::total_cmp)
}

/// The size a line of faux small capitals is set at, or `None` for any other line.
///
/// A word processor without a small-caps face fakes one: each word's initial at the type size
/// and the rest of the word as capitals a few points smaller ("R" 14 pt + "ECOLLECTION" 11 pt).
/// Weighted by characters, such a line reads as the smaller size -- on a page whose body is set
/// at that size, a title that reads as body text. The line is set at the size of its initials.
///
/// `spans` are in reading order. The pattern is strict, so that a line merely mixing two sizes
/// is not taken for it: no lowercase letters, exactly two sizes a capital-to-small-capital ratio
/// apart, at most one letter per word at the larger size, and every smaller run that opens with
/// a letter continuing a word whose initial ends the run before it.
fn faux_small_caps_size(spans: &[TextSpan]) -> Option<f32> {
    let big = spans.iter().map(|s| s.font_size).fold(f32::MIN, f32::max);
    let small = spans.iter().map(|s| s.font_size).fold(f32::MAX, f32::min);
    if !(big * 0.6..=big * 0.9).contains(&small) {
        return None;
    }
    let is_big = |s: &TextSpan| (s.font_size - big).abs() < 0.25;
    let is_small = |s: &TextSpan| (s.font_size - small).abs() < 0.25;
    if spans.iter().any(|s| !is_big(s) && !is_small(s))
        || spans.iter().any(|s| s.text.chars().any(char::is_lowercase))
    {
        return None;
    }
    let mut continued = 0;
    for (i, span) in spans.iter().enumerate() {
        if is_big(span) {
            let initials_only = span
                .text
                .split_whitespace()
                .all(|word| word.chars().filter(|c| c.is_alphabetic()).count() <= 1);
            if !initials_only {
                return None;
            }
        } else if span.text.chars().next().is_some_and(char::is_alphabetic) {
            let previous = &spans[i.checked_sub(1)?];
            let after_initial =
                is_big(previous) && previous.text.chars().last().is_some_and(char::is_uppercase);
            if !after_initial {
                return None;
            }
            continued += 1;
        }
    }
    (continued > 0).then_some(big)
}

impl TextLine {
    /// Create a new text line from spans.
    pub fn from_spans(mut spans: Vec<TextSpan>) -> Self {
        if spans.is_empty() {
            return Self {
                spans: vec![],
                y: 0.0,
                x: 0.0,
                font_size: 0.0,
                is_heading: false,
                heading_level: 0,
            };
        }

        // Sort spans by X position
        spans.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

        // Position is taken from the original leftmost span, before dot-leader
        // normalization — a line that turns out to be leader-only still occupies
        // its real position on the page.
        let y = spans[0].y;
        let x = spans[0].x;

        let spans = normalize_dot_leaders(spans);
        if spans.is_empty() {
            return Self {
                spans,
                y,
                x,
                font_size: 0.0,
                is_heading: false,
                heading_level: 0,
            };
        }

        // Calculate dominant font size (weighted by text length) -- except on a line of faux
        // small capitals, which is set at the size of its initials.
        let total_chars: usize = spans.iter().map(|s| s.text.len()).sum();
        let weighted_size: f32 = spans
            .iter()
            .map(|s| s.font_size * s.text.len() as f32)
            .sum();
        let font_size = if let Some(size) = faux_small_caps_size(&spans) {
            size
        } else if total_chars > 0 {
            weighted_size / total_chars as f32
        } else {
            spans[0].font_size
        };

        Self {
            spans,
            y,
            x,
            font_size,
            is_heading: false,
            heading_level: 0,
        }
    }

    /// Get the combined text of all spans with appropriate spacing.
    ///
    /// Inserts spaces between spans based on their X coordinate gaps.
    /// For CJK characters, no space is inserted between adjacent characters.
    pub fn text(&self) -> String {
        if self.spans.is_empty() {
            return String::new();
        }

        if self.spans.len() == 1 {
            return self.spans[0].text.clone();
        }

        let mut result = String::new();

        for (i, span) in self.spans.iter().enumerate() {
            if i == 0 {
                result.push_str(&span.text);
                continue;
            }

            if should_insert_space_between(&self.spans[i - 1], span) {
                result.push(' ');
            }

            result.push_str(&span.text);
        }

        // Apply BiDi reordering for RTL scripts (Arabic, Hebrew, etc.)
        if super::bidi::contains_rtl(&result) {
            result = super::bidi::reorder_bidi(&result);
        }

        result
    }

    /// The line's text broken into per-span pieces, with any gap-inserted space
    /// attached to the span that precedes it. Each entry is `(text, span_index)`,
    /// and concatenating the texts reproduces [`Self::text`] (minus BiDi
    /// reordering, which operates on the joined string). The structured parser
    /// uses this to build styled inline runs whose spacing matches `text()`.
    pub(crate) fn styled_segments(&self) -> Vec<(String, usize)> {
        let mut segs = Vec::with_capacity(self.spans.len());
        for (i, span) in self.spans.iter().enumerate() {
            if i > 0 && should_insert_space_between(&self.spans[i - 1], span) {
                segs.push((" ".to_string(), i - 1));
            }
            segs.push((span.text.clone(), i));
        }
        segs
    }

    /// Where the line's text ends on the right.
    fn right(&self) -> f32 {
        self.spans
            .iter()
            .map(|s| s.x + s.width)
            .fold(self.x, f32::max)
    }

    /// The gaps between the line's spans that separate words — wider than [`WORD_GAP_EM`]
    /// of an em — or `None` when the line's spacing cannot be read off its gaps: some span's
    /// width is an estimate, or some span holds whitespace (its producer writes spaces as
    /// glyphs, and how far those were stretched does not show).
    pub(crate) fn word_gaps(&self) -> Option<Vec<f32>> {
        if self.spans.is_empty()
            || self
                .spans
                .iter()
                .any(|s| !s.width_measured || s.text.contains(char::is_whitespace))
        {
            return None;
        }
        Some(
            self.spans
                .windows(2)
                .map(|pair| pair[1].x - (pair[0].x + pair[0].width))
                .filter(|&gap| gap > self.font_size * WORD_GAP_EM)
                .collect(),
        )
    }

    /// How wide the line's first unit of breaking is for a producer that breaks lines at any
    /// syllable: one syllable (an em) when the line starts with Hangul or another script
    /// broken anywhere, otherwise the leading run of characters that is not — a Latin word
    /// or a number, which is not broken.
    fn leading_unit_width(&self) -> f32 {
        let Some(first) = self.spans.first() else {
            return 0.0;
        };
        let breaks_anywhere = |c: char| is_hangul_char(c) || is_spaceless_script_char(c);
        let chars = first.text.chars().count().max(1);
        let lead = first
            .text
            .chars()
            .take_while(|&c| !breaks_anywhere(c) && !c.is_whitespace())
            .count();
        if lead == 0 {
            first.font_size
        } else {
            first.width * lead as f32 / chars as f32
        }
    }

    /// Check if the line is predominantly bold.
    pub fn is_bold(&self) -> bool {
        let bold_chars: usize = self
            .spans
            .iter()
            .filter(|s| s.is_bold)
            .map(|s| s.text.len())
            .sum();
        let total_chars: usize = self.spans.iter().map(|s| s.text.len()).sum();
        total_chars > 0 && bold_chars as f32 / total_chars as f32 > 0.5
    }

    /// Whether every span carrying visible text is bold — unlike [`Self::is_bold`],
    /// a bold lead-in followed by regular text does not count.
    pub fn is_all_bold(&self) -> bool {
        let mut visible = self.spans.iter().filter(|s| !s.text.trim().is_empty());
        let mut any = false;
        let all = visible.all(|s| {
            any = true;
            s.is_bold
        });
        any && all
    }

    /// Check if the line appears to be uppercase.
    pub fn is_uppercase(&self) -> bool {
        let text = self.text();
        let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
        !letters.is_empty() && letters.iter().all(|c| c.is_uppercase())
    }
}

/// A text block (paragraph, heading, etc.).
#[derive(Debug, Clone)]
pub struct TextBlock {
    /// The lines in this block
    pub lines: Vec<TextLine>,
    /// Block type
    pub block_type: BlockType,
    /// Heading level (1-6 for headings, 0 otherwise)
    pub heading_level: u8,
    /// For `BlockType::ListItem`: the item's printed number, or `None` for an
    /// unordered (bullet/enclosed-enumeration) item. Meaningless otherwise.
    pub list_item_number: Option<u32>,
    /// For `BlockType::ListItem`: bytes of `text()` that are the marker and its
    /// trailing whitespace — stripped by [`TextBlock::list_item_text`].
    list_marker_len: usize,
}

/// Type of text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// A heading (H1-H6)
    Heading,
    /// A regular paragraph
    Paragraph,
    /// A list item
    ListItem,
    /// Unknown or unclassified
    Unknown,
}

impl TextBlock {
    /// Create a new text block.
    pub fn new(lines: Vec<TextLine>, block_type: BlockType) -> Self {
        Self {
            lines,
            block_type,
            heading_level: 0,
            list_item_number: None,
            list_marker_len: 0,
        }
    }

    /// The block's lines joined as [`line_join`] says: by one space where the break is a
    /// word boundary, by nothing inside a word of a script that breaks lines anywhere.
    /// Whitespace a line ends or starts with at a join is the line break's, not text, and
    /// does not double the space.
    pub fn text(&self) -> String {
        let in_word = self.hangul_breaks_in_words();
        let texts: Vec<String> = self.lines.iter().map(TextLine::text).collect();
        let last = texts.len().saturating_sub(1);
        let mut out = String::new();
        for (i, text) in texts.iter().enumerate() {
            let piece = if i > 0 {
                text.trim_start()
            } else {
                text.as_str()
            };
            let piece = if i < last { piece.trim_end() } else { piece };
            if i > 0 {
                out.push_str(line_join(&texts[i - 1], text, in_word[i - 1]));
            }
            out.push_str(piece);
        }
        out
    }

    /// For each line break of the block (after line `i`), whether a break there between two
    /// Hangul syllables with no space falls inside a word — what [`line_join`] needs to join
    /// Korean, which uses spaces between words but may break a line at any syllable. Such a
    /// break says nothing by itself; the block has to show how its producer breaks lines:
    ///
    /// - It writes a space at the breaks between words ([`Self::marks_word_boundaries`]):
    ///   every break without one is inside a word.
    /// - It draws word gaps as moves, so its lines show how full they are
    ///   ([`Self::tight_breaks`]): in a justified column, a line that lacks less of it than
    ///   most of a syllable ends inside a word.
    pub(crate) fn hangul_breaks_in_words(&self) -> Vec<bool> {
        let breaks = self.lines.len().saturating_sub(1);
        if self.marks_word_boundaries() {
            vec![true; breaks]
        } else {
            self.tight_breaks()
        }
    }

    /// Whether the producer writes a space at the line breaks that fall between words: some
    /// line of the block ends with one, or the next starts with one. Then a break with none
    /// is inside a word.
    fn marks_word_boundaries(&self) -> bool {
        self.lines.windows(2).any(|pair| {
            let (a, b) = (pair[0].text(), pair[1].text());
            !a.trim().is_empty()
                && !b.trim().is_empty()
                && (a.ends_with(char::is_whitespace) || b.starts_with(char::is_whitespace))
        })
    }

    /// For each line break, whether the line before it is too full to end between words.
    ///
    /// This reads a justified column: its lines run to the column's right edge, and a line
    /// hands what it lacks of the column — its *room* — to its word gaps, which stretch past
    /// the block's narrowest one by that much. A producer that breaks at any syllable leaves
    /// a line less room than the next line's first syllable (or unbreakable run of Latin or
    /// digits) and a space; one that breaks only between words leaves up to a whole word.
    ///
    /// In a column of the first kind a line ending inside a word has less room than a
    /// syllable, and one ending between words keeps the dropped space in its room as well,
    /// so the two overlap but lean apart: below [`IN_WORD_ROOM_EM`] of a syllable a break is
    /// inside a word about three times in four, above it mostly between words (measured on
    /// justified Korean reports, a few hundred breaks). The break is joined below it.
    ///
    /// The block reads as such a column when most of its lines whose word gaps can be
    /// measured ([`TextLine::word_gaps`]) — two at least — run to its right edge, and hardly
    /// any of those has more room than a producer breaking at syllables would leave. Lines
    /// that stop short are not read: a paragraph's last line, where blocks hold several.
    fn tight_breaks(&self) -> Vec<bool> {
        let breaks = self.lines.len().saturating_sub(1);
        let no = vec![false; breaks];
        let gaps: Vec<Option<Vec<f32>>> = self.lines.iter().map(TextLine::word_gaps).collect();
        let Some(space) = gaps
            .iter()
            .flatten()
            .flatten()
            .copied()
            .min_by(f32::total_cmp)
        else {
            return no;
        };
        let right = self
            .lines
            .iter()
            .zip(&gaps)
            .filter(|(_, g)| g.is_some())
            .map(|(line, _)| line.right())
            .fold(f32::MIN, f32::max);
        // The room of each line before a break that runs to the right edge.
        let rooms: Vec<Option<f32>> = (0..breaks)
            .map(|i| {
                let line = &self.lines[i];
                let line_gaps = gaps[i].as_ref()?;
                (right - line.right() < line.font_size * FLUSH_EM)
                    .then(|| line_gaps.iter().map(|g| g - space).sum())
            })
            .collect();
        let measured = gaps[..breaks].iter().flatten().count();
        let flush = rooms.iter().flatten().count();
        // Lines with more room than breaking at syllables leaves. One in ten is tolerated:
        // something else on the line — a page number set on its baseline — widens a gap.
        let roomy = (0..breaks)
            .filter(|&i| {
                rooms[i].is_some_and(|room| {
                    let next = &self.lines[i + 1];
                    room >= next.leading_unit_width() + space + next.font_size * SYLLABLE_ROOM_SLACK
                })
            })
            .count();
        if flush < 2 || flush * 2 < measured || roomy * 10 > flush {
            return no;
        }
        (0..breaks)
            .map(|i| {
                rooms[i].is_some_and(|room| room < self.lines[i + 1].font_size * IN_WORD_ROOM_EM)
            })
            .collect()
    }

    /// The list item's visible text with its marker (bullet or `N.`/`N)`) and
    /// the whitespace after it stripped from the front. Meaningless when
    /// `block_type` isn't `ListItem` (returns the same as [`Self::text`]).
    pub fn list_item_text(&self) -> String {
        let full = self.text();
        full.get(self.list_marker_len..)
            .unwrap_or(&full)
            .trim_start()
            .to_string()
    }

    /// Check if the block is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() || self.text().trim().is_empty()
    }
}

/// How many of each kind of painting operator a page's content holds, forms interpreted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageOpCounts {
    /// Text-showing operators (`Tj`, `TJ`, `'`, `"`).
    pub text: u32,
    /// Image paints: `Do` of an image (or of anything that is not a form) and inline
    /// images (`BI`), including inside forms.
    pub image: u32,
    /// `Do` of a Form XObject.
    pub form: u32,
}

/// What reading the page last analysed found out about it, beyond its text — the facts a
/// consumer needs to tell a page that was read well from one that was not.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageFacts {
    /// Share of the page box painted by images (0–1): the union of every image paint's
    /// rectangle, clipped to the page box.
    pub image_coverage: f32,
    /// Text runs whose baseline is not horizontal left-to-right — rotated, vertical or
    /// upside-down text, which the reading order treats as horizontal.
    pub rotated_text_runs: u32,
    /// Ruling-line grids drawn on the page.
    pub ruled_grids: u32,
    /// Tables built from those grids.
    pub ruled_tables: u32,
    /// Text regions the reading order read independently.
    pub reading_regions: u32,
    /// The most of those regions set side by side at any height (1 for a single column).
    pub column_count: u32,
    /// Regions read line by line across although their text looked like two columns —
    /// where the reading order had to guess.
    pub ambiguous_layout_regions: u32,
}

/// Layout analyzer for extracting structured text from PDF pages.
pub struct LayoutAnalyzer<'a> {
    backend: &'a dyn PdfBackend,
    /// Font size statistics for the document
    font_stats: FontStatistics,
    /// Whether to drop an invisible OCR text layer that decodes to nothing meaningful.
    suppress_low_confidence_ocr: bool,
    /// Set when a page's text layer was dropped by that gate.
    ocr_text_suppressed: Cell<bool>,
    /// 마지막으로 분석한 페이지의 텍스트 쇼잉 오퍼레이터(Tj/TJ/'/") 수.
    /// `parse_operations` 진입 시 리셋 — 같은 페이지가 재분석돼도 최종값이 유효.
    text_op_count: Cell<u32>,
    /// 마지막으로 분석한 페이지의 이미지 XObject `Do` 호출 수 — Form XObject 안의 것 포함,
    /// Form 자체를 그리는 `Do` 는 제외(그것은 `form_op_count`).
    image_op_count: Cell<u32>,
    /// Form XObjects the page last analysed painted.
    form_op_count: Cell<u32>,
    /// Text runs the font decoder discarded on the page last analysed.
    ///
    /// Reset on entry to `parse_operations`, like the operator counts above, so a
    /// re-analysed page reports the last pass rather than the sum of every pass.
    suppressed_text_runs: Cell<usize>,
    /// The fonts those runs were discarded for, reset together with the count.
    unreadable_fonts: RefCell<Vec<UnreadableFont>>,
    /// Content streams of the page last analysed that could not be decoded.
    ///
    /// Set each time the page's content is read, so it reports the last pass.
    undecodable_content_streams: Cell<usize>,
    /// The page last analysed, as far as reading it found out. The paint facts are set by
    /// `parse_operations`, the reading-order facts each time spans are grouped into lines,
    /// so a re-analysed page reports its last pass.
    page_facts: Cell<PageFacts>,
}

/// What a page's content stream says about how its text was produced.
///
/// A searchable scan draws the page as one raster image and puts the OCR result on
/// top in rendering mode 3, which paints nothing — the text exists only to be
/// selected and searched. That combination identifies an OCR layer; whether the
/// layer is worth keeping is a separate question, answered by [`super::ocr_gate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PageTextLayerSignals {
    /// Share of characters drawn in text rendering mode 3 (invisible).
    pub invisible_char_ratio: f32,
    /// Whether an XObject was drawn covering essentially the whole page.
    pub has_page_covering_image: bool,
}

impl PageTextLayerSignals {
    /// Fraction of the page an image must cover to count as the page itself.
    const PAGE_COVERAGE: f32 = 0.9;
    /// Share of invisible characters above which the layer is not meant to be read.
    const INVISIBLE_TEXT: f32 = 0.9;

    /// Whether the page looks like a scan with an OCR text layer on top.
    pub fn is_ocr_layer_over_scan(&self) -> bool {
        self.has_page_covering_image && self.invisible_char_ratio >= Self::INVISIBLE_TEXT
    }
}

/// The depth of the section number a line opens with — `5.`/`12` → 1, `3.1.` → 2,
/// `3.2.6.` → 3, a roman `III.` → 1 — when the rest of the line reads as a title: it
/// starts with a capital (or a letter with no case) and does not end like a sentence.
///
/// Each numeric component has at most three digits, which keeps a year ("2024 Annual
/// Report") from passing for a section number.
/// Whether `text` is a section or chapter number and nothing else: `4`, `4.2`, `IV`, `A.`.
fn is_bare_section_number(text: &str) -> bool {
    let text = text.trim();
    let number = text.strip_suffix('.').unwrap_or(text);
    let arabic = !number.is_empty()
        && number.split('.').count() <= 4
        && number
            .split('.')
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()));
    let roman = !number.is_empty()
        && number.len() <= 6
        && number
            .chars()
            .all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C'));
    let letter = number.len() == 1 && number.bytes().all(|b| b.is_ascii_uppercase());
    arabic || roman || letter
}

/// Whether `number` sits just above `title`: its baseline higher by no more than two and a
/// half of the number's own font size, and starting near the title's left edge or centred
/// over it.
fn number_sits_over(number: &TextLine, title: &TextLine) -> bool {
    let gap = number.y - title.y;
    let centre = |line: &TextLine| {
        let right = line
            .spans
            .iter()
            .map(|s| s.x + s.width)
            .fold(line.x, f32::max);
        (line.x + right) / 2.0
    };
    let aligned = (number.x - title.x).abs() <= number.font_size * 2.0
        || (centre(number) - centre(title)).abs() <= number.font_size * 2.0;
    gap > 0.0 && gap <= number.font_size * 2.5 && aligned
}

/// Whether `lower` carries on the text of `upper` as the next line of one wrapped title, both
/// set at `size`: stacked one line apart, short (the second line no longer than the first), and joined mid-phrase — the lower
/// line opens in lowercase or with a bracket, or the upper one breaks after a comma, a hyphen
/// or a word that cannot end a title ("of", "and", "the" …). Lines that each open a phrase of
/// their own ("Cell one of a row" | "Cell two of a row") are siblings, not one title.
fn wraps_into(upper: &str, upper_y: f32, lower: &str, lower_y: f32, size: f32) -> bool {
    const OPEN_ENDED: [&str; 16] = [
        "a", "an", "and", "as", "at", "by", "for", "from", "in", "of", "on", "or", "the", "to",
        "with", "&",
    ];
    let gap = upper_y - lower_y;
    let stacked = gap >= size * 0.6 && gap <= size * 1.8;
    let visible = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    // A title wraps short and its second line runs no longer than its first; a body line that
    // happens to start in lowercase under a long line is a paragraph going on.
    let short = visible(upper) <= 70 && visible(lower) <= visible(upper);
    let lower_continues = lower
        .chars()
        .next()
        .is_some_and(|c| c.is_lowercase() || c == '(');
    let upper_open = upper.ends_with(',')
        || upper.ends_with('-')
        || upper
            .rsplit(char::is_whitespace)
            .next()
            .is_some_and(|w| OPEN_ENDED.contains(&w.to_lowercase().as_str()));
    stacked && short && (lower_continues || upper_open)
}

/// Whether a section number set as a line of its own sits beside `title`, on the same
/// baseline and just to its left ("3." | "RECOLLECTION OF NATIONAL INITIATIVES").
fn number_sits_beside(number: &TextLine, title: &TextLine) -> bool {
    let right = number
        .spans
        .iter()
        .map(|s| s.x + s.width)
        .fold(number.x, f32::max);
    (number.y - title.y).abs() <= number.font_size * 0.5
        && number.x < title.x
        && title.x - right <= number.font_size * 3.0
}

fn section_number_depth(text: &str) -> Option<usize> {
    let text = text.trim();
    let (number, rest) = text.split_once(char::is_whitespace)?;
    // A dash, colon or closing parenthesis may separate the number from the title, against
    // either ("01 - Introduction", "02- Methods", "3) Results").
    const SEPARATORS: [char; 5] = ['-', '–', '—', ':', ')'];
    let number = number.trim_end_matches(SEPARATORS);
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix(SEPARATORS) {
        Some(after) if after.starts_with(char::is_whitespace) => after.trim_start(),
        _ => rest,
    };

    let is_roman = number.strip_suffix('.').is_some_and(|r| {
        !r.is_empty() && r.len() <= 6 && r.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C'))
    });
    let depth = if is_roman {
        1
    } else {
        let digits = number.strip_suffix('.').unwrap_or(number);
        let parts: Vec<&str> = digits.split('.').collect();
        let numeric =
            |p: &&str| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit());
        if parts.len() > 4 || !parts.iter().all(numeric) {
            return None;
        }
        parts.len()
    };

    let first = rest.chars().next()?;
    let title_start = first.is_alphabetic() && !first.is_lowercase();
    let sentence_end = rest.ends_with(['.', ',', ';']);
    let words = rest.split_whitespace().count();
    (title_start && !sentence_end && words <= 16).then_some(depth)
}

/// Whether `line` could title what follows as a run-in title in the body face: bold
/// throughout, the body's size or a little above it, a dozen words at most, and not a sentence (it may end in a
/// colon, not a full stop). A line that opens with a number is a section title only by its
/// section number (`section_number_depth`), so it is left to that test.
fn is_bold_run_in_title(line: &TextLine, text: &str, body_size: f32) -> bool {
    let text = text.trim_end();
    // Up to the size at which a bold line is a heading by size alone
    // (`FontStatistics::get_heading_level`): a title a point or so above the body — 12pt
    // over 11pt — has nothing else to make it one.
    line.is_all_bold()
        && line.font_size >= body_size - 1.0
        && !text.starts_with(|c: char| c.is_ascii_digit())
        && text.split_whitespace().count() <= 12
        && !text.ends_with(['.', '!', '?', ',', ';'])
}

/// Whether the line after `i` is plain text of the body's size, mostly not bold — what a
/// title stands over.
fn lines_follow_as_body(sizes: &[f32], has_bold: &[bool], i: usize, body_size: f32) -> bool {
    match (sizes.get(i + 1), has_bold.get(i + 1)) {
        (Some(&size), Some(&bold)) => !bold && (size - body_size).abs() <= 1.0,
        _ => false,
    }
}

/// Whether `text` opens with a list item's glyph — a bullet or an enclosed enumeration —
/// rather than a heading or plain text.
///
/// Such a line is an enumeration inside body content no matter how large its font is, and a
/// document that renders its lists in a display face would otherwise fill the output with
/// headings. Enclosed enumerations (①, ⒈, ㈀, ❶ …) count: Korean documents use them for
/// choices and clause lists, which are the least heading-like things on the page.
///
/// A dash that signs a number is not a bullet ([`opens_with_signed_number`]): a statistical
/// table drawn as text lines puts `-0.2` at the start of a line, and read as a marker the
/// value lost its sign.
fn opens_a_list_item(text: &str) -> bool {
    let text = text.trim_start();
    text.chars().next().is_some_and(is_list_glyph) && !opens_with_signed_number(text)
}

/// Whether `text` opens with a number carrying a minus sign: a dash printed against a digit
/// (`-0.2`, `−12`, `–3`) or against a decimal point and a digit (`-.5`).
///
/// A bullet is followed by its item's words, a sign by its number, and nothing comes between
/// a sign and its number — so a dash with a space after it is a marker even before a number
/// (`- 2023.1.12 …`), and a dash against a letter is a bullet drawn without a gap
/// (`-item`). A sign drawn as a run of its own against the number joins it the same way, as
/// [`TextLine::text`] joins two runs with no gap between them.
pub(crate) fn opens_with_signed_number(text: &str) -> bool {
    let mut chars = text.trim_start().chars();
    let (Some(sign), Some(next)) = (chars.next(), chars.next()) else {
        return false;
    };
    is_minus_sign(sign)
        && (next.is_ascii_digit()
            || (next == '.' && chars.next().is_some_and(|c| c.is_ascii_digit())))
}

/// The characters a number's minus sign is printed with: the hyphen-minus, the minus sign
/// itself, the figure and en dashes typesetters set for it, and their small and fullwidth
/// forms. The em dash is not among them.
fn is_minus_sign(c: char) -> bool {
    matches!(
        c,
        '-' | '\u{2012}' | '\u{2013}' | '\u{2212}' | '\u{FE63}' | '\u{FF0D}'
    )
}

/// Whether `c` is a bullet or an enclosed-enumeration glyph — what can open a list item.
fn is_list_glyph(c: char) -> bool {
    const BULLETS: &[char] = &[
        '-', '–', '—', '*', '·', '∙', 'ㆍ', 'ㅇ', '•', '◦', '○', '●', '◎', '■', '□', '▪', '▫', '◼',
        '◾', '◆', '◇', '★', '☆', '※', '→', '▶', '►', '▷', '▹', '◁', '◀', '◃', '◂', '☞',
    ];

    BULLETS.contains(&c)
        || matches!(c,
            // Enclosed Alphanumerics: ①-⑳, ⑴-⒇, ⒈-⒛, ⓐ-ⓩ, Ⓐ-Ⓩ, ⓪, ⓫-⓾
            '\u{2460}'..='\u{24FF}'
            // Dingbats: negative and sans-serif circled digits ❶-➓
            | '\u{2776}'..='\u{2793}'
            // Enclosed CJK: parenthesized hangul ㈀-㈜, circled hangul ㉠-㉻
            | '\u{3200}'..='\u{32FF}'
        )
}

/// A list marker recognized at the very start of a line's text.
struct ListMarkerMatch {
    /// `Some(n)` for an ordered item's printed number; `None` for a bullet or
    /// enclosed-enumeration glyph ([`opens_a_list_item`]).
    ordered_number: Option<u32>,
    /// Bytes of the line's text — the marker plus the whitespace right after
    /// it — to strip before treating the rest as the item's visible content.
    prefix_len: usize,
}

/// Recognize a list marker opening `text`, if any.
///
/// The unordered case reuses [`opens_a_list_item`]'s bullet/enclosed-enumeration
/// glyphs, so a dash that signs a number (`-0.2`) is not a marker. The ordered case is a
/// literal `"<1-3 digits>."` or `"<1-3 digits>)"`
/// prefix — the number is read off the page as printed, not inferred: unpdf
/// analyzes one page at a time and has no cross-line state to detect a
/// *continuing* sequence the way undoc/unhwp's two-pass IR can (see the
/// umbrella ROADMAP's U-3 notes on why that logic doesn't port). The 3-digit
/// cap keeps a 4+ digit prefix (a year, in "2024. 01. 15") from being read as
/// an implausible ordered-list start.
fn detect_list_marker(text: &str) -> Option<ListMarkerMatch> {
    let leading_ws = text.len() - text.trim_start().len();
    let trimmed = &text[leading_ws..];
    let first = trimmed.chars().next()?;

    if opens_a_list_item(trimmed) {
        let marker_len = first.len_utf8();
        let after = &trimmed[marker_len..];
        let ws_len = after.len() - after.trim_start().len();
        return Some(ListMarkerMatch {
            ordered_number: None,
            prefix_len: leading_ws + marker_len + ws_len,
        });
    }

    let digits_len = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    if digits_len == 0 || digits_len > 3 {
        return None;
    }
    let after_digits = &trimmed[digits_len..];
    let delim = after_digits.chars().next()?;
    if delim != '.' && delim != ')' {
        return None;
    }
    let after_delim = &after_digits[delim.len_utf8()..];
    if !after_delim.is_empty() && !after_delim.starts_with(char::is_whitespace) {
        return None;
    }
    let number: u32 = trimmed[..digits_len].parse().ok()?;
    let ws_len = after_delim.len() - after_delim.trim_start().len();
    Some(ListMarkerMatch {
        ordered_number: Some(number),
        prefix_len: leading_ws + digits_len + delim.len_utf8() + ws_len,
    })
}

/// Font statistics for heading detection.
#[derive(Debug, Clone, Default)]
pub struct FontStatistics {
    /// Body text font size (most common)
    pub body_size: f32,
    /// Font sizes larger than body (potential headings)
    pub heading_sizes: Vec<f32>,
    /// All observed font sizes with frequency (BTreeMap for deterministic iteration)
    pub size_histogram: BTreeMap<i32, usize>,
}

impl FontStatistics {
    /// Add a font size observation.
    pub fn add_size(&mut self, size: f32) {
        let key = (size * 10.0) as i32; // Round to 0.1 precision
        *self.size_histogram.entry(key).or_insert(0) += 1;
    }

    /// Calculate body size and heading sizes.
    pub fn analyze(&mut self) {
        if self.size_histogram.is_empty() {
            self.body_size = 12.0;
            return;
        }

        // Find the most common font size (body text)
        let (body_key, _) = self
            .size_histogram
            .iter()
            .max_by_key(|(_, count)| *count)
            .unwrap();
        self.body_size = *body_key as f32 / 10.0;

        // Find sizes larger than body (potential headings)
        let mut larger_sizes: Vec<f32> = self
            .size_histogram
            .keys()
            .filter(|k| **k as f32 / 10.0 > self.body_size + 0.5)
            .map(|k| *k as f32 / 10.0)
            .collect();
        larger_sizes.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        self.heading_sizes = larger_sizes;
    }

    /// Get heading level for a font size (1-6, or 0 for body text).
    ///
    /// Conservative: requires font size ≥ body + 2.5 to qualify, or body + 1.5
    /// combined with bold. Caps at level 4 to avoid `#####` spam.
    pub fn get_heading_level(&self, font_size: f32, is_bold: bool) -> u8 {
        let strong_threshold = self.body_size + 2.5;
        let bold_threshold = self.body_size + 1.5;

        let qualifies = font_size >= strong_threshold || (is_bold && font_size >= bold_threshold);
        if !qualifies {
            return 0;
        }

        // Rank within distinct heading-size tiers. We dedupe `heading_sizes`
        // on the fly so that sizes clustered within 2pt count as one tier —
        // Hancom docs have many discrete sizes that would otherwise produce
        // erratic H-level assignment.
        let mut tier = 0u8;
        let mut last_tier_size: Option<f32> = None;
        for &heading_size in &self.heading_sizes {
            let is_new_tier = match last_tier_size {
                None => true,
                Some(prev) => (prev - heading_size).abs() >= 2.0,
            };
            if is_new_tier {
                tier = tier.saturating_add(1);
                last_tier_size = Some(heading_size);
            }
            if font_size >= heading_size - 0.5 {
                return tier.min(4);
            }
        }
        4
    }
}

impl<'a> LayoutAnalyzer<'a> {
    /// Create a new layout analyzer.
    pub fn new(backend: &'a dyn PdfBackend) -> Self {
        Self {
            backend,
            font_stats: FontStatistics::default(),
            suppress_low_confidence_ocr: true,
            ocr_text_suppressed: Cell::new(false),
            text_op_count: Cell::new(0),
            image_op_count: Cell::new(0),
            form_op_count: Cell::new(0),
            suppressed_text_runs: Cell::new(0),
            unreadable_fonts: RefCell::new(Vec::new()),
            undecodable_content_streams: Cell::new(0),
            page_facts: Cell::new(PageFacts::default()),
        }
    }

    /// Enable or disable dropping of low-confidence OCR text layers.
    pub fn with_ocr_suppression(mut self, enabled: bool) -> Self {
        self.suppress_low_confidence_ocr = enabled;
        self
    }

    /// Whether any page analysed so far had its OCR text layer dropped.
    pub fn ocr_text_suppressed(&self) -> bool {
        self.ocr_text_suppressed.get()
    }

    /// Text runs discarded by the font decoder on the page last analysed.
    ///
    /// Non-zero means the page's output is missing content the document contained —
    /// the decoder could not read those runs and dropped them rather than emit noise.
    pub fn suppressed_text_runs(&self) -> usize {
        self.suppressed_text_runs.get()
    }

    /// The fonts behind [`Self::suppressed_text_runs`] on the page last analysed: one
    /// entry per font and reason, in order of first occurrence.
    pub fn unreadable_fonts(&self) -> Vec<UnreadableFont> {
        self.unreadable_fonts.borrow().clone()
    }

    /// Content streams of the page last analysed that could not be decoded.
    ///
    /// Non-zero means the page's output is missing whatever those streams held.
    pub fn undecodable_content_streams(&self) -> usize {
        self.undecodable_content_streams.get()
    }

    /// The operator counts of the page last analysed — what tells an image-only scanned
    /// page apart from a blank one; `parse_single_page` copies them onto the `Page`.
    pub fn page_op_counts(&self) -> PageOpCounts {
        PageOpCounts {
            text: self.text_op_count.get(),
            image: self.image_op_count.get(),
            form: self.form_op_count.get(),
        }
    }

    /// What reading the page last analysed found out about it.
    pub fn page_facts(&self) -> PageFacts {
        self.page_facts.get()
    }

    /// Record how many tables were built from the page's ruling-line grids.
    pub(crate) fn note_ruled_tables(&self, tables: usize) {
        let mut facts = self.page_facts.get();
        facts.ruled_tables = tables as u32;
        self.page_facts.set(facts);
    }

    /// The display names of the fonts the names in `scope` refer to.
    fn font_names(&self, scope: ResourceScope) -> HashMap<Vec<u8>, FontInfo> {
        self.backend
            .page_fonts(scope)
            .unwrap_or_default()
            .into_iter()
            .map(|fi| {
                (
                    fi.name,
                    FontInfo {
                        name: fi.base_font,
                        bold: fi.bold,
                        italic: fi.italic,
                    },
                )
            })
            .collect()
    }

    /// Get mutable reference to font statistics (for external use).
    pub fn font_stats_mut(&mut self) -> &mut FontStatistics {
        &mut self.font_stats
    }

    /// Public wrapper for group_spans_into_lines.
    pub fn group_spans_into_lines_pub(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        self.group_spans_into_lines(spans)
    }

    /// Public wrapper for detect_headings.
    pub fn detect_headings_pub(&self, lines: Vec<TextLine>) -> Vec<TextLine> {
        Self::detect_headings(&self.font_stats, lines)
    }

    /// Public wrapper for detect_headings around tables taken out of the text (their tops).
    pub fn detect_headings_around_pub(
        &self,
        lines: Vec<TextLine>,
        tables_at: &[f32],
    ) -> Vec<TextLine> {
        Self::detect_headings_around(&self.font_stats, lines, tables_at)
    }

    /// Public wrapper for group_lines_into_blocks.
    pub fn group_lines_into_blocks_pub(&self, lines: Vec<TextLine>) -> Vec<TextBlock> {
        Self::group_lines_into_blocks(lines)
    }

    /// Filter header/footer spans in-place using page dimensions.
    ///
    /// Exposed so callers that operate on raw spans (e.g., the table-detection
    /// path) can apply the same margin filtering that `extract_page_blocks` uses.
    pub fn filter_spans_for_page(&self, spans: &mut Vec<TextSpan>, page_num: u32) {
        let pages = self.backend.pages();
        if let Some(&page_id) = pages.get(&page_num) {
            filter_header_footer_spans(spans, self.backend.crop_box(page_id));
        }
    }

    /// Extract text spans from a page with position and font information.
    pub fn extract_page_spans(&self, page_num: u32) -> Result<Vec<TextSpan>> {
        Ok(self.extract_page_spans_and_lattice_grids(page_num)?.0)
    }

    /// Like [`extract_page_spans`](Self::extract_page_spans), but also returns
    /// the page's inferred lattice (ruling-line) grids — used by lattice-mode
    /// table detection. Kept `pub(crate)`: `LatticeGrid` is an internal
    /// detection-pipeline type, not part of this crate's public surface.
    pub(crate) fn extract_page_spans_and_lattice_grids(
        &self,
        page_num: u32,
    ) -> Result<(Vec<TextSpan>, PageRulings)> {
        let pages = self.backend.pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;

        let painted = super::form_xobject::page_operations(self.backend, *page_id)?;
        self.undecodable_content_streams
            .set(painted.undecodable_streams);
        self.form_op_count.set(painted.forms_painted);
        let (spans, signals, grids) = self.parse_operations(&painted.ops, *page_id)?;

        if self.suppress_low_confidence_ocr && signals.is_ocr_layer_over_scan() {
            let text = spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if super::ocr_gate::is_incoherent_text(&text) {
                log::debug!(
                    "Page {}: dropping invisible OCR text layer — no readable text recognised",
                    page_num
                );
                self.ocr_text_suppressed.set(true);
                return Ok((Vec::new(), PageRulings::default()));
            }
        }

        Ok((spans, grids))
    }

    /// Extract structured text blocks from a page.
    pub fn extract_page_blocks(&mut self, page_num: u32) -> Result<Vec<TextBlock>> {
        // Get page dimensions for header/footer filtering
        let pages = self.backend.pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
        let page_box = self.backend.crop_box(*page_id);

        let mut spans = self.extract_page_spans(page_num)?;

        // Filter out page numbers / running headers from top/bottom margins
        filter_header_footer_spans(&mut spans, page_box);

        // Update font statistics
        for span in &spans {
            self.font_stats.add_size(span.font_size);
        }
        self.font_stats.analyze();

        // Group spans into lines
        let lines = self.group_spans_into_lines(spans);

        // Detect headings
        let lines = Self::detect_headings(&self.font_stats, lines);

        // Group lines into blocks (paragraphs)
        let blocks = Self::group_lines_into_blocks(lines);

        Ok(blocks)
    }

    /// Parse a page's operations (forms already interpreted in place) into text spans.
    ///
    /// Delegates text decoding to the backend, keeping layout.rs free from concrete PDF
    /// library types.
    fn parse_operations(
        &self,
        operations: &[ContentOp],
        page_id: super::backend::PageId,
    ) -> Result<(Vec<TextSpan>, PageTextLayerSignals, PageRulings)> {
        // Font names resolve per resource scope: the same `/F1` can be a different font
        // inside a form than on the page.
        let mut fonts: HashMap<Option<super::backend::ObjectId>, HashMap<Vec<u8>, FontInfo>> =
            HashMap::new();
        let ruling_lines =
            super::vector_graphics::extract_lines(operations, self.backend.crop_box(page_id));
        let lattice_grids =
            super::lattice::infer_grids(&ruling_lines, &super::lattice::LatticeConfig::default());
        let rule_stacks = super::ruled_rows::rule_stacks(&ruling_lines);
        log::debug!(
            "page has {} painted vector line segments; {} lattice grid(s) inferred",
            ruling_lines.len(),
            lattice_grids.len()
        );
        // 페이지 오퍼레이터 통계 리셋 — 같은 페이지를 재분석해도(fallback 경로)
        // 마지막 호출의 집계가 그대로 유효하도록 진입 시점에 0으로 되돌린다.
        self.text_op_count.set(0);
        self.image_op_count.set(0);
        self.suppressed_text_runs.set(0);
        self.unreadable_fonts.borrow_mut().clear();
        // What the page shows: marks outside its crop box or its clipping path are never
        // seen, so they are not page content.
        let visible = self.backend.crop_box(page_id);
        let mut clip = ClipTracker::new(visible);
        // Where each image paint shows on the page, in page space.
        let mut image_rects: Vec<Bounds> = Vec::new();
        let mut rotated_text_runs = 0u32;
        let mut invisible_chars = 0usize;
        let mut total_chars = 0usize;
        let mut suppressed_runs = 0usize;
        let mut unreadable_fonts: Vec<UnreadableFont> = Vec::new();
        let mut signals = PageTextLayerSignals::default();

        let mut spans = Vec::new();
        // Text state (ISO 32000-1 §9.3) is part of the graphics state: `q`/`Q` save and
        // restore it along with the CTM, and it persists across `BT`/`ET`.
        let mut text_state = TextState::default();
        let mut text_state_stack: Vec<TextState> = Vec::new();
        let mut text_matrix = super::text_state::TextMatrix::default();
        let mut in_text_block = false;
        // Current Transformation Matrix (starts as identity [1,0,0,1,0,0])
        let mut ctm: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut ctm_stack: Vec<[f32; 6]> = Vec::new();

        for op in operations {
            clip.observe(op, &ctm);
            // 페이지 판별용 오퍼레이터 통계 — 아래 본 match 의 가드 조건과
            // 무관하게 항상 집계한다 (`Do` arm 은 page_area 가드가 있음).
            match op.operator.as_str() {
                "Tj" | "TJ" | "'" | "\"" => self.text_op_count.set(self.text_op_count.get() + 1),
                // An image paint maps the unit square through the CTM — an XObject's `Do`
                // and an inline image (`BI ... EI`) alike. Forms are interpreted in place,
                // so a `Do` left here is never a form's.
                "Do" | "BI" => {
                    self.image_op_count.set(self.image_op_count.get() + 1);
                    image_rects.extend(clip.visible_part(&unit_square_on_page(&ctm)));
                }
                _ => {}
            }
            let operand = |i: usize| op.operands.get(i).and_then(get_number_from_value);
            // The text state and the text position, read as page rendering reads them.
            text_state.params.apply(op);
            text_matrix.apply(op, &text_state.params);
            match op.operator.as_str() {
                "q" => {
                    ctm_stack.push(ctm);
                    text_state_stack.push(text_state.clone());
                }
                "Q" => {
                    if let Some(saved) = ctm_stack.pop() {
                        ctm = saved;
                    }
                    if let Some(saved) = text_state_stack.pop() {
                        text_state = saved;
                    }
                }
                "cm" if op.operands.len() >= 6 => {
                    let cm = [
                        operand(0).unwrap_or(1.0),
                        operand(1).unwrap_or(0.0),
                        operand(2).unwrap_or(0.0),
                        operand(3).unwrap_or(1.0),
                        operand(4).unwrap_or(0.0),
                        operand(5).unwrap_or(0.0),
                    ];
                    ctm = apply_cm(&ctm, &cm);
                }
                "BT" => in_text_block = true,
                "ET" => {
                    in_text_block = false;
                }
                "Tf" => {
                    if let PdfValue::Name(font_name) = &op.operands[0] {
                        text_state.font_resource = font_name.clone();
                        let fonts = fonts
                            .entry(op.form)
                            .or_insert_with(|| self.font_names(op.scope(page_id)));
                        let info = fonts.get(font_name.as_slice());
                        text_state.font = match info {
                            Some(info) => info.name.clone(),
                            None => String::from_utf8_lossy(font_name.as_slice()).to_string(),
                        };
                        text_state.font_bold = info.is_some_and(|info| info.bold);
                        text_state.font_italic = info.is_some_and(|info| info.italic);
                    }
                }
                // `'` and `"` have already moved to the next line, and `"` set its spacing.
                "Tj" | "TJ" | "'" | "\"" => {
                    if !in_text_block {
                        continue;
                    }

                    let items: &[PdfValue] = match op.operator.as_str() {
                        "TJ" => match op.operands.first() {
                            Some(PdfValue::Array(arr)) => arr,
                            _ => &[],
                        },
                        "\"" => op.operands.get(2..3).unwrap_or(&[]),
                        _ => op.operands.get(..1).unwrap_or(&[]),
                    };

                    // A cell gap drawn inside one TJ is not a word space: LaTeX sets a table row
                    // as one array, the space between its cells as one large adjustment, and a
                    // run spanning the row crosses every column. The array is split there and
                    // each part placed where the array draws it — the adjustment becomes the
                    // next part's lead. Only when every string's glyph widths are known, since
                    // the next part starts where the previous one's advance ends.
                    let splittable = op.operator == "TJ"
                        && !self
                            .backend
                            .is_vertical_font(op.scope(page_id), &text_state.font_resource)
                        && items.iter().all(|item| match item {
                            PdfValue::Str(bytes) => self
                                .backend
                                .glyph_advances(op.scope(page_id), &text_state.font_resource, bytes)
                                .is_some(),
                            _ => true,
                        });
                    let parts = if splittable {
                        split_at_cell_gaps(items)
                    } else {
                        vec![items]
                    };
                    for items in parts {
                        // The run's text, and how far it moves the text position in text
                        // space — `None` as soon as one string's glyph widths are unknown.
                        let mut text = String::new();
                        let mut advance: Option<Shift> = Some(Shift::default());
                        // Adjustments before the first string move where the run *starts*, not
                        // how long it is: a TJ that opens with a large offset draws its first
                        // glyph far to the right of the text origin (a right-aligned date, a
                        // tab stop). They depend on the font size only, never on glyph widths.
                        let mut lead = Shift::default();
                        let mut drawn = false;
                        // A vertical font writes down the page, so its adjustments move y.
                        let vertical = self
                            .backend
                            .is_vertical_font(op.scope(page_id), &text_state.font_resource);
                        // Where the first glyph's origin lies from the text position, and how wide
                        // the widest glyph is (the horizontal extent of a vertical run).
                        let mut origin = Shift::default();
                        let mut cell: f32 = 0.0;
                        let mut first_glyph = true;
                        for item in items {
                            match item {
                                PdfValue::Str(bytes) => {
                                    drawn = true;
                                    let decoded = self.backend.decode_text(
                                        op.scope(page_id),
                                        &text_state.font_resource,
                                        bytes,
                                    );
                                    note_suppression(
                                        &decoded,
                                        &text_state,
                                        &mut suppressed_runs,
                                        &mut unreadable_fonts,
                                    );
                                    text.push_str(&decoded.text);
                                    let glyphs = self.backend.glyph_advances(
                                        op.scope(page_id),
                                        &text_state.font_resource,
                                        bytes,
                                    );
                                    if let Some(glyphs) = &glyphs {
                                        if let (true, Some(first)) = (first_glyph, glyphs.first()) {
                                            origin = text_state.params.origin_offset(first);
                                            first_glyph = false;
                                        }
                                        for g in glyphs {
                                            cell = cell.max(
                                                g.width / 1000.0
                                                    * text_state.params.size
                                                    * text_state.params.horizontal_scale,
                                            );
                                        }
                                    }
                                    advance = advance.zip(glyphs).map(|(sum, glyphs)| {
                                        sum + text_state.params.advance_of(&glyphs)
                                    });
                                }
                                // TJ adjustments: thousandths of text space, subtracted.
                                PdfValue::Integer(_) | PdfValue::Real(_) => {
                                    let n = get_number_from_value(item).unwrap_or(0.0);
                                    let shift = text_state.params.adjustment(n, vertical);
                                    if !drawn {
                                        lead += shift;
                                    }
                                    maybe_insert_space_tj(&mut text, -n);
                                    if let Some(sum) = advance.as_mut() {
                                        *sum += shift;
                                    }
                                }
                                _ => {}
                            }
                        }

                        // The run's baseline is lifted by the text rise, as a superscript's is.
                        let rise = text_state.params.rise;
                        let start = lead + origin;
                        let (tx, ty) = text_matrix.point(start.tx, rise + start.ty);
                        let (x, y) = apply_ctm(&ctm, tx, ty);
                        let effective_size = text_state.params.size
                            * text_matrix.vertical_scale()
                            * ctm_y_scale(&ctm);
                        // The run's extent in device space, from its first glyph to where it
                        // left the text position.
                        let end = advance.map(|run| {
                            let end = run + origin;
                            let (ex, ey) = text_matrix.point(end.tx, rise + end.ty);
                            apply_ctm(&ctm, ex, ey)
                        });
                        // A vertical run is as wide as its widest glyph; its length is down the
                        // page, not along the line.
                        let measured_width = if vertical {
                            advance.map(|_| {
                                let (ex, ey) = text_matrix.point(start.tx + cell, rise + start.ty);
                                let (ex, ey) = apply_ctm(&ctm, ex, ey);
                                (ex - x).hypot(ey - y)
                            })
                        } else {
                            end.map(|(dx, dy)| (dx - x).hypot(dy - y))
                        };

                        // A run the clip hides entirely shows nothing — the margin text of a
                        // larger page placed onto a smaller one, a form's content beyond its box.
                        if !clip.admits(&run_bounds((x, y), end, effective_size)) {
                            if let Some(run) = advance {
                                text_matrix.advance(run);
                            }
                            continue;
                        }

                        if !text.trim().is_empty() {
                            if !reads_left_to_right(&text_matrix.tm, &ctm) {
                                rotated_text_runs += 1;
                            }
                            count_render_mode(
                                &text,
                                text_state.params.render_mode,
                                &mut total_chars,
                                &mut invisible_chars,
                            );
                            let mut span =
                                TextSpan::new(text, x, y, effective_size, text_state.font.clone());
                            span.is_bold |= text_state.font_bold;
                            span.is_italic |= text_state.font_italic;
                            if let Some(width) = measured_width {
                                span.width = width;
                                span.width_measured = true;
                            }
                            spans.push(span);
                        } else if text.chars().any(char::is_whitespace) {
                            attach_word_space(&mut spans, x, y, effective_size, measured_width);
                        }

                        if let Some(run) = advance {
                            text_matrix.advance(run);
                        }
                    }
                }
                _ => {}
            }
        }

        if total_chars > 0 {
            signals.invisible_char_ratio = invisible_chars as f32 / total_chars as f32;
        }
        self.suppressed_text_runs.set(suppressed_runs);
        *self.unreadable_fonts.borrow_mut() = unreadable_fonts;

        let page_area = Bounds::from_page_box(visible).area();
        let clipped = image_rects;
        if page_area > 0.0 {
            signals.has_page_covering_image = clipped
                .iter()
                .any(|r| r.area() / page_area >= PageTextLayerSignals::PAGE_COVERAGE);
        }
        let image_coverage = if page_area > 0.0 {
            (union_area(&clipped) / page_area).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut facts = self.page_facts.get();
        facts.image_coverage = image_coverage;
        facts.rotated_text_runs = rotated_text_runs;
        facts.ruled_grids = lattice_grids.len() as u32;
        facts.ruled_tables = 0;
        self.page_facts.set(facts);

        Ok((
            spans,
            signals,
            PageRulings {
                grids: lattice_grids,
                rule_stacks,
            },
        ))
    }

    /// Group spans into lines based on Y position, using XY-Cut for layout segmentation.
    ///
    /// Uses the recursive XY-Cut algorithm to detect multi-column layouts and
    /// other complex structures. Each segmented region is processed independently
    /// as a single-column block.
    fn group_spans_into_lines(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        if spans.is_empty() {
            return vec![];
        }

        // Convert spans to XY-cut blocks
        let blocks = xycut_blocks(&spans);
        let config = xycut_config(&spans);
        let segmentation = super::xycut::xycut_partition(&blocks, &config);

        log::debug!(
            "XY-Cut segmented {} spans into {} groups, {} ambiguous ({:?})",
            spans.len(),
            segmentation.groups.len(),
            segmentation.ambiguous_regions,
            config,
        );

        let mut facts = self.page_facts.get();
        facts.reading_regions = segmentation.groups.len() as u32;
        facts.column_count = segmentation.column_count(&blocks) as u32;
        facts.ambiguous_layout_regions = segmentation.ambiguous_regions as u32;
        self.page_facts.set(facts);

        if segmentation.groups.len() <= 1 {
            // Single column — use simple grouping
            return self.group_spans_into_lines_single_column(spans);
        }

        // Multi-column: process each group independently. The groups hold the indices of
        // the blocks built from `spans`, one block per span, so each span lands in exactly
        // one group.
        let mut slots: Vec<Option<TextSpan>> = spans.into_iter().map(Some).collect();
        let mut all_lines = Vec::new();
        for group in &segmentation.groups {
            let group_spans: Vec<TextSpan> =
                group.iter().filter_map(|&i| slots[i].take()).collect();
            let lines = self.group_spans_into_lines_single_column(group_spans);
            all_lines.extend(lines);
        }
        all_lines
    }

    /// Simple Y-based line grouping for single-column layout.
    fn group_spans_into_lines_single_column(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        if spans.is_empty() {
            return vec![];
        }

        let mut spans = coalesce_runs(spans);

        // Widths are needed downstream by `should_insert_space_between` and by table
        // detection, but they must be assigned *after* merging: `merge_fragmented_spans`
        // uses `width == 0.0` as its "unmeasured fragment" signal. The estimate is also
        // only ever used as a plausible bounding box — it is not an advance width.
        for span in &mut spans {
            if span.width <= 0.0 {
                span.width = estimate_text_width(&span.text, span.font_size);
            }
        }

        let mut lines: Vec<TextLine> = Vec::new();
        let mut current_line_spans: Vec<TextSpan> = Vec::new();
        let mut current_y: Option<f32> = None;
        // Font size of the span that opened the current line. Used together with
        // the incoming span's size so a large-font line can't pull a small-font
        // neighbour onto its baseline (a 20pt title's 30% tolerance would
        // otherwise swallow a 10pt line sitting a few points above it and the
        // two would average into a mid-size pseudo-heading).
        let mut current_line_font_size: f32 = 0.0;

        for span in spans {
            // Allow 30% of font size variance, measured on the *smaller* of the
            // two sizes so tolerance scales with the line, not with whichever
            // span happens to arrive next.
            let y_tolerance = span.font_size.min(current_line_font_size) * 0.3;

            if let Some(y) = current_y {
                if (span.y - y).abs() <= y_tolerance {
                    // Same line
                    current_line_spans.push(span);
                } else {
                    // New line
                    if !current_line_spans.is_empty() {
                        lines.push(TextLine::from_spans(std::mem::take(
                            &mut current_line_spans,
                        )));
                    }
                    current_line_font_size = span.font_size;
                    current_y = Some(span.y);
                    current_line_spans.push(span);
                }
            } else {
                current_line_font_size = span.font_size;
                current_y = Some(span.y);
                current_line_spans.push(span);
            }
        }

        // Don't forget the last line
        if !current_line_spans.is_empty() {
            lines.push(TextLine::from_spans(current_line_spans));
        }

        attach_script_lines(lines)
    }

    /// Detect headings based on font size hierarchy.
    /// Takes the font statistics explicitly rather than reading `self`: everything this
    /// decision depends on is in there, and a heading rule that can be exercised without a
    /// backend is a heading rule that can be tested.
    fn detect_headings(font_stats: &FontStatistics, lines: Vec<TextLine>) -> Vec<TextLine> {
        Self::detect_headings_around(font_stats, lines, &[])
    }

    /// [`Self::detect_headings`] for text a table was taken out of: `tables_at` holds the top
    /// of each table, so that the lines above and below one are not taken for neighbours.
    fn detect_headings_around(
        font_stats: &FontStatistics,
        mut lines: Vec<TextLine>,
        tables_at: &[f32],
    ) -> Vec<TextLine> {
        // Snapshot each line's font size so neighbour lookups aren't polluted
        // by mutations inside the loop.
        let sizes: Vec<f32> = lines.iter().map(|l| l.font_size).collect();
        // Whether each line is mostly bold.
        let has_bold: Vec<bool> = lines.iter().map(TextLine::is_bold).collect();
        // Each line's text and baseline, for telling a wrapped title from a run of siblings.
        let texts: Vec<String> = lines.iter().map(|l| l.text().trim().to_string()).collect();
        let ys: Vec<f32> = lines.iter().map(|l| l.y).collect();
        let body_size = font_stats.body_size;

        for (i, line) in lines.iter_mut().enumerate() {
            let visible_chars: usize = line
                .text()
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
                .count();
            if visible_chars < 3 {
                continue;
            }

            // List / bullet marker exclusion — never promote a line that
            // begins with a common bullet/list glyph. These are inline
            // enumerations inside body content, regardless of font size.
            let trimmed = line.text();
            let trimmed = trimmed.trim_start();
            if opens_a_list_item(trimmed) {
                continue;
            }

            // Uppercase stands in for bold: a run of capitals is how many PDFs mark a heading
            // whose font carries no bold variant.
            let size_level =
                font_stats.get_heading_level(line.font_size, line.is_bold() || line.is_uppercase());

            // Style-based fallback: a short, bold, ALL-CAPS line at body size is a
            // section header ("EDUCATION", "EXPERIENCE") even though its font size
            // never reaches the size thresholds. Mixed-case bold lines (job titles,
            // names-in-bold) are deliberately excluded by the uppercase test so they
            // stay body text. These get a fixed level rather than a size tier.
            let (level, from_size) = if size_level > 0 {
                (size_level, true)
            } else if line.is_bold() && line.is_uppercase() && visible_chars <= 40 {
                (2, false)
            } else if let Some(depth) =
                (line.is_all_bold() && line.font_size >= body_size - 0.5 && visible_chars <= 100)
                    .then(|| section_number_depth(trimmed))
                    .flatten()
            {
                // A numbered section title in the body face's bold ("5. The dynamics",
                // "3.1. Status of operations") carries no size signal at all; the section
                // number on a line that is bold throughout does the work. The number's
                // depth gives the level, below a size-tier title.
                ((depth + 1).min(4) as u8, false)
            } else if is_bold_run_in_title(line, trimmed, body_size)
                && (i == 0 || !has_bold[i - 1])
                && lines_follow_as_body(&sizes, &has_bold, i, body_size)
            {
                // A short line in the body face's bold, standing between plain lines and
                // followed by plain body text, titles what follows ("Procedure:",
                // "Steps for Using the Microscope"). A bold line next to lines mostly in bold
                // (a list of names, a bold paragraph, a contents page's bold entries) or one
                // ending a sentence does not.
                (3, false)
            } else {
                continue;
            };

            // Neighbour-context suppression — if EITHER adjacent line shares this line's font
            // size (±0.5pt), the line is probably one of a run of siblings: a table column, a
            // list, a paragraph whose body font drifts by a point or two between cells. A
            // heading normally sits alone within its size cohort. The `body_size + 6.0` escape
            // keeps genuinely large headings that happen to be adjacent to another. Only
            // applies to size-based headings: a style-based header shares the body size with
            // its neighbours by definition, so the check would always suppress it.
            // Two kinds of same-size neighbour are not siblings: a section number set as a line
            // of its own beside or above its title ("3." | "RECOLLECTION OF …"), and the other
            // half of a title wrapped over two lines ("… balance of wood pellets and" |
            // "structure in Japan").
            if from_size {
                let same = |a: f32, b: f32| (a - b).abs() < 0.5;
                // A neighbour in the list is not a sibling across a table taken out of the text
                // between them: the titles above and below it are next to each other only in
                // the list.
                let sibling = |j: usize| {
                    same(sizes[j], line.font_size)
                        && !tables_at
                            .iter()
                            .any(|&t| t > ys[j].min(line.y) && t < ys[j].max(line.y))
                        && !is_bare_section_number(&texts[j])
                        && !(if j < i {
                            wraps_into(&texts[j], ys[j], trimmed, line.y, line.font_size)
                        } else {
                            wraps_into(trimmed, line.y, &texts[j], ys[j], line.font_size)
                        })
                };
                let matches_prev = i > 0 && sibling(i - 1);
                let matches_next = i + 1 < sizes.len() && sibling(i + 1);
                if (matches_prev || matches_next) && line.font_size < body_size + 6.0 {
                    continue;
                }
            }

            line.is_heading = true;
            line.heading_level = level;
        }

        // A chapter or section number set on a line of its own above its title ("4" over
        // "Basis Fields", as book classes lay out a chapter opening) is part of that title —
        // too short to be a heading by itself, so it joins the one below it. It must be set
        // at least as large as the title: a page number above a heading is smaller.
        for i in 0..lines.len().saturating_sub(1) {
            let (head, rest) = lines.split_at_mut(i + 1);
            let (number, title) = (&mut head[i], &rest[0]);
            if !number.is_heading
                && title.is_heading
                && is_bare_section_number(&number.text())
                && number.font_size >= title.font_size - 0.5
                && (number_sits_over(number, title) || number_sits_beside(number, title))
            {
                let own = font_stats.get_heading_level(number.font_size, number.is_bold());
                number.is_heading = true;
                number.heading_level = match own {
                    0 => title.heading_level,
                    own => own.min(title.heading_level),
                };
            }
        }
        lines
    }

    /// Group lines into blocks (paragraphs) based on spacing.
    fn group_lines_into_blocks(lines: Vec<TextLine>) -> Vec<TextBlock> {
        if lines.is_empty() {
            return vec![];
        }

        let mut blocks: Vec<TextBlock> = Vec::new();
        let mut current_block_lines: Vec<TextLine> = Vec::new();

        // Calculate average line spacing
        let avg_spacing = Self::calculate_avg_line_spacing(&lines);
        // The distance from each line to the one before it (0 for the first).
        let spacings: Vec<f32> = std::iter::once(0.0)
            .chain(lines.windows(2).map(|w| (w[0].y - w[1].y).abs()))
            .collect();
        let sizes: Vec<f32> = lines.iter().map(|l| l.font_size).collect();

        for (i, line) in lines.into_iter().enumerate() {
            if i == 0 {
                current_block_lines.push(line);
                continue;
            }

            let prev_line = current_block_lines.last().unwrap();

            // Check if this should start a new block
            let local_pitch = local_line_pitch(&spacings, &sizes, i);
            let should_break = Self::should_break_block(prev_line, &line, avg_spacing, local_pitch);

            if should_break {
                // Create block from current lines
                if !current_block_lines.is_empty() {
                    Self::push_block(
                        &mut blocks,
                        std::mem::take(&mut current_block_lines),
                        avg_spacing,
                    );
                }
            }

            current_block_lines.push(line);
        }

        // Don't forget the last block
        if !current_block_lines.is_empty() {
            Self::push_block(&mut blocks, current_block_lines, avg_spacing);
        }

        blocks
    }

    /// Finish `lines` as a block and append it — or, when it is a lone number that
    /// nothing continued, append it to the block before.
    ///
    /// A line starting `N.` opens a new block so that each list item is its own
    /// block. A marker alone on its line (`01.` with the item text on the lines
    /// below) is still an item when the lines after it join its block. When none
    /// do, the "marker" is the end of the previous block's text — the second half
    /// of a page range wrapped onto its own line (`432: 298-` / `306.`) — and as a
    /// list item it would be stripped to nothing and dropped.
    fn push_block(blocks: &mut Vec<TextBlock>, lines: Vec<TextLine>, avg_spacing: f32) {
        if let [line] = lines.as_slice() {
            let text = line.text();
            let is_bare_number = detect_list_marker(&text)
                .is_some_and(|m| m.ordered_number.is_some() && m.prefix_len >= text.len());
            if is_bare_number {
                if let Some(prev) = blocks.last_mut() {
                    let close = prev
                        .lines
                        .last()
                        .is_some_and(|p| (p.y - line.y).abs() <= avg_spacing * 1.5);
                    if close && prev.block_type != BlockType::Heading {
                        prev.lines.extend(lines);
                        return;
                    }
                }
            }
        }
        blocks.push(Self::finish_block(lines));
    }

    /// Classify a finished run of lines into a [`TextBlock`], deriving the
    /// heading level or list-item marker each type carries.
    ///
    /// Heading takes precedence over list-marker detection: `detect_headings`
    /// already declines to promote a bullet/enclosed-enumeration line (see
    /// `opens_a_list_item`), so a line that still reaches here `is_heading`
    /// is never itself an unordered marker — but an ordered numeral prefix
    /// (`"1. "`) isn't excluded there, since a numbered *heading*
    /// ("1. Introduction") is common and must stay a heading when the
    /// font-size heuristic already caught it.
    fn finish_block(lines: Vec<TextLine>) -> TextBlock {
        // A line of bold capitals that is a block of its own titles what follows, however long
        // it runs. (Within running text `detect_headings` caps such a line at 40 characters: a
        // bold capital sentence inside a paragraph is emphasis. Set off on its own, it is not.)
        if let [line] = lines.as_slice() {
            let visible = line
                .text()
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
                .count();
            if !line.is_heading
                && line.is_all_bold()
                && line.is_uppercase()
                && (3..=100).contains(&visible)
                && !opens_a_list_item(line.text().trim_start())
            {
                let mut block = TextBlock::new(lines, BlockType::Heading);
                block.heading_level = 2;
                return block;
            }
        }
        if lines.iter().any(|l| l.is_heading) {
            let heading_level = lines
                .iter()
                .filter(|l| l.is_heading)
                .map(|l| l.heading_level)
                .min()
                .unwrap_or(0);
            let mut block = TextBlock::new(lines, BlockType::Heading);
            block.heading_level = heading_level;
            return block;
        }

        if let Some(marker) = lines.first().and_then(|l| detect_list_marker(&l.text())) {
            let mut block = TextBlock::new(lines, BlockType::ListItem);
            block.list_item_number = marker.ordered_number;
            block.list_marker_len = marker.prefix_len;
            // An item is its text; with the marker stripped this one has none, and
            // would be dropped. What is left is a number standing on its own.
            if !block.list_item_text().trim().is_empty() {
                return block;
            }
            return TextBlock::new(block.lines, BlockType::Paragraph);
        }

        TextBlock::new(lines, BlockType::Paragraph)
    }

    /// Calculate average line spacing.
    /// The typical distance from one line to the next: the median of the spacings between
    /// consecutive lines.
    ///
    /// Not the mean. The spacings that matter least -- the jump over a figure, from the body to
    /// the footnotes, from one column to the next -- are the largest, and a few of them pulled a
    /// mean so far up that a title set off by twice the line pitch no longer counted as set off
    /// at all, and ran into the paragraph after it.
    fn calculate_avg_line_spacing(lines: &[TextLine]) -> f32 {
        let mut spacings: Vec<f32> = lines
            .windows(2)
            .map(|w| (w[0].y - w[1].y).abs())
            .filter(|s| *s > 0.1) // Filter out very small spacings
            .collect();
        if spacings.is_empty() {
            return 12.0; // Default
        }
        spacings.sort_by(f32::total_cmp);
        let mid = spacings.len() / 2;
        if spacings.len() % 2 == 1 {
            spacings[mid]
        } else {
            (spacings[mid - 1] + spacings[mid]) / 2.0
        }
    }

    /// Determine if a new block should start.
    fn should_break_block(
        prev_line: &TextLine,
        curr_line: &TextLine,
        avg_spacing: f32,
        local_pitch: Option<f32>,
    ) -> bool {
        // Heading always starts a new block, UNLESS the previous line is
        // also a heading of the same level sitting close by (within ~2x
        // line-height). This merges decorative stacked titles on covers —
        // e.g. "스마트\n제조혁신\n통합공고" stays one heading block.
        if curr_line.is_heading {
            // Merge consecutive heading lines that are spatially close and
            // at similar font size (≤ 2pt delta). Level difference is
            // tolerated — decorative stacked titles often vary font size
            // per word. The block picks up the minimum (most prominent)
            // level via existing `block.heading_level = ...min()` logic.
            // A section number on its own line opens the title below it, whatever the
            // difference in size (see `detect_headings`).
            if prev_line.is_heading
                && is_bare_section_number(&prev_line.text())
                && number_sits_over(prev_line, curr_line)
            {
                return false;
            }
            if prev_line.is_heading && (prev_line.font_size - curr_line.font_size).abs() <= 2.0 {
                let gap = (prev_line.y - curr_line.y).abs();
                let bigger = prev_line.font_size.max(curr_line.font_size);
                let close = gap <= (bigger * 2.5).max(avg_spacing * 2.0);
                if close {
                    return false;
                }
            }
            return true;
        }

        // After a heading, start new block
        if prev_line.is_heading {
            return true;
        }

        // A list marker (bullet or "N."/"N)") always opens a new item — never
        // silently merge it into whatever block precedes it, the way the
        // indentation tolerance below deliberately merges a wrapped item's
        // own continuation lines (no marker) into the same block. Neither
        // `curr_line` nor `prev_line` can be a heading here (both cases
        // returned above), so a numbered *heading* never reaches this check.
        if detect_list_marker(&curr_line.text()).is_some() {
            return true;
        }

        // Large spacing indicates new paragraph
        let spacing = (prev_line.y - curr_line.y).abs();
        if spacing > avg_spacing * 1.5 {
            return true;
        }
        // Set off from the lines around it: half as far again as the pitch of the neighbouring
        // lines, and at least twice the type size -- more than any leading, double spacing
        // included. A page with few lines has no typical spacing to compare against (a title, two
        // lines of text, a figure), and its lines are still set off from one another.
        let size = prev_line.font_size.max(curr_line.font_size);
        if local_pitch.is_some_and(|pitch| spacing > pitch * 1.5 && spacing > size * 2.0) {
            return true;
        }

        // Significant font size change (only break on >=2pt difference —
        // smaller changes are common in superscripts / mixed-font Korean
        // text and shouldn't fragment paragraphs).
        if (prev_line.font_size - curr_line.font_size).abs() >= 2.0 {
            return true;
        }

        // Significant left margin change (indentation) — raised from 20pt
        // to 40pt so minor indent variation within a Hancom bullet list
        // doesn't start a new block per line.
        if (prev_line.x - curr_line.x).abs() > 40.0 {
            return true;
        }

        false
    }
}

/// Filter out header/footer text spans (page numbers, running headers).
///
/// Removes spans in the top/bottom margin that contain only numbers or short
/// page-number patterns (e.g. "- 3 -", "Page 5", "2 / 10").
fn filter_header_footer_spans(spans: &mut Vec<TextSpan>, page_box: super::backend::PageBox) {
    let page_height = page_box.height();
    if spans.is_empty() || page_height <= 0.0 {
        return;
    }

    // Define margin regions: top/bottom 5% of page height.
    // PDF Y axis is bottom-up, measured from the page box's lower edge.
    let margin = page_height * 0.05;
    let top_threshold = page_box.ury - margin; // Near the top edge
    let bottom_threshold = page_box.lly + margin; // Near the bottom edge

    spans.retain(|span| {
        let in_header = span.y >= top_threshold;
        let in_footer = span.y <= bottom_threshold;

        if !in_header && !in_footer {
            return true; // Keep spans that are not in the margins
        }

        let text = span.text.trim();
        if text.is_empty() {
            return false; // Remove empty spans in margins
        }

        // Keep the span unless it looks like a bare page number
        let is_page_num = text.chars().all(|c| c.is_ascii_digit()) || is_page_number_pattern(text);

        !is_page_num
    });
    remove_side_margin_numbers(spans, page_box);
}

/// How far, as a share of the page width, the side margins reach in from the page edges.
const SIDE_MARGIN_SHARE: f32 = 0.15;

/// Remove page numbers set in a side margin — beside the text rather than above or below
/// it, as books and reports with a thumb tab or an outer-edge folio do.
///
/// Such a number often shares a baseline with a body line, which then reads it as a word:
/// `국내 46 경제는`. A run is taken for one when it is a bare page number lying in a side
/// margin entirely outside the horizontal extent of the page's other text, at least an em
/// clear of it, and set no larger than that text. Two things that look alike are kept: a
/// section number hanging in the margin beside its heading (its baseline holds bold or larger
/// text), and line numbers (more than two numbers down one margin).
fn remove_side_margin_numbers(spans: &mut Vec<TextSpan>, page_box: super::backend::PageBox) {
    let width = page_box.urx - page_box.llx;
    if spans.len() < 2 || width <= 0.0 {
        return;
    }
    let extent = |s: &TextSpan| {
        let w = if s.width > 0.0 {
            s.width
        } else {
            estimate_text_width(s.text.trim(), s.font_size)
        };
        (s.x, s.x + w)
    };
    let is_number = |s: &TextSpan| {
        let text = s.text.trim();
        !text.is_empty()
            && ((text.len() <= 4 && text.chars().all(|c| c.is_ascii_digit()))
                || is_page_number_pattern(text))
    };
    let text: Vec<&TextSpan> = spans.iter().filter(|s| !is_number(s)).collect();
    if text.is_empty() {
        return;
    }
    let left = text.iter().map(|s| extent(s).0).fold(f32::MAX, f32::min);
    let right = text.iter().map(|s| extent(s).1).fold(f32::MIN, f32::max);
    let mut sizes: Vec<f32> = text.iter().map(|s| s.font_size).collect();
    sizes.sort_by(f32::total_cmp);
    let body = sizes[sizes.len() / 2];

    // -1 for the left margin, 1 for the right, 0 for neither.
    let side = |s: &TextSpan| -> i8 {
        if !is_number(s) || s.font_size > body * 1.05 {
            return 0;
        }
        let (x0, x1) = extent(s);
        let em = s.font_size;
        if x1 <= page_box.llx + width * SIDE_MARGIN_SHARE && x1 + em <= left {
            -1
        } else if x0 >= page_box.urx - width * SIDE_MARGIN_SHARE && x0 >= right + em {
            1
        } else {
            0
        }
    };
    let beside_a_heading = |s: &TextSpan| {
        text.iter().any(|t| {
            (t.y - s.y).abs() <= t.font_size.max(s.font_size) * 0.3
                && (t.is_bold || t.font_size > body * 1.1)
        })
    };
    let sides: Vec<i8> = spans
        .iter()
        .map(|s| match side(s) {
            0 => 0,
            _ if beside_a_heading(s) => 0,
            d => d,
        })
        .collect();
    let down = |d: i8| sides.iter().filter(|&&x| x == d).count();
    let (left_count, right_count) = (down(-1), down(1));
    let mut i = 0;
    spans.retain(|_| {
        let d = sides[i];
        i += 1;
        !(d == -1 && left_count <= 2 || d == 1 && right_count <= 2)
    });
}

/// Return `true` if `text` matches a common page-number decoration pattern.
///
/// Recognised patterns:
/// - `"- N -"` / `"– N –"` / `"— N —"` (hyphen/dash-surrounded numbers)
/// - `"Page N"` / `"page N"`
/// - `"N / M"` or `"N of M"` (fraction-style)
fn is_page_number_pattern(text: &str) -> bool {
    let text = text.trim();

    // Pattern: "- N -" or "– N –" or "— N —"
    for dash in &['-', '–', '—'] {
        let dash_str = dash.to_string();
        if let Some(inner) = text.strip_prefix(dash_str.as_str()) {
            if let Some(inner) = inner.trim().strip_suffix(dash_str.as_str()) {
                if inner.trim().chars().all(|c| c.is_ascii_digit()) {
                    return true;
                }
            }
        }
    }

    // Pattern: "Page N" or "page N"
    if let Some(rest) = text
        .strip_prefix("Page ")
        .or_else(|| text.strip_prefix("page "))
    {
        if rest.trim().chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }

    // Pattern: "N / M" or "N of M"
    // Split on whitespace and '/', keep non-empty tokens
    let tokens: Vec<&str> = text
        .split(|c: char| c == '/' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.len() == 3
        && tokens[0].chars().all(|c| c.is_ascii_digit())
        && (tokens[1] == "of" || tokens[1] == "/")
        && tokens[2].chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    // "N / M" where slash is surrounded by spaces → tokens = ["N", "M"] after filtering
    if tokens.len() == 2
        && tokens[0].chars().all(|c| c.is_ascii_digit())
        && tokens[1].chars().all(|c| c.is_ascii_digit())
        && text.contains('/')
    {
        return true;
    }

    false
}

/// Font information.
#[derive(Debug, Clone)]
struct FontInfo {
    name: String,
    /// The font declares itself bold, whatever its name says.
    bold: bool,
    /// The font declares itself italic, whatever its name says.
    italic: bool,
}

/// The text-related parameters of the graphics state (ISO 32000-1 §9.3).
#[derive(Debug, Clone, Default)]
struct TextState {
    /// Font resource name as `Tf` names it (the key into the page's `/Font`).
    font_resource: Vec<u8>,
    /// The font's base name, for bold/italic detection.
    font: String,
    /// The font declares itself bold (descriptor or program), whatever its name says.
    font_bold: bool,
    /// The font declares itself italic (descriptor), whatever its name says.
    font_italic: bool,
    /// The rest of the text state.
    params: super::text_state::TextParams,
}

/// Where a text run can paint, in page space: from its origin to where it left the text
/// position (`end`, when its glyph widths are known), and from its descent below the baseline
/// to a full font size above it. It decides only whether the run can be visible at all.
fn run_bounds(origin: (f32, f32), end: Option<(f32, f32)>, size: f32) -> Bounds {
    let size = size.abs();
    let b = Bounds::around([origin, end.unwrap_or(origin)]).expect("at least one point");
    Bounds {
        y0: b.y0 - size * 0.3,
        y1: b.y1 + size,
        ..b
    }
}

/// The bounding box, in page space, of the unit square mapped through `ctm` — where an
/// image paint lands.
fn unit_square_on_page(ctm: &[f32; 6]) -> Bounds {
    let corners =
        [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(x, y)| apply_ctm(ctm, x, y));
    Bounds::around(corners).expect("four corners")
}

/// The area covered by the union of `rects`.
///
/// Exact by coordinate compression, which costs the square of the number of rectangles; a
/// page with more image paints than that can afford (tiled scans run to hundreds) is
/// measured on a sampling grid instead, accurate to well under a percent of the page.
fn union_area(rects: &[Bounds]) -> f32 {
    const EXACT_LIMIT: usize = 256;
    if rects.is_empty() {
        return 0.0;
    }
    if rects.len() > EXACT_LIMIT {
        return sampled_union_area(rects);
    }
    let mut xs: Vec<f32> = rects.iter().flat_map(|r| [r.x0, r.x1]).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mut area = 0.0f64;
    for w in xs.windows(2) {
        let (left, right) = (w[0], w[1]);
        let mut spans: Vec<(f32, f32)> = rects
            .iter()
            .filter(|r| r.x0 <= left && r.x1 >= right)
            .map(|r| (r.y0, r.y1))
            .collect();
        if spans.is_empty() {
            continue;
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut covered = 0.0f64;
        let (mut lo, mut hi) = spans[0];
        for &(y0, y1) in &spans[1..] {
            if y0 > hi {
                covered += f64::from(hi - lo);
                (lo, hi) = (y0, y1);
            } else {
                hi = hi.max(y1);
            }
        }
        covered += f64::from(hi - lo);
        area += covered * f64::from(right - left);
    }
    area as f32
}

fn sampled_union_area(rects: &[Bounds]) -> f32 {
    const N: usize = 400;
    let x0 = rects.iter().map(|r| r.x0).fold(f32::MAX, f32::min);
    let y0 = rects.iter().map(|r| r.y0).fold(f32::MAX, f32::min);
    let x1 = rects.iter().map(|r| r.x1).fold(f32::MIN, f32::max);
    let y1 = rects.iter().map(|r| r.y1).fold(f32::MIN, f32::max);
    let (dx, dy) = ((x1 - x0) / N as f32, (y1 - y0) / N as f32);
    if dx <= 0.0 || dy <= 0.0 {
        return 0.0;
    }
    let mut hit = 0usize;
    for row in 0..N {
        let y = y0 + (row as f32 + 0.5) * dy;
        let row_rects: Vec<&Bounds> = rects.iter().filter(|r| r.y0 <= y && y < r.y1).collect();
        for col in 0..N {
            let x = x0 + (col as f32 + 0.5) * dx;
            if row_rects.iter().any(|r| r.x0 <= x && x < r.x1) {
                hit += 1;
            }
        }
    }
    hit as f32 * dx * dy
}

/// Whether text set with text matrix `tm` under `ctm` runs left to right along a
/// horizontal baseline — within about five degrees, as text set on a horizontal line is.
fn reads_left_to_right(tm: &[f32; 6], ctm: &[f32; 6]) -> bool {
    // The text-space x axis in device space.
    let dx = tm[0] * ctm[0] + tm[1] * ctm[2];
    let dy = tm[0] * ctm[1] + tm[1] * ctm[3];
    dx > 0.0 && dy.abs() <= dx * 0.0875
}

/// Insert a space into `text` if it doesn't already end with one and the
/// last character is not from a spaceless script (CJK/Japanese).
/// A TJ adjustment at least this wide (thousandths of text space, i.e. of the font size)
/// separates cells, not words: justification stretches word spaces to about two-thirds of the
/// size at most, and a table's columns stand at least this far apart.
const TJ_CELL_GAP: f32 = 800.0;

/// The parts of a TJ array between its cell gaps — adjustments of at least [`TJ_CELL_GAP`]
/// that follow a drawn string and precede another. Each gap starts the part after it, so
/// that part's lead carries it.
fn split_at_cell_gaps(items: &[PdfValue]) -> Vec<&[PdfValue]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut drawn = false;
    for (i, item) in items.iter().enumerate() {
        match item {
            PdfValue::Str(_) => drawn = true,
            PdfValue::Integer(_) | PdfValue::Real(_) => {
                let gap = -get_number_from_value(item).unwrap_or(0.0);
                let more = items[i + 1..]
                    .iter()
                    .any(|later| matches!(later, PdfValue::Str(_)));
                if drawn && more && gap >= TJ_CELL_GAP {
                    parts.push(&items[start..i]);
                    start = i;
                    drawn = false;
                }
            }
            _ => {}
        }
    }
    parts.push(&items[start..]);
    parts
}

/// Insert a space in TJ array based on kerning adjustment, with script-aware thresholds.
///
/// TJ adjustments are in 1/1000 text space units. The threshold for inserting a space
/// varies by script:
/// - Latin: 120 units. Real-world word gaps sit well under the nominal ~250-unit
///   space width of many fonts — EB Garamond body text, for example, spaces words
///   with adjustments of ~167-200, while intra-word kerning and display-header
///   letterspacing stay at or below ~95. 120 splits that gap; the previous 200
///   threshold missed genuine word spaces and concatenated whole sentences.
/// - Hangul (Korean): 500 units (~50% of typical char width 1000)
///   Korean uses word spaces, but kerning between syllables is typically 100-300 units.
/// - CJK (Chinese/Japanese): never insert spaces (handled by is_spaceless_script_char)
fn maybe_insert_space_tj(text: &mut String, adjustment: f32) {
    if text.is_empty() || text.ends_with(' ') || text.ends_with('\u{00A0}') {
        return;
    }

    if let Some(last_char) = text.chars().last() {
        if is_spaceless_script_char(last_char) {
            return;
        }

        let threshold = if is_hangul_char(last_char) {
            500.0
        } else {
            120.0
        };
        if adjustment > threshold {
            text.push(' ');
        }
    }
}

/// Concatenate two PDF transformation matrices (right-multiply: result = a × b).
/// Matrix form: `[a, b, c, d, e, f]` where a point `(x,y)` transforms as
/// `x' = a*x + c*y + e`,  `y' = b*x + d*y + f`.
fn concat_matrix(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// Tally a decoded run that the font decoder discarded.
///
/// Counted per run rather than per character: the discarded text was never decoded,
/// so its length is unknown — the only honest unit is "how many runs went missing".
///
/// The run is also attributed to the font it was set in, so a consumer can say which
/// font could not be read.
fn note_suppression(
    decoded: &super::backend::DecodedText,
    text_state: &TextState,
    suppressed_runs: &mut usize,
    unreadable_fonts: &mut Vec<UnreadableFont>,
) {
    if let Some(reason) = decoded.suppressed {
        *suppressed_runs += 1;
        // A font without `/BaseFont` is listed as "Unknown" by the font table; the
        // resource name is the only identity it has.
        let name = if text_state.font.is_empty() || text_state.font == "Unknown" {
            String::from_utf8_lossy(&text_state.font_resource).into_owned()
        } else {
            text_state.font.clone()
        };
        match unreadable_fonts
            .iter_mut()
            .find(|f| f.reason == reason && f.name == name)
        {
            Some(entry) => entry.runs += 1,
            None => unreadable_fonts.push(UnreadableFont {
                name,
                reason,
                runs: 1,
            }),
        }
        log::debug!("dropped an unreadable text run: {:?}", reason);
    }
}

/// Tally characters by whether they were painted, for [`PageTextLayerSignals`].
///
/// Rendering mode 3 (and 7, which only sets a clipping path) draws nothing.
fn count_render_mode(text: &str, render_mode: i64, total: &mut usize, invisible: &mut usize) {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    *total += chars;
    if render_mode == 3 || render_mode == 7 {
        *invisible += chars;
    }
}

/// The CTM after a `cm` operator with operands `cm`.
///
/// `cm` maps the new user space into the *current* one, so the operand matrix is
/// applied first: CTM' = cm × CTM (ISO 32000-1 §8.4.4). The reverse order agrees
/// with it only while the CTM is the identity or a pure scale, which is why it
/// goes unnoticed on simple files — and why a browser-printed page, which opens
/// with a flipped and scaled CTM and then nests `cm` translations, came out with
/// every coordinate far off the page.
pub(crate) fn apply_cm(ctm: &[f32; 6], cm: &[f32; 6]) -> [f32; 6] {
    concat_matrix(cm, ctm)
}

/// Apply a CTM to a user-space point, returning device-space coordinates.
#[inline]
pub(crate) fn apply_ctm(ctm: &[f32; 6], x: f32, y: f32) -> (f32, f32) {
    (
        ctm[0] * x + ctm[2] * y + ctm[4],
        ctm[1] * x + ctm[3] * y + ctm[5],
    )
}

/// Return the scaling factor applied by `ctm` to the Y axis.
/// Used to scale font sizes into device space for layout comparisons.
#[inline]
fn ctm_y_scale(ctm: &[f32; 6]) -> f32 {
    // Y-axis unit vector (0,1) transforms to (ctm[2], ctm[3]).
    (ctm[2] * ctm[2] + ctm[3] * ctm[3]).sqrt().max(0.01)
}

/// What joins line `prev` to the next line `next` of one block: a space, unless the break
/// falls inside a word.
///
/// A space the producer wrote at the break says it is a word boundary. Without one, two
/// characters of a script with no spaces between words (CJK ideographs, kana) join directly,
/// and so do two Hangul syllables when the block shows this break is inside a word
/// (`hangul_in_word`, from [`TextBlock::hangul_breaks_in_words`]) — Korean may break a line
/// at any syllable. Anything else keeps the space: a Latin line's producer rarely writes
/// one, and its words never break without a hyphen.
pub(crate) fn line_join(prev: &str, next: &str, hangul_in_word: bool) -> &'static str {
    if prev.ends_with(char::is_whitespace) || next.starts_with(char::is_whitespace) {
        return " ";
    }
    let (Some(a), Some(b)) = (
        prev.trim_end().chars().last(),
        next.trim_start().chars().next(),
    ) else {
        return " ";
    };
    let spaceless = is_spaceless_script_char(a) && is_spaceless_script_char(b);
    let hangul_word = hangul_in_word && is_hangul_char(a) && is_hangul_char(b);
    if spaceless || hangul_word {
        ""
    } else {
        " "
    }
}

/// Check if a character is a Hangul (Korean) syllable or jamo.
fn is_hangul_char(c: char) -> bool {
    let code = c as u32;
    // Hangul Syllables
    (0xAC00..=0xD7AF).contains(&code)
    // Hangul Jamo
    || (0x1100..=0x11FF).contains(&code)
    // Hangul Compatibility Jamo
    || (0x3130..=0x318F).contains(&code)
    // Hangul Jamo Extended-A/B
    || (0xA960..=0xA97F).contains(&code)
    || (0xD7B0..=0xD7FF).contains(&code)
}

/// One XY-Cut block per span: where its text lies on the page.
pub(crate) fn xycut_blocks(spans: &[TextSpan]) -> Vec<super::xycut::Block> {
    spans
        .iter()
        .map(|s| super::xycut::Block {
            x: s.x,
            y: s.y,
            width: if s.width > 0.0 {
                s.width
            } else {
                estimate_text_width(&s.text, s.font_size)
            },
            height: s.font_size,
        })
        .collect()
}

/// XY-Cut's gap thresholds for `spans`, scaled to their median font size.
///
/// The unconditional ones are intentionally large so XY-Cut only fires on true
/// multi-column layouts — not on intra-table cell gaps or bulleted list indentation, which
/// previously fragmented pages into dozens of groups on Hancom-produced PDFs.
///
/// A column gutter is far narrower than that, so a narrower channel still splits — but only
/// when text of real width lies on both sides of it (see `XyCutConfig::min_gutter`), which
/// list markers and indents never do.
pub(crate) fn xycut_config(spans: &[TextSpan]) -> super::xycut::XyCutConfig {
    let median_font = median_font_size(spans);
    super::xycut::XyCutConfig {
        min_x_gap: (median_font * 5.0).max(60.0),
        min_y_gap: (median_font * 3.0).max(36.0),
        min_gutter: median_font.max(8.0),
    }
}

fn median_font_size(spans: &[TextSpan]) -> f32 {
    if spans.is_empty() {
        return 12.0;
    }
    let mut sizes: Vec<f32> = spans.iter().map(|s| s.font_size).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes[sizes.len() / 2]
}

/// Decide whether a space belongs between two adjacent spans on the same line,
/// based on the gap between the previous span's right edge and the current
/// span's left edge. Shared by [`TextLine::text`] and [`TextLine::styled_segments`]
/// so the plain-text and styled-run views of a line always agree on spacing.
/// For CJK characters, no space is inserted between adjacent characters.
pub(crate) fn should_insert_space_between(prev_span: &TextSpan, span: &TextSpan) -> bool {
    // Calculate gap between end of previous span and start of current span
    let prev_end = prev_span.x + prev_span.width;
    let gap = span.x - prev_end;

    // Estimate average character width from current span
    let char_count = span.text.chars().count();
    let avg_char_width = if char_count > 0 && span.width > 0.0 {
        span.width / char_count as f32
    } else {
        span.font_size * 0.5 // Fallback: assume half of font size
    };

    // Gap threshold: if gap is more than 20% of average char width, insert space
    let space_threshold = avg_char_width * 0.2;
    if gap <= space_threshold {
        return false;
    }

    // Don't insert a space between two CJK characters
    let prev_is_cjk = prev_span
        .text
        .chars()
        .last()
        .map(is_spaceless_script_char)
        .unwrap_or(false);
    let curr_is_cjk = span
        .text
        .chars()
        .next()
        .map(is_spaceless_script_char)
        .unwrap_or(false);
    if prev_is_cjk && curr_is_cjk {
        return false;
    }

    // Don't double up when either side already carries whitespace
    let prev_ends_with_space =
        prev_span.text.ends_with(' ') || prev_span.text.ends_with('\u{00A0}');
    let curr_starts_with_space = span.text.starts_with(' ') || span.text.starts_with('\u{00A0}');
    !prev_ends_with_space && !curr_starts_with_space
}

/// Check if character is from a script that doesn't use word spaces.
/// Chinese and Japanese don't use spaces between words, but Korean does.
fn is_spaceless_script_char(c: char) -> bool {
    let code = c as u32;

    // CJK Unified Ideographs (Chinese characters, used in Chinese/Japanese)
    (0x4E00..=0x9FFF).contains(&code)
    // CJK Unified Ideographs Extension A
    || (0x3400..=0x4DBF).contains(&code)
    // CJK Unified Ideographs Extension B-F
    || (0x20000..=0x2A6DF).contains(&code)
    || (0x2A700..=0x2B73F).contains(&code)
    || (0x2B740..=0x2B81F).contains(&code)
    || (0x2B820..=0x2CEAF).contains(&code)
    || (0x2CEB0..=0x2EBEF).contains(&code)
    // Hiragana (Japanese)
    || (0x3040..=0x309F).contains(&code)
    // Katakana (Japanese)
    || (0x30A0..=0x30FF).contains(&code)
    // NOTE: Hangul (Korean) is NOT included - Korean uses word spaces like English
    // CJK Symbols and Punctuation
    || (0x3000..=0x303F).contains(&code)
}

/// Estimate the drawn width of a text run from its characters and font size.
///
/// The PDF text-showing operators don't report an advance width and unpdf does
/// not parse the font's glyph-width tables, so `TextSpan.width` would otherwise
/// stay 0 — which makes XY-Cut treat every span as a zero-width point and split
/// a single-column page on any horizontal gap (e.g. right-aligned dates), and
/// leaves the span-join spacing heuristic with nothing to measure against. A
/// per-script average advance is enough to give layout a plausible bounding box:
/// - CJK ideographs / kana / Hangul syllables: ~1.0 em (full-width)
/// - whitespace: ~0.25 em
/// - everything else (Latin, digits, punctuation): ~0.5 em
fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    let ems: f32 = text
        .chars()
        .map(|c| {
            if is_spaceless_script_char(c) || is_hangul_char(c) {
                1.0
            } else if c.is_whitespace() {
                0.25
            } else {
                0.5
            }
        })
        .sum();
    ems * font_size
}

/// Attach an inter-word space to the most recent span.
///
/// Some producers (Word/LibreOffice exports) emit
/// each word as its own `BT … ET` object and put the word gaps in
/// whitespace-only text runs such as `[( )] TJ`. Those runs carry no content of
/// their own, but they are the only in-band evidence of the gap: `span.width`
/// is an estimate and cannot reliably recover it. Append a single space to the
/// preceding run on the same baseline instead of dropping the whitespace run.
///
/// `x` and `measured_width` place the whitespace run itself: when the font's widths
/// are known, the preceding run's extent grows to cover it exactly.
fn attach_word_space(
    spans: &mut [TextSpan],
    x: f32,
    y: f32,
    font_size: f32,
    measured_width: Option<f32>,
) {
    let Some(prev) = spans.last_mut() else {
        return;
    };

    // A whitespace run that opens a line is indentation, not a word gap: only
    // join it to a run on the same baseline.
    if (prev.y - y).abs() > font_size.max(prev.font_size) * 0.3 {
        return;
    }

    // Don't accumulate spaces.
    if prev.text.ends_with(' ') || prev.text.ends_with('\u{00A0}') {
        return;
    }

    prev.text.push(' ');
    // Keep the extent consistent with the appended glyph: measured when both runs
    // were, otherwise estimated (a no-op for unmeasured runs, whose width is 0 until
    // after merging).
    match measured_width {
        Some(width) if prev.width > 0.0 => {
            prev.width = prev.width.max(x + width - prev.x);
        }
        _ if prev.width > 0.0 => {
            prev.width += estimate_text_width(" ", prev.font_size);
        }
        _ => {}
    }
}

/// Fold lines that are really subscripts or superscripts back into the line they belong to.
///
/// A script is set smaller and shifted off the baseline, often by more than the same-line
/// tolerance, so baseline grouping gives it a line of its own — which then becomes a
/// paragraph between two body lines and, for a superscript, comes *before* its own line.
/// `lines` is in reading order (top to bottom), so a script line's owner is one of its
/// two neighbours: the line above for a subscript, the line below for a superscript.
fn attach_script_lines(lines: Vec<TextLine>) -> Vec<TextLine> {
    let mut out: Vec<TextLine> = Vec::with_capacity(lines.len());
    let mut pending = lines.into_iter().peekable();
    while let Some(line) = pending.next() {
        let above = out.last().filter(|owner| is_script_of(&line, owner));
        let below = pending.peek().filter(|owner| is_script_of(&line, owner));
        // Both can qualify only for a script squeezed between two lines; the nearer
        // baseline is the one it was set against.
        let into_above = match (above, below) {
            (Some(a), Some(b)) => (a.y - line.y).abs() <= (b.y - line.y).abs(),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => {
                out.push(line);
                continue;
            }
        };
        if into_above {
            let owner = out.pop().expect("checked above");
            out.push(owner.with_scripts(line));
        } else {
            let owner = pending.next().expect("checked below");
            out.push(owner.with_scripts(line));
        }
    }
    out
}

/// Whether every span of `line` reads as a script of `owner`: set clearly smaller,
/// shifted off `owner`'s baseline by less than a script's usual rise or drop, and sitting
/// beside `owner`'s text rather than over it.
///
/// The last condition is what keeps a genuine small line — a byline under a title, a
/// caption line — its own: it runs *across* its neighbour's text, a script runs *after*
/// a word.
fn is_script_of(line: &TextLine, owner: &TextLine) -> bool {
    let size = owner.font_size;
    if line.spans.is_empty() || owner.spans.is_empty() || size <= 0.0 {
        return false;
    }
    let smaller = line.spans.iter().all(|s| s.font_size <= size * 0.9);
    let near = (line.y - owner.y).abs() <= size * 0.6;
    if !smaller || !near {
        return false;
    }
    let extent = |s: &TextSpan| {
        let width = if s.width > 0.0 {
            s.width
        } else {
            estimate_text_width(&s.text, s.font_size)
        };
        (s.x, s.x + width)
    };
    let left = owner
        .spans
        .iter()
        .map(|s| extent(s).0)
        .fold(f32::MAX, f32::min);
    let right = owner
        .spans
        .iter()
        .map(|s| extent(s).1)
        .fold(f32::MIN, f32::max);
    // Estimated extents are approximate; a sliver of overlap is not "over the text".
    let slack = size * 0.25;
    line.spans.iter().all(|script| {
        let (x0, x1) = extent(script);
        let beside_owner = x0 >= left - size && x0 <= right + size;
        let over_text = owner.spans.iter().any(|s| {
            let (o0, o1) = extent(s);
            x0 < o1 - slack && x1 > o0 + slack
        });
        beside_owner && !over_text
    })
}

impl TextLine {
    /// This line with `scripts`' spans merged in, keeping this line's baseline.
    fn with_scripts(self, scripts: TextLine) -> TextLine {
        let (y, font_size) = (self.y, self.font_size);
        let mut spans = self.spans;
        spans.extend(scripts.spans);
        let mut merged = TextLine::from_spans(spans);
        merged.y = y;
        merged.font_size = font_size;
        merged
    }
}

/// Put spans in reading order (top to bottom, then left to right) and join the
/// fragments of one run: glyphs a producer drew one operator at a time, so that each
/// span is a stretch of text rather than an accident of how it was drawn.
///
/// Everything that reasons about span positions should see runs, not fragments —
/// line grouping, and table detection before it: a column edge found at the start of
/// a fragment is a boundary in the middle of a word. Applying it twice is harmless;
/// joined runs are no longer fragments.
pub(crate) fn coalesce_runs(mut spans: Vec<TextSpan>) -> Vec<TextSpan> {
    spans.sort_by(|a, b| {
        let y_cmp = b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal);
        if y_cmp == std::cmp::Ordering::Equal {
            a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            y_cmp
        }
    });
    merge_fragmented_spans(spans)
}

/// Whether `next` continues `prev` on the same line, in the same font, starting where
/// `prev` ends. Both extents must be measured.
///
/// The tolerance is a tenth of an em: kerning moves a glyph by a few hundredths, while
/// the narrowest word space in common fonts is about a fifth of an em.
fn abuts(prev: &TextSpan, next: &TextSpan) -> bool {
    let size = prev.font_size;
    let same_line = (prev.y - next.y).abs() <= size.min(next.font_size) * 0.3;
    let same_font = prev.font_name == next.font_name && (size - next.font_size).abs() < 0.1;
    let gap = next.x - (prev.x + prev.width);
    same_line && same_font && gap <= size * 0.1 && gap >= -size * 0.3
}

/// Merge adjacent fragmented spans that likely form words.
///
/// Some PDFs render text character-by-character with separate Tj operations,
/// creating many single-character spans with width=0. This function merges
/// consecutive short spans (≤3 chars) with width=0 that are:
/// - On the same baseline (same Y within tolerance)
/// - Using the same font at the same size
/// - Positioned sequentially without large gaps
///
/// Only merges into width=0 spans — when the previous span also has width=0 or
/// is itself a recently-merged fragment. This prevents merging normal
/// multi-character spans that happen to be adjacent.
fn merge_fragmented_spans(spans: Vec<TextSpan>) -> Vec<TextSpan> {
    if spans.len() < 2 {
        return spans;
    }

    let mut result: Vec<TextSpan> = Vec::with_capacity(spans.len());
    // Track which result spans were created by merging (started as width=0)
    let mut was_fragment: Vec<bool> = Vec::with_capacity(spans.len());

    for span in spans {
        // Runs whose extent was measured from the font's widths abut exactly when they
        // belong to one word, so they need no guess: join them when the next one starts
        // where the previous one ends. A word gap drawn as whitespace is already part of
        // the previous run (`attach_word_space`); a gap drawn as a move is left for the
        // line's spacing rule to turn into a space.
        if let Some(prev) = result.last_mut() {
            if prev.width > 0.0 && span.width > 0.0 && abuts(prev, &span) {
                prev.width = prev.width.max(span.x + span.width - prev.x);
                prev.width_measured &= span.width_measured;
                prev.text.push_str(&span.text);
                continue;
            }
        }

        let is_fragment = span.text.chars().count() <= 3 && span.width <= 0.0;

        let should_merge =
            if let Some((prev, prev_was_frag)) = result.last().zip(was_fragment.last()) {
                // Only merge if BOTH are fragments (or prev was already merged from fragments)
                if !is_fragment || !prev_was_frag {
                    false
                } else {
                    // Same baseline (Y within tolerance)
                    let y_tolerance = span.font_size * 0.3;
                    let same_y = (prev.y - span.y).abs() <= y_tolerance;

                    // Same font and size
                    let same_font = prev.font_name == span.font_name
                        && (prev.font_size - span.font_size).abs() < 0.1;

                    if !same_y || !same_font {
                        false
                    } else {
                        // Estimate character width from font size
                        let est_char_width = prev.font_size * 0.6;

                        // Estimate where the previous span ends
                        let prev_end = if prev.width > 0.0 {
                            prev.x + prev.width
                        } else {
                            prev.x + est_char_width * prev.text.chars().count() as f32
                        };

                        let gap = span.x - prev_end;

                        // Merge if gap is small enough to not be a word space.
                        // Character-to-character gap within a word: 0 to ~0.3 * char_width
                        // Word space gap: ~0.4 * char_width or more
                        gap < est_char_width * 0.4 && gap > -est_char_width * 0.5
                    }
                }
            } else {
                false
            };

        if should_merge {
            let prev = result.last_mut().unwrap();
            // Update width to cover the merged extent
            let new_end = span.x + span.font_size * 0.6 * span.text.chars().count() as f32;
            prev.width = new_end - prev.x;
            prev.width_measured = false;
            prev.text.push_str(&span.text);

            // A merged fragment that now ends in a word space is a completed word:
            // don't let the next fragment chain onto it.
            if let Some(flag) = was_fragment.last_mut() {
                *flag = !prev.text.ends_with(char::is_whitespace);
            }
        } else {
            was_fragment.push(is_fragment);
            result.push(span);
        }
    }

    result
}

#[cfg(test)]
mod tests {

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
        Bounds { x0, y0, x1, y1 }
    }

    #[test]
    fn union_area_counts_overlap_once() {
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let b = rect(5.0, 5.0, 15.0, 15.0);
        assert_eq!(union_area(&[a, b]), 175.0);
        assert_eq!(union_area(&[a, a]), 100.0);
        assert_eq!(union_area(&[]), 0.0);
    }

    /// Past the exact limit the area is sampled; it must agree with the exact answer.
    #[test]
    fn sampled_union_area_agrees_with_the_exact_one() {
        // 300 strips of a 600 x 800 page, each 2 wide, overlapping their neighbour by half.
        let strips: Vec<Bounds> = (0..300)
            .map(|i| rect(i as f32 * 1.0, 0.0, i as f32 * 1.0 + 2.0, 800.0))
            .collect();
        let exact = 301.0 * 800.0;
        let sampled = union_area(&strips);
        assert!(
            (sampled - exact).abs() / exact < 0.01,
            "{sampled} vs {exact}"
        );
    }

    #[test]
    fn text_direction_comes_from_the_text_matrix_through_the_ctm() {
        const I: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert!(reads_left_to_right(&I, &I));
        assert!(
            !reads_left_to_right(&[0.0, 1.0, -1.0, 0.0, 0.0, 0.0], &I),
            "rotated 90"
        );
        assert!(
            !reads_left_to_right(&[-1.0, 0.0, 0.0, -1.0, 0.0, 0.0], &I),
            "upside down"
        );
        // A page whose CTM rotates by 90 turns horizontal text-space lines vertical.
        assert!(!reads_left_to_right(&I, &[0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
        // A slight skew is still a horizontal line.
        assert!(reads_left_to_right(&[1.0, 0.05, 0.0, 1.0, 0.0, 0.0], &I));
    }

    use super::*;

    /// Font statistics for a document whose body text is 12pt.
    fn body_12pt_stats(heading_sizes: &[f32]) -> FontStatistics {
        let mut stats = FontStatistics::default();
        for _ in 0..100 {
            stats.add_size(12.0);
        }
        for size in heading_sizes {
            for _ in 0..3 {
                stats.add_size(*size);
            }
        }
        stats.analyze();
        stats
    }

    fn line_at(text: &str, y: f32, font_size: f32, font: &str) -> TextLine {
        TextLine::from_spans(vec![TextSpan::new(
            text.to_string(),
            0.0,
            y,
            font_size,
            font.to_string(),
        )])
    }

    /// A line of spans with widths assigned, as they are by the time lines are grouped.
    fn span_line(parts: &[(&str, f32, f32, f32)]) -> TextLine {
        TextLine::from_spans(
            parts
                .iter()
                .map(|&(text, x, y, size)| TextSpan {
                    width: estimate_text_width(text, size),
                    width_measured: false,
                    ..TextSpan::new(text.to_string(), x, y, size, "Helvetica".to_string())
                })
                .collect(),
        )
    }

    /// "3. RECOLLECTION OF NATIONAL INITIATIVES" as a word processor fakes small caps: the
    /// number and each word's initial at 14 pt, the rest of each word as 11 pt capitals.
    fn faux_small_caps_title() -> Vec<(&'static str, f32, f32, f32)> {
        let runs = [
            ("3. ", 102.0, 14.0),
            ("R", 102.0, 14.0),
            ("ECOLLECTION OF ", 101.3, 11.0),
            ("N", 102.0, 14.0),
            ("ATIONAL ", 101.3, 11.0),
            ("I", 102.0, 14.0),
            ("NITIATIVES", 101.3, 11.0),
        ];
        // Each run starts where the one before it ends, as the glyphs are set.
        let mut x = 121.0;
        runs.iter()
            .map(|&(text, y, size)| {
                let at = x;
                x += estimate_text_width(text, size);
                (text, at, y, size)
            })
            .collect()
    }

    #[test]
    fn a_line_of_faux_small_caps_is_set_at_the_size_of_its_initials() {
        let line = span_line(&faux_small_caps_title());
        assert_eq!(line.font_size, 14.0);
        assert_eq!(line.text(), "3. RECOLLECTION OF NATIONAL INITIATIVES");
    }

    #[test]
    fn mixed_sizes_that_are_not_faux_small_caps_keep_the_weighted_size() {
        // Lowercase anywhere: an ordinary line with a larger word in it.
        let mut parts = faux_small_caps_title();
        parts[2].0 = "ecollection of ";
        assert!(span_line(&parts).font_size < 12.0);
        // A whole word at the larger size is not an initial.
        let mut parts = faux_small_caps_title();
        parts[1].0 = "RE";
        assert!(span_line(&parts).font_size < 12.0);
        // A smaller run that opens a word with no initial before it.
        let mut parts = faux_small_caps_title();
        parts[3].3 = 11.0;
        assert!(span_line(&parts).font_size < 12.0);
        // Sizes too far apart for capitals and small capitals (a 7 pt footnote mark).
        let mut parts = faux_small_caps_title();
        for part in &mut parts {
            if part.3 == 11.0 {
                part.3 = 7.0;
            }
        }
        assert!(span_line(&parts).font_size < 12.0);
    }

    #[test]
    fn a_superscript_line_joins_the_line_below_it() {
        let body = span_line(&[("own right.", 72.0, 686.0, 12.0)]);
        let sup = span_line(&[(
            "[35]",
            72.0 + estimate_text_width("own right.", 12.0),
            690.2,
            9.6,
        )]);
        let lines = attach_script_lines(vec![sup, body]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text(), "own right.[35]");
        assert_eq!(lines[0].y, 686.0, "the merged line keeps the body baseline");
    }

    #[test]
    fn a_small_line_running_across_its_neighbour_is_not_a_script() {
        // A byline set smaller just under a title: close enough and small enough, but it
        // spans the title's text instead of sitting after a word.
        let title = span_line(&[("A Study of Leaf Anatomy", 72.0, 700.0, 14.0)]);
        let byline = span_line(&[("by A. Author and B. Author", 72.0, 694.0, 10.0)]);
        let lines = attach_script_lines(vec![title, byline]);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn a_same_size_line_is_never_a_script() {
        let first = span_line(&[("first line", 72.0, 700.0, 12.0)]);
        let second = span_line(&[("tail", 200.0, 696.0, 12.0)]);
        assert_eq!(attach_script_lines(vec![first, second]).len(), 2);
    }

    #[test]
    fn test_detect_headings_joins_a_chapter_number_to_the_title_below_it() {
        let stats = body_12pt_stats(&[17.0, 25.0]);
        let lines = vec![
            line_at("4", 730.0, 25.0, "Helvetica-Bold"),
            line_at("Basis Fields", 705.0, 17.0, "Helvetica-Bold"),
            line_at(
                "A vector field may be written as a combination.",
                675.0,
                12.0,
                "Helvetica",
            ),
        ];
        let result = LayoutAnalyzer::detect_headings(&stats, lines);
        assert!(result[1].is_heading);
        assert!(result[0].is_heading, "the number opens the title");
        assert!(result[0].heading_level <= result[1].heading_level);
    }

    #[test]
    fn test_detect_headings_leaves_a_number_smaller_than_the_title_alone() {
        // A page number above a heading is set smaller than it.
        let stats = body_12pt_stats(&[17.0]);
        let lines = vec![
            line_at("12", 730.0, 10.0, "Helvetica"),
            line_at("Basis Fields", 715.0, 17.0, "Helvetica-Bold"),
            line_at(
                "A vector field may be written as a combination.",
                690.0,
                12.0,
                "Helvetica",
            ),
        ];
        let result = LayoutAnalyzer::detect_headings(&stats, lines);
        assert!(!result[0].is_heading);
    }

    #[test]
    fn test_detect_headings_leaves_a_number_far_above_the_title_alone() {
        let stats = body_12pt_stats(&[17.0, 25.0]);
        let lines = vec![
            line_at("4", 800.0, 25.0, "Helvetica-Bold"),
            line_at("Basis Fields", 705.0, 17.0, "Helvetica-Bold"),
            line_at(
                "A vector field may be written as a combination.",
                675.0,
                12.0,
                "Helvetica",
            ),
        ];
        let result = LayoutAnalyzer::detect_headings(&stats, lines);
        assert!(!result[0].is_heading);
    }

    #[test]
    fn test_is_bare_section_number() {
        for yes in ["4", "4.", "4.2", "1.2.3", "IV", "XII.", "A", "B."] {
            assert!(is_bare_section_number(yes), "{yes}");
        }
        for no in ["4 Basis", "1999", "page 4", "4.2a", "", "ab", "§4"] {
            assert!(!is_bare_section_number(no), "{no}");
        }
    }

    #[test]
    fn test_detect_headings_promotes_a_lone_larger_line() {
        let stats = body_12pt_stats(&[20.0]);
        let lines = vec![
            line_at("Body text before the heading.", 100.0, 12.0, "Helvetica"),
            line_at("Chapter One", 80.0, 20.0, "Helvetica"),
            line_at("Body text after the heading.", 60.0, 12.0, "Helvetica"),
        ];

        let result = LayoutAnalyzer::detect_headings(&stats, lines);

        assert!(!result[0].is_heading);
        assert!(
            result[1].is_heading,
            "a lone 20pt line among 12pt body text"
        );
        assert!(result[1].heading_level > 0);
        assert!(!result[2].is_heading);
    }

    /// A line that opens with a list marker is an enumeration inside body content, whatever
    /// its font size. Enclosed enumerations count: Korean documents use them for choices and
    /// clause lists, and a document that sets them in a display face would otherwise turn
    /// every one of them into a heading.
    #[test]
    fn test_detect_headings_skips_list_markers_regardless_of_size() {
        let stats = body_12pt_stats(&[20.0]);
        for marker in ["- ", "• ", "① ", "❶ ", "㉠ ", "※ ", "◦ "] {
            let lines = vec![
                line_at("Body text.", 100.0, 12.0, "Helvetica"),
                line_at(
                    &format!("{marker}An enumerated item"),
                    80.0,
                    20.0,
                    "Helvetica",
                ),
                line_at("Body text.", 60.0, 12.0, "Helvetica"),
            ];

            let result = LayoutAnalyzer::detect_headings(&stats, lines);
            assert!(
                !result[1].is_heading,
                "{marker:?} opens a list item, not a heading"
            );
        }
    }

    /// A dash printed against a digit, or against a decimal point and a digit, is a minus
    /// sign — with the hyphen-minus, the minus sign, the figure and en dashes.
    #[test]
    fn a_dash_against_a_number_is_its_sign() {
        for signed in [
            "-0.2 4.5",
            "-12",
            "-.5",
            "  -3.1",
            "\u{2212}0.8 3.1",
            "\u{2013}0.7",
            "\u{2012}4",
            "\u{FF0D}2",
        ] {
            assert!(opens_with_signed_number(signed), "{signed:?}");
            assert!(!opens_a_list_item(signed), "{signed:?} is not a list item");
            assert!(detect_list_marker(signed).is_none(), "{signed:?}");
        }
    }

    /// A dash with a space after it is a marker whatever follows; so is a dash glued to a
    /// word (a bullet drawn without a gap), and a dash before a lone point. The em dash is
    /// never a sign.
    #[test]
    fn a_dash_before_a_space_or_a_word_is_a_list_marker() {
        for (item, rest) in [
            ("- 2023.1.12일 실시한 점검", "2023.1.12일 실시한 점검"),
            ("\u{2013} 0.3 하락", "0.3 하락"),
            ("-외화 유동성 점검", "외화 유동성 점검"),
            ("-item", "item"),
            ("-. trailing", ". trailing"),
            ("\u{2014}5 items", "5 items"),
        ] {
            assert!(!opens_with_signed_number(item), "{item:?}");
            let m = detect_list_marker(item).unwrap_or_else(|| panic!("{item:?} is a bullet"));
            assert_eq!(m.ordered_number, None);
            assert_eq!(&item[m.prefix_len..], rest);
        }
        assert!(!opens_with_signed_number("-"));
        assert!(!opens_with_signed_number(""));
        assert!(!opens_with_signed_number("5"));
    }

    /// A large line that opens with a negative number is not kept from being a heading as
    /// if it were a list item — the exclusion is for list items.
    #[test]
    fn test_detect_headings_does_not_take_a_signed_number_for_a_list_marker() {
        let stats = body_12pt_stats(&[20.0]);
        let lines = vec![
            line_at("Body text.", 100.0, 12.0, "Helvetica"),
            line_at("-3.9% growth forecast", 80.0, 20.0, "Helvetica"),
            line_at("Body text.", 60.0, 12.0, "Helvetica"),
        ];
        let result = LayoutAnalyzer::detect_headings(&stats, lines);
        assert!(result[1].is_heading, "the size makes it a heading");
    }

    #[test]
    fn test_detect_list_marker_bullet() {
        let m = detect_list_marker("- first item").expect("bullet should match");
        assert_eq!(m.ordered_number, None);
        assert_eq!(&"- first item"[m.prefix_len..], "first item");
    }

    #[test]
    fn test_detect_list_marker_ordered() {
        let m = detect_list_marker("12) twelfth item").expect("ordered marker should match");
        assert_eq!(m.ordered_number, Some(12));
        assert_eq!(&"12) twelfth item"[m.prefix_len..], "twelfth item");
    }

    #[test]
    fn test_detect_list_marker_rejects_a_decimal_number() {
        assert!(detect_list_marker("1.5cm gap").is_none());
    }

    #[test]
    fn test_detect_list_marker_rejects_a_four_digit_year() {
        assert!(detect_list_marker("2024. 01. 15").is_none());
    }

    #[test]
    fn test_detect_list_marker_rejects_plain_prose() {
        assert!(detect_list_marker("Regular paragraph text.").is_none());
    }

    #[test]
    fn test_finish_block_classifies_bullet_as_list_item_with_marker_stripped() {
        let lines = vec![line_at("• Buy milk", 100.0, 12.0, "Helvetica")];
        let block = LayoutAnalyzer::finish_block(lines);
        assert_eq!(block.block_type, BlockType::ListItem);
        assert_eq!(block.list_item_number, None);
        assert_eq!(block.list_item_text(), "Buy milk");
    }

    #[test]
    fn test_finish_block_classifies_ordered_marker_as_list_item_with_number() {
        let lines = vec![line_at("3. Third step", 100.0, 12.0, "Helvetica")];
        let block = LayoutAnalyzer::finish_block(lines);
        assert_eq!(block.block_type, BlockType::ListItem);
        assert_eq!(block.list_item_number, Some(3));
        assert_eq!(block.list_item_text(), "Third step");
    }

    /// The second half of a page range wrapped onto its own line reads like an
    /// ordered marker (`306.`). Nothing continues it, so it is the end of the
    /// previous item — as an item of its own it was stripped to nothing and lost.
    #[test]
    fn test_push_block_keeps_a_lone_number_with_the_block_before() {
        let mut blocks = Vec::new();
        let item = |text: &str, y: f32| vec![line_at(text, y, 9.0, "Times")];
        LayoutAnalyzer::push_block(
            &mut blocks,
            item("24. Massagué J. Nature 2004; 432: 298-", 600.0),
            14.0,
        );
        LayoutAnalyzer::push_block(&mut blocks, item("306.", 586.0), 14.0);
        LayoutAnalyzer::push_block(
            &mut blocks,
            item("25. Wakefield LM, Roberts AB.", 572.0),
            14.0,
        );

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].list_item_number, Some(24));
        assert!(
            blocks[0].text().ends_with("298- 306."),
            "{}",
            blocks[0].text()
        );
        assert_eq!(blocks[1].list_item_number, Some(25));
    }

    /// A marker on its own line whose item text follows on the next lines is still
    /// an item: those lines joined its block before `push_block` saw it.
    #[test]
    fn test_push_block_keeps_a_marker_line_that_its_text_continues() {
        let mut blocks = Vec::new();
        LayoutAnalyzer::push_block(
            &mut blocks,
            vec![line_at("Intro text.", 600.0, 9.0, "Times")],
            14.0,
        );
        LayoutAnalyzer::push_block(
            &mut blocks,
            vec![
                line_at("01.", 586.0, 9.0, "Times"),
                line_at("Item text", 572.0, 9.0, "Times"),
            ],
            14.0,
        );

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].block_type, BlockType::ListItem);
        assert_eq!(blocks[1].list_item_number, Some(1));
    }

    /// A lone number far below the block before is not its continuation.
    /// A page with a title, two lines of text and a figure: the gap under the title is almost
    /// twice the text's pitch, but the jump over the figure makes the page's spacings no guide.
    #[test]
    fn a_title_set_off_from_the_lines_around_it_is_a_block_of_its_own() {
        let lines = vec![
            line_at("Print vs. Digital", 727.0, 11.5, "Montserrat"),
            line_at(
                "Why do some researchers abhor digital and favor print,",
                694.6,
                11.5,
                "Montserrat",
            ),
            line_at(
                "or vice-versa? The classic debate was necessary.",
                677.3,
                11.5,
                "Montserrat",
            ),
            line_at("format.", 402.8, 11.5, "Montserrat"),
        ];
        let blocks = LayoutAnalyzer::group_lines_into_blocks(lines);
        let texts: Vec<String> = blocks.iter().map(TextBlock::text).collect();
        assert_eq!(texts.len(), 3, "{texts:?}");
        assert_eq!(texts[0], "Print vs. Digital");
    }

    /// Double spacing is a pitch, not a paragraph break, however wide it is.
    #[test]
    fn evenly_double_spaced_lines_stay_one_block() {
        let lines: Vec<TextLine> = (0..6)
            .map(|i| {
                line_at(
                    "a line of double-spaced text",
                    700.0 - 26.0 * i as f32,
                    12.0,
                    "Times",
                )
            })
            .collect();
        assert_eq!(LayoutAnalyzer::group_lines_into_blocks(lines).len(), 1);
    }

    #[test]
    fn the_typical_line_spacing_is_not_pulled_up_by_a_jump_over_a_figure() {
        let lines = vec![
            line_at("one", 700.0, 11.0, "Times"),
            line_at("two", 685.0, 11.0, "Times"),
            line_at("three", 670.0, 11.0, "Times"),
            line_at("four", 400.0, 11.0, "Times"),
        ];
        assert_eq!(LayoutAnalyzer::calculate_avg_line_spacing(&lines), 15.0);
    }

    #[test]
    fn a_block_of_one_line_of_bold_capitals_is_a_heading_however_long() {
        let title = "ARE CIGARETTE SMOKERS HYPERBOLIC TIME DISCOUNTERS?";
        let block = LayoutAnalyzer::finish_block(vec![line_at(title, 500.0, 11.0, "Lato-Bold")]);
        assert_eq!(block.block_type, BlockType::Heading);
        // Not bold, or not all capitals: a line of its own, not a title.
        for (text, font) in [
            (title, "Lato"),
            ("Are Cigarette Smokers Discounters?", "Lato-Bold"),
        ] {
            let block = LayoutAnalyzer::finish_block(vec![line_at(text, 500.0, 11.0, font)]);
            assert_eq!(block.block_type, BlockType::Paragraph, "{text} in {font}");
        }
    }

    #[test]
    fn test_push_block_does_not_reach_across_a_paragraph_gap() {
        let mut blocks = Vec::new();
        LayoutAnalyzer::push_block(
            &mut blocks,
            vec![line_at("Body text.", 600.0, 9.0, "Times")],
            14.0,
        );
        LayoutAnalyzer::push_block(&mut blocks, vec![line_at("12.", 500.0, 9.0, "Times")], 14.0);
        assert_eq!(blocks.len(), 2);
        // Kept as text, not as an item that strips to nothing.
        assert_eq!(blocks[1].block_type, BlockType::Paragraph);
        assert_eq!(blocks[1].text(), "12.");
    }

    /// A numbered *heading* ("1. Introduction", large font) must stay a heading —
    /// `detect_headings` already decided that before `finish_block` ever runs, and an
    /// ordered-marker line isn't excluded there the way a bullet line is (see the doc
    /// comment on `finish_block`).
    #[test]
    fn test_finish_block_a_numbered_heading_stays_a_heading() {
        let mut line = line_at("1. Introduction", 100.0, 20.0, "Helvetica");
        line.is_heading = true;
        line.heading_level = 1;
        let block = LayoutAnalyzer::finish_block(vec![line]);
        assert_eq!(block.block_type, BlockType::Heading);
    }

    /// Sibling lines of the same size are a run — a table column, a list, a paragraph whose
    /// body font drifts between cells — and a heading normally sits alone in its size cohort.
    #[test]
    fn test_detect_headings_suppresses_a_line_with_a_same_size_neighbour() {
        let stats = body_12pt_stats(&[16.0]);
        let lines = vec![
            line_at("Body text.", 100.0, 12.0, "Helvetica"),
            line_at("Cell one of a row", 80.0, 16.0, "Helvetica"),
            line_at("Cell two of a row", 60.0, 16.0, "Helvetica"),
        ];

        let result = LayoutAnalyzer::detect_headings(&stats, lines);

        assert!(
            !result[1].is_heading && !result[2].is_heading,
            "16pt is under body + 6.0, so a same-size neighbour suppresses both"
        );
    }

    /// Two titles of one size with a table taken out between them are next to each other in
    /// the list of lines, but not on the page: neither is the other's sibling. Without the
    /// table between them they still are.
    #[test]
    fn test_detect_headings_keeps_same_size_titles_a_table_stands_between() {
        let stats = body_12pt_stats(&[16.0]);
        let lines = || {
            vec![
                line_at("Table 13.2. Effect of cations", 700.0, 16.0, "Helvetica"),
                line_at("Activity 4. Determining CEC", 560.0, 16.0, "Helvetica"),
                line_at("Body text follows the title.", 540.0, 12.0, "Helvetica"),
            ]
        };
        let result = LayoutAnalyzer::detect_headings_around(&stats, lines(), &[690.0]);
        assert!(result[0].is_heading && result[1].is_heading);
        let result = LayoutAnalyzer::detect_headings(&stats, lines());
        assert!(!result[0].is_heading && !result[1].is_heading);
    }

    /// A title wrapped over two lines is one heading, not two siblings: the second line opens
    /// in lowercase or a bracket, or the first breaks after a word no title ends on.
    #[test]
    fn test_detect_headings_keeps_a_title_wrapped_over_two_lines() {
        let stats = body_12pt_stats(&[16.0]);
        for (first, second) in [
            ("Observations from the Spitzer Space Telescope", "(SST)."),
            ("Balance of wood pellets and", "Cost Structure"),
            ("Supply and demand of wood", "pellets in Japan"),
        ] {
            let lines = vec![
                line_at("Body text before.", 120.0, 12.0, "Helvetica"),
                line_at(first, 100.0, 16.0, "Helvetica"),
                line_at(second, 82.0, 16.0, "Helvetica"),
                line_at("Body text after.", 60.0, 12.0, "Helvetica"),
            ];
            let result = LayoutAnalyzer::detect_headings(&stats, lines);
            assert!(
                result[1].is_heading && result[2].is_heading,
                "{first} / {second}"
            );
        }
    }

    /// A long line over a lowercase one is a paragraph going on, not a wrapped title — and a
    /// second line longer than the first is not how a title wraps.
    #[test]
    fn test_detect_headings_still_suppresses_lines_that_do_not_wrap_like_a_title() {
        let stats = body_12pt_stats(&[16.0]);
        for (first, second) in [
            (
                // Two columns read as one line: a heading on the left, body text on the right.
                "6.2. Expectations for Re-Hiring Employees they had no plans to re-hire and another 36% said",
                "they did not know whether they would re-hire",
            ),
            ("Short and", "a second line that runs on much longer than the first"),
        ] {
            let lines = vec![
                line_at(first, 100.0, 16.0, "Helvetica"),
                line_at(second, 82.0, 16.0, "Helvetica"),
            ];
            let result = LayoutAnalyzer::detect_headings(&stats, lines);
            assert!(!result[0].is_heading && !result[1].is_heading, "{first} / {second}");
        }
    }

    /// A section number set as a line of its own is not a same-size sibling of its title.
    #[test]
    fn test_detect_headings_does_not_let_a_bare_section_number_suppress_its_title() {
        let stats = body_12pt_stats(&[16.0]);
        let lines = vec![
            line_at("3.", 101.0, 16.0, "Helvetica"),
            line_at(
                "RECOLLECTION OF NATIONAL INITIATIVES",
                100.0,
                16.0,
                "Helvetica",
            ),
            line_at("Body text.", 80.0, 12.0, "Helvetica"),
        ];
        let result = LayoutAnalyzer::detect_headings(&stats, lines);
        assert!(result[1].is_heading);
    }

    /// The suppression above has an escape hatch: a line far enough above body size is a
    /// heading even next to another of its size, or a document with two adjacent headings
    /// would lose both.
    #[test]
    fn test_detect_headings_keeps_large_lines_despite_a_same_size_neighbour() {
        let stats = body_12pt_stats(&[24.0]);
        let lines = vec![
            line_at("Part I", 100.0, 24.0, "Helvetica"),
            line_at("Chapter One", 80.0, 24.0, "Helvetica"),
            line_at("Body text.", 60.0, 12.0, "Helvetica"),
        ];

        let result = LayoutAnalyzer::detect_headings(&stats, lines);

        assert!(result[0].is_heading && result[1].is_heading);
    }

    /// Uppercase stands in for bold. A PDF whose heading font has no bold variant marks the
    /// heading by setting it in capitals, and that is the only signal available.
    #[test]
    fn test_detect_headings_treats_uppercase_as_bold() {
        // 14pt is body + 2.0: short of the plain threshold (+2.5), past the bold one (+1.5).
        let stats = body_12pt_stats(&[14.0]);
        let mixed = LayoutAnalyzer::detect_headings(
            &stats,
            vec![
                line_at("Body text.", 100.0, 12.0, "Helvetica"),
                line_at("Introduction here", 80.0, 14.0, "Helvetica"),
                line_at("Body text.", 60.0, 12.0, "Helvetica"),
            ],
        );
        assert!(
            !mixed[1].is_heading,
            "14pt alone does not reach the plain threshold"
        );

        let upper = LayoutAnalyzer::detect_headings(
            &stats,
            vec![
                line_at("Body text.", 100.0, 12.0, "Helvetica"),
                line_at("INTRODUCTION HERE", 80.0, 14.0, "Helvetica"),
                line_at("Body text.", 60.0, 12.0, "Helvetica"),
            ],
        );
        assert!(
            upper[1].is_heading,
            "the same size in capitals qualifies through the bold threshold"
        );
    }

    /// A short line in the body face's bold, between plain lines, titles what follows.
    #[test]
    fn test_detect_headings_promotes_a_bold_run_in_title() {
        let stats = body_12pt_stats(&[20.0]);
        for title in [
            "Procedure:",
            "Steps for Using the Microscope",
            "Our Mission",
        ] {
            let result = LayoutAnalyzer::detect_headings(
                &stats,
                vec![
                    line_at("Body text before it.", 100.0, 12.0, "Helvetica"),
                    line_at(title, 80.0, 12.0, "Helvetica-Bold"),
                    line_at("Body text after it.", 60.0, 12.0, "Helvetica"),
                ],
            );
            assert!(result[1].is_heading, "{title:?}");
            assert_eq!(result[1].heading_level, 3, "{title:?}");
        }
    }

    /// What a bold run-in title is not: a sentence, a line among bold lines (names, a bold
    /// paragraph, a contents page's entries), a line over another bold line, or a long one.
    #[test]
    fn test_detect_headings_leaves_other_bold_lines_as_text() {
        let stats = body_12pt_stats(&[20.0]);
        let cases: [(&str, &str, &str, &str); 4] = [
            (
                "Helvetica",
                "Note that this is a sentence.",
                "Helvetica",
                "a sentence",
            ),
            (
                "Helvetica-Bold",
                "Campaign",
                "Helvetica",
                "after a bold line",
            ),
            (
                "Helvetica",
                "Campaign",
                "Helvetica-Bold",
                "before a bold line",
            ),
            (
                "Helvetica",
                "A bold line that runs on for well over a dozen words of text here",
                "Helvetica",
                "a long line",
            ),
        ];
        for (before, title, after, why) in cases {
            let result = LayoutAnalyzer::detect_headings(
                &stats,
                vec![
                    line_at("Body text before it", 100.0, 12.0, before),
                    line_at(title, 80.0, 12.0, "Helvetica-Bold"),
                    line_at("Body text after it", 60.0, 12.0, after),
                ],
            );
            assert!(!result[1].is_heading, "{why}");
        }
    }

    /// A numbered section title set in the body face's bold at body size: no size signal,
    /// but the section number plus a line that is bold throughout marks it. The level
    /// follows the number's depth.
    #[test]
    fn test_detect_headings_promotes_a_bold_numbered_section_title() {
        let stats = body_12pt_stats(&[20.0]);
        for (title, level) in [
            ("5. The dynamics", 2),
            ("12 Conclusion", 2),
            ("III. Regulatory cholesterol", 2),
            ("3.1. Status of Business Operations", 3),
            ("3.2.6. SDGs Dissemination in Social Media", 4),
            ("01 - Find Open Educational Resources", 2),
            ("02- Prepare Your Content", 2),
            ("3) Results", 2),
        ] {
            let lines = vec![
                line_at("Body text before.", 100.0, 12.0, "Helvetica"),
                line_at(title, 80.0, 12.0, "Helvetica-Bold"),
                line_at("Body text after.", 60.0, 12.0, "Helvetica"),
            ];
            let result = LayoutAnalyzer::detect_headings(&stats, lines);
            assert!(result[1].is_heading, "{title:?} is a section title");
            assert_eq!(result[1].heading_level, level, "{title:?}");
        }
    }

    /// The same shapes that are not section titles: a numbered sentence, a number
    /// that starts running text, a year, the plain face, and a bold lead-in on a
    /// line that continues in the regular face.
    #[test]
    fn test_detect_headings_leaves_numbered_body_lines_alone() {
        let stats = body_12pt_stats(&[20.0]);
        let bold = |t: &str| line_at(t, 80.0, 12.0, "Helvetica-Bold");
        let lead_in = TextLine::from_spans(vec![
            TextSpan::new(
                "2. Definition.".into(),
                0.0,
                80.0,
                12.0,
                "Helvetica-Bold".into(),
            ),
            TextSpan::new(
                " A universe is a chain of states with an extra property".into(),
                90.0,
                80.0,
                12.0,
                "Helvetica".into(),
            ),
        ]);
        for line in [
            bold("1. Install the package and run it."),
            bold("12 apples were sold that day"),
            bold("2024 Annual Report"),
            line_at("5. The dynamics", 80.0, 12.0, "Helvetica"),
            lead_in,
        ] {
            let text = line.text();
            let lines = vec![
                line_at("Body text before.", 100.0, 12.0, "Helvetica"),
                line,
                line_at("Body text after.", 60.0, 12.0, "Helvetica"),
            ];
            let result = LayoutAnalyzer::detect_headings(&stats, lines);
            assert!(!result[1].is_heading, "{text:?} is not a section title");
        }
    }

    /// Too few visible characters to be a heading — page furniture, a stray glyph, a rule.
    #[test]
    fn test_detect_headings_skips_lines_with_almost_no_text() {
        let stats = body_12pt_stats(&[20.0]);
        let lines = vec![
            line_at("Body text.", 100.0, 12.0, "Helvetica"),
            line_at("A.", 80.0, 20.0, "Helvetica"),
            line_at("Body text.", 60.0, 12.0, "Helvetica"),
        ];

        let result = LayoutAnalyzer::detect_headings(&stats, lines);

        assert!(!result[1].is_heading);
    }

    #[test]
    fn test_font_statistics() {
        let mut stats = FontStatistics::default();
        // Simulate body text (most common)
        for _ in 0..100 {
            stats.add_size(12.0);
        }
        // Simulate headings
        for _ in 0..5 {
            stats.add_size(18.0);
        }
        for _ in 0..3 {
            stats.add_size(24.0);
        }

        stats.analyze();

        assert!((stats.body_size - 12.0).abs() < 0.1);
        assert_eq!(stats.get_heading_level(12.0, false), 0);
        assert!(stats.get_heading_level(18.0, false) > 0);
        assert!(stats.get_heading_level(24.0, false) > 0);
    }

    #[test]
    fn test_text_span_bold_detection() {
        let span = TextSpan::new(
            "Test".to_string(),
            0.0,
            0.0,
            12.0,
            "Helvetica-Bold".to_string(),
        );
        assert!(span.is_bold);
        assert!(!span.is_italic);

        let span2 = TextSpan::new(
            "Test".to_string(),
            0.0,
            0.0,
            12.0,
            "Helvetica-Oblique".to_string(),
        );
        assert!(!span2.is_bold);
        assert!(span2.is_italic);
    }

    #[test]
    fn test_merge_fragmented_spans_single_chars() {
        // Simulate per-character rendering: "Hello" as 5 separate spans
        let spans: Vec<TextSpan> = "Hello"
            .chars()
            .enumerate()
            .map(|(i, c)| TextSpan {
                text: c.to_string(),
                x: 100.0 + i as f32 * 6.0,
                y: 500.0,
                width: 0.0, // width=0 is the fragmentation signal
                width_measured: false,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            })
            .collect();

        let merged = merge_fragmented_spans(spans);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Hello");
    }

    #[test]
    fn test_merge_fragmented_spans_preserves_normal() {
        // Normal multi-character spans should not be merged unnecessarily
        let spans = vec![
            TextSpan {
                text: "Hello".to_string(),
                x: 100.0,
                y: 500.0,
                width: 30.0,
                width_measured: true,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            },
            TextSpan {
                text: "World".to_string(),
                x: 145.0, // gap indicates word space
                y: 500.0,
                width: 30.0,
                width_measured: true,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            },
        ];

        let merged = merge_fragmented_spans(spans);
        // Should not merge because fragmentation threshold is not met
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_attach_word_space_appends_once_on_the_same_baseline() {
        let mut spans = vec![TextSpan::new(
            "Hello".to_string(),
            0.0,
            500.0,
            12.0,
            "Helvetica".to_string(),
        )];

        attach_word_space(&mut spans, 0.0, 500.0, 12.0, None);
        assert_eq!(spans[0].text, "Hello ");

        attach_word_space(&mut spans, 0.0, 500.0, 12.0, None);
        assert_eq!(spans[0].text, "Hello ", "must not accumulate spaces");
    }

    #[test]
    fn test_attach_word_space_ignores_a_different_baseline() {
        let mut spans = vec![TextSpan::new(
            "Hello".to_string(),
            0.0,
            500.0,
            12.0,
            "Helvetica".to_string(),
        )];

        attach_word_space(&mut spans, 0.0, 400.0, 12.0, None);
        assert_eq!(spans[0].text, "Hello", "indentation is not a word gap");
    }

    #[test]
    fn test_merge_fragmented_spans_does_not_absorb_after_a_word_space() {
        let chars = ["H", "e", "l", "l", "o ", "W"];
        let xs = [0.0, 6.0, 12.0, 18.0, 24.0, 40.0];
        let spans: Vec<TextSpan> = chars
            .iter()
            .zip(xs)
            .map(|(c, x)| TextSpan::new(c.to_string(), x, 500.0, 12.0, "Helvetica".to_string()))
            .collect();

        let merged = merge_fragmented_spans(spans);

        assert_eq!(
            merged.len(),
            2,
            "a fragment ending in a space is a finished word"
        );
        assert_eq!(merged[0].text, "Hello ");
        assert_eq!(merged[1].text, "W");
    }

    /// Helper: a span at a given x/width, dots leader spans included.
    fn span_at(text: &str, x: f32, width: f32) -> TextSpan {
        TextSpan {
            width,
            width_measured: true,
            ..TextSpan::new(text.to_string(), x, 500.0, 12.0, "Helvetica".to_string())
        }
    }

    #[test]
    fn test_normalize_dot_leaders_drops_leader_keeps_page_number() {
        let spans = vec![
            span_at("Chapter 1", 100.0, 54.0),
            span_at("....................", 160.0, 120.0),
            span_at("6", 290.0, 6.0),
        ];

        let result = normalize_dot_leaders(spans);

        assert_eq!(result.len(), 2, "leader span dropped, title and page kept");
        assert_eq!(result[0].text, "Chapter 1");
        assert_eq!(result[1].text, "(p.6)");
    }

    #[test]
    fn test_normalize_dot_leaders_drops_leader_without_page_number() {
        let spans = vec![
            span_at("Introduction", 100.0, 70.0),
            span_at("............................", 180.0, 150.0),
        ];

        let result = normalize_dot_leaders(spans);

        assert_eq!(result.len(), 1, "leader dropped, nothing follows to keep");
        assert_eq!(result[0].text, "Introduction");
    }

    #[test]
    fn test_normalize_dot_leaders_merges_a_split_leader_run() {
        // Some backends emit the leader as several adjacent dot-only spans rather
        // than one — the run must still be treated as a single leader.
        let spans = vec![
            span_at("Chapter 1", 100.0, 54.0),
            span_at("..........", 160.0, 60.0),
            span_at("..........", 220.0, 60.0),
            span_at("6", 290.0, 6.0),
        ];

        let result = normalize_dot_leaders(spans);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "Chapter 1");
        assert_eq!(result[1].text, "(p.6)");
    }

    #[test]
    fn test_normalize_dot_leaders_inline_between_two_titles() {
        // "Chapter 1 .......... Chapter 2" — a leader with no page number on either
        // side, e.g. two TOC entries whose own line-joining put them on one line.
        let spans = vec![
            span_at("Chapter 1", 100.0, 54.0),
            span_at("....................", 160.0, 120.0),
            span_at("Chapter 2", 290.0, 54.0),
        ];

        let result = normalize_dot_leaders(spans);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "Chapter 1");
        assert_eq!(result[1].text, "Chapter 2");
    }

    #[test]
    fn test_normalize_dot_leaders_preserves_short_prose_ellipsis() {
        // A 3-dot span (e.g. a quoted trailing pause) is under the 4-dot floor and
        // must survive untouched — mirrors render::cleanup's own prose guard.
        let spans = vec![span_at("Wait", 100.0, 24.0), span_at("...", 130.0, 10.0)];

        let result = normalize_dot_leaders(spans.clone());

        assert_eq!(result.len(), spans.len());
        assert_eq!(result[1].text, "...");
    }

    #[test]
    fn test_normalize_dot_leaders_ignores_non_numeric_span_after_run() {
        // A leader followed by more prose text (not a page number) — drop the
        // leader, leave the following span exactly as it was.
        let spans = vec![
            span_at("Section", 100.0, 50.0),
            span_at("........", 155.0, 60.0),
            span_at("Appendix", 220.0, 60.0),
        ];

        let result = normalize_dot_leaders(spans);

        assert_eq!(result.len(), 2);
        assert_eq!(result[1].text, "Appendix");
    }

    #[test]
    fn test_from_spans_renders_toc_entry_end_to_end() {
        let line = TextLine::from_spans(vec![
            span_at("Chapter 1", 100.0, 54.0),
            span_at("....................", 160.0, 120.0),
            span_at("6", 290.0, 6.0),
        ]);

        assert_eq!(line.text(), "Chapter 1 (p.6)");
    }

    #[test]
    fn test_from_spans_leader_only_line_is_empty() {
        // A pure divider line of dots with no title and no page number — decorative
        // noise, not content. Must not panic on an all-filtered span list.
        let line =
            TextLine::from_spans(vec![span_at("............................", 100.0, 150.0)]);

        assert_eq!(line.text(), "");
        assert!(line.spans.is_empty());
    }

    /// A page set like a report with an outer-edge folio: a body column from x 68 to 255 in
    /// 9.3pt type, and `extra` runs beside it.
    fn margin_page(extra: Vec<TextSpan>) -> Vec<TextSpan> {
        let run = |text: &str, x: f32, y: f32, width: f32| TextSpan {
            width,
            width_measured: true,
            ..TextSpan::new(text.to_string(), x, y, 9.3, "Body".to_string())
        };
        let mut spans = vec![
            run("국내", 68.0, 560.0, 18.6),
            run("경제는 소비 회복세가 더디고", 92.0, 546.0, 163.0),
            run("건설투자가 부진하겠지만", 68.0, 532.0, 120.0),
        ];
        spans.extend(extra);
        spans
    }

    const REPORT_PAGE: super::super::backend::PageBox = super::super::backend::PageBox {
        llx: 0.0,
        lly: 0.0,
        urx: 533.0,
        ury: 728.0,
    };

    fn number(text: &str, x: f32, y: f32, size: f32) -> TextSpan {
        TextSpan {
            width: estimate_text_width(text, size),
            width_measured: true,
            ..TextSpan::new(text.to_string(), x, y, size, "Body".to_string())
        }
    }

    fn texts(spans: &[TextSpan]) -> Vec<&str> {
        spans.iter().map(|s| s.text.trim()).collect()
    }

    /// The folio on the outer edge shares a body line's baseline; it goes, as a folio at the
    /// foot of the page does.
    #[test]
    fn a_page_number_in_the_side_margin_is_removed() {
        let mut spans = margin_page(vec![
            number("46", 30.0, 546.0, 8.6),
            number("47", 495.0, 546.0, 8.6),
        ]);
        filter_header_footer_spans(&mut spans, REPORT_PAGE);
        assert_eq!(
            texts(&spans),
            [
                "국내",
                "경제는 소비 회복세가 더디고",
                "건설투자가 부진하겠지만"
            ]
        );
    }

    /// A section number hanging in the margin beside its bold heading stays with it.
    #[test]
    fn a_section_number_hanging_beside_its_heading_is_kept() {
        let mut heading =
            TextSpan::new("Results".to_string(), 68.0, 600.0, 9.3, "Body-Bold".into());
        heading.width = 40.0;
        heading.width_measured = true;
        let mut spans = margin_page(vec![heading, number("3", 50.0, 600.0, 9.3)]);
        filter_header_footer_spans(&mut spans, REPORT_PAGE);
        assert!(texts(&spans).contains(&"3"));
    }

    /// Numbers down a margin beside many lines are line numbers, not a folio.
    #[test]
    fn line_numbers_down_the_margin_are_kept() {
        let mut spans = margin_page(
            (0..3)
                .map(|i| number(&(i + 1).to_string(), 40.0, 560.0 - i as f32 * 14.0, 8.6))
                .collect(),
        );
        filter_header_footer_spans(&mut spans, REPORT_PAGE);
        assert_eq!(texts(&spans).iter().filter(|t| t.len() == 1).count(), 3);
    }

    /// A number inside the text's extent — a table's cell, a figure's label — is text.
    #[test]
    fn a_number_within_the_text_is_kept() {
        let mut spans = margin_page(vec![number("46", 200.0, 518.0, 9.3)]);
        filter_header_footer_spans(&mut spans, REPORT_PAGE);
        assert!(texts(&spans).contains(&"46"));
    }
}

/// What a page's rules say about tables: grids ruled on both axes, and stacks of horizontal
/// rules that share an extent (tables ruled across but not down — see `ruled_rows`).
#[derive(Debug, Default)]
pub(crate) struct PageRulings {
    pub grids: Vec<super::lattice::LatticeGrid>,
    pub rule_stacks: Vec<super::ruled_rows::RuleStack>,
}
