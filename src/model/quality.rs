//! Extraction quality diagnostics.

use serde::{Deserialize, Serialize};

/// Metrics describing the quality of text extraction from a PDF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractionQuality {
    /// Total number of characters in the extracted text.
    pub char_count: usize,

    /// Total number of whitespace-delimited words.
    pub word_count: usize,

    /// Number of U+FFFD replacement characters (indicates decoding failures).
    pub replacement_char_count: usize,

    /// Whether the source PDF was encrypted.
    pub encrypted: bool,

    /// Whether the PDF appears to be a scanned image (no text layer).
    /// Detected by sampling content stream operators across the first few pages:
    /// image (`Do`) operators present with no text (`Tj`/`TJ`) operators.
    #[serde(default)]
    pub is_scan_pdf: bool,

    /// Number of pages whose OCR text layer was dropped as unreadable.
    ///
    /// A searchable scan puts the OCR result on the page as invisible text. When
    /// that text recognises nothing real it is discarded, leaving the page image
    /// alone. Set `ParseOptions::suppress_low_confidence_ocr` to `false` to keep it.
    #[serde(default)]
    pub suppressed_ocr_pages: usize,

    /// Number of text runs the font decoder could not read and discarded.
    ///
    /// A run is one text string handed to the decoder — a `Tj` operand, or a single
    /// element of a `TJ` array. The decoder drops a run when the font's
    /// codes cannot be resolved to characters — emitting the raw bytes would produce
    /// mojibake, not text. That policy is deliberate, but the dropped runs are content
    /// the document had and the output does not, so the count is reported: a non-zero
    /// value means the extraction is incomplete in a way no other field shows.
    ///
    /// Counted in runs because the discarded text was never decoded — its character
    /// count is unknowable. Treat any non-zero value as "incomplete"; the magnitude
    /// only compares documents to each other, not to a share of the whole.
    #[serde(default)]
    pub suppressed_text_runs: usize,

    /// Whether pages are known to be missing from the output.
    ///
    /// `true` means the parser recovered what it could from a damaged document and some
    /// pages never made it — extraction "succeeded" over an incomplete page set. Callers
    /// that index or archive the result should surface this: a page that silently never
    /// arrived is indistinguishable from a page that never existed.
    ///
    /// Always `false` for intact documents. Derived from [`Self::declared_page_count`]
    /// and [`Self::unresolved_page_nodes`]; unaffected by page-range selection.
    #[serde(default)]
    pub pages_incomplete: bool,

    /// Page count the document declares (root `Pages` `/Count`), when readable.
    ///
    /// Compare against the number of pages actually extracted: a lower extracted count
    /// means pages were lost. `None` means the declaration itself was unreadable, which
    /// is itself a damage signal — the page tree is then reported via
    /// [`Self::unresolved_page_nodes`].
    #[serde(default)]
    pub declared_page_count: Option<u32>,

    /// Page-tree nodes that could not be read, so their pages never reached the output.
    ///
    /// Any non-zero value means **the page set is incomplete**. It is deliberately not a
    /// count of lost pages: one unusable intermediate node drops its whole subtree, so a
    /// single unresolved node can cost one page or a hundred. Do not report it as "N
    /// pages lost".
    #[serde(default)]
    pub unresolved_page_nodes: usize,

    /// Objects the cross-reference table pointed at that could not be loaded.
    ///
    /// Damage indicator only — most skipped objects (fonts, annotations, metadata) cost
    /// no page at all, so this does not imply missing text. [`Self::unresolved_page_nodes`]
    /// is the signal for lost content.
    #[serde(default)]
    pub skipped_object_count: usize,

    /// Embedded images recognized as image XObjects but not extractable in the current
    /// output format (an unsupported color space or bit depth), so they were dropped.
    ///
    /// Distinguishes "this page has no image" from "this page has an image we couldn't
    /// materialize" — a resource inventory with zero entries looks the same either way
    /// unless this is checked.
    #[serde(default)]
    pub unsupported_image_count: usize,
}

impl ExtractionQuality {
    /// Compute quality metrics from extracted text.
    pub fn from_text(text: &str) -> Self {
        Self {
            char_count: text.chars().count(),
            word_count: text.split_whitespace().count(),
            replacement_char_count: text.chars().filter(|&c| c == '\u{FFFD}').count(),
            ..Default::default()
        }
    }

    /// Ratio of replacement characters to total characters (0.0 if empty).
    pub fn replacement_char_ratio(&self) -> f32 {
        if self.char_count == 0 {
            0.0
        } else {
            self.replacement_char_count as f32 / self.char_count as f32
        }
    }

    /// Returns `true` if the extraction produced usable text.
    ///
    /// Criteria: non-empty text with less than 30% replacement characters.
    /// `is_scan_pdf` is intentionally excluded: `char_count == 0` already covers pure
    /// scan PDFs, and mixed PDFs (some scanned pages, some text) should not be penalised.
    pub fn is_good(&self) -> bool {
        self.char_count > 0 && self.replacement_char_ratio() < 0.3
    }

    /// Returns a human-readable warning message if extraction quality is poor, or `None` if good.
    ///
    /// The returned string does NOT include a "Warning:" prefix — callers add their own label.
    pub fn warning_message(&self) -> Option<String> {
        // Reported first, ahead of encryption and empty-text: every other warning here
        // describes text that came out wrong, while this one means text never came out
        // at all — and a caller who only ever sees "PDF is encrypted" on a damaged file
        // has no way to learn that pages went missing.
        if self.pages_incomplete {
            let of_declared = match self.declared_page_count {
                Some(n) => format!(" The document declares {} page(s).", n),
                None => String::new(),
            };
            return Some(format!(
                "Some pages could not be read and are missing from the output: the PDF's \
                 page structure is damaged.{} Extracted content is incomplete.",
                of_declared
            ));
        }
        if self.encrypted {
            return Some(
                "PDF is encrypted. Text extraction may be incomplete or unavailable.".to_string(),
            );
        }
        // Ahead of the empty-text branch: that message lists "unsupported font
        // encoding" among several *guesses*, and a non-zero count here means we
        // observed exactly that and know it. Reporting the guess-list over the
        // observation would be a strict loss of information. A document that
        // suppressed every run reaches both branches, and this is the useful one.
        if self.suppressed_text_runs > 0 {
            return Some(format!(
                "Dropped {} unreadable text run(s): the fonts' character codes could not \
                 be resolved. Extracted text is incomplete.",
                self.suppressed_text_runs
            ));
        }
        if self.char_count == 0 {
            if self.is_scan_pdf {
                return Some(
                    "This PDF appears to be a scanned image (no text layer detected). \
                     OCR processing is required to extract text."
                        .to_string(),
                );
            }
            return Some(
                "No text was extracted. Possible causes: scanned/image-based PDF, \
                 encrypted PDF, unsupported font encoding"
                    .to_string(),
            );
        }
        if self.suppressed_ocr_pages > 0 {
            return Some(format!(
                "Dropped the OCR text layer on {} page(s): the scan's recognised text \
                 was not readable. Use --keep-ocr-text to extract it anyway.",
                self.suppressed_ocr_pages
            ));
        }
        if self.replacement_char_ratio() >= 0.3 {
            return Some(format!(
                "Low extraction quality ({} of {} chars are replacement characters). \
                 The PDF may use unsupported font encodings.",
                self.replacement_char_count, self.char_count
            ));
        }
        // Last: a text-quality problem above is more consequential than a partial image
        // gap, so it takes priority when both are present in the same document.
        if self.unsupported_image_count > 0 {
            return Some(format!(
                "{} embedded image(s) use an unsupported color space or bit depth and were \
                 dropped from the resource inventory.",
                self.unsupported_image_count
            ));
        }
        None
    }
}

/// 페이지 단위로 텍스트를 누적하며 품질 지표를 계산한다.
///
/// 목적: 2298페이지 규모 문서에서 `Document::plain_text()` 를
/// 한 번에 재조립하지 않고, 페이지를 하나씩 흘려보내며 동일한 지표를
/// 얻기 위함.
#[derive(Debug, Default, Clone)]
pub struct QualityAccumulator {
    char_count: usize,
    replacement_char_count: usize,
    word_count: usize,
    last_was_non_ws: bool,
    suppressed_ocr_pages: usize,
    suppressed_text_runs: usize,
    unsupported_image_count: usize,
}

impl QualityAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accumulate(&mut self, text: &str) {
        let mut prev_non_ws = self.last_was_non_ws;
        for c in text.chars() {
            self.char_count += 1;
            if c == '\u{FFFD}' {
                self.replacement_char_count += 1;
            }
            let is_ws = c.is_whitespace();
            if !is_ws && !prev_non_ws {
                self.word_count += 1;
            }
            prev_non_ws = !is_ws;
        }
        self.last_was_non_ws = prev_non_ws;
    }

    /// Record that a page's unreadable OCR text layer was dropped.
    pub fn note_suppressed_ocr_page(&mut self) {
        self.suppressed_ocr_pages += 1;
    }

    /// Record text runs a page lost to a decode the font resolver could not complete.
    pub fn note_suppressed_text_runs(&mut self, runs: usize) {
        self.suppressed_text_runs += runs;
    }

    /// Record images a page recognized as image XObjects but could not extract.
    pub fn note_unsupported_images(&mut self, count: usize) {
        self.unsupported_image_count += count;
    }

    pub fn finalize(self) -> ExtractionQuality {
        ExtractionQuality {
            char_count: self.char_count,
            word_count: self.word_count,
            replacement_char_count: self.replacement_char_count,
            suppressed_ocr_pages: self.suppressed_ocr_pages,
            suppressed_text_runs: self.suppressed_text_runs,
            unsupported_image_count: self.unsupported_image_count,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_matches_from_text_for_single_chunk() {
        let text = "Hello world 안녕 \u{FFFD} test";
        let expected = ExtractionQuality::from_text(text);

        let mut acc = QualityAccumulator::new();
        acc.accumulate(text);
        let got = acc.finalize();

        assert_eq!(got.char_count, expected.char_count);
        assert_eq!(got.word_count, expected.word_count);
        assert_eq!(got.replacement_char_count, expected.replacement_char_count);
    }

    #[test]
    fn accumulator_matches_from_text_for_multi_chunks() {
        let full = "alpha beta gamma\n한글  \u{FFFD}delta";
        let chunks = ["alpha beta ", "gamma\n한글  \u{FFFD}", "delta"];

        let expected = ExtractionQuality::from_text(full);
        let mut acc = QualityAccumulator::new();
        for c in chunks {
            acc.accumulate(c);
        }
        let got = acc.finalize();

        assert_eq!(got.char_count, expected.char_count);
        assert_eq!(got.word_count, expected.word_count);
        assert_eq!(got.replacement_char_count, expected.replacement_char_count);
    }

    #[test]
    fn accumulator_word_count_handles_chunk_boundaries() {
        let expected = ExtractionQuality::from_text("foo bar").word_count;

        let mut a = QualityAccumulator::new();
        a.accumulate("foo");
        a.accumulate(" bar");
        assert_eq!(a.finalize().word_count, expected);

        let mut b = QualityAccumulator::new();
        b.accumulate("foo ");
        b.accumulate("bar");
        assert_eq!(b.finalize().word_count, expected);
    }
}
