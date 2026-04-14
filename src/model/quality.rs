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
}

impl ExtractionQuality {
    /// Compute quality metrics from extracted text.
    pub fn from_text(text: &str) -> Self {
        Self {
            char_count: text.chars().count(),
            word_count: text.split_whitespace().count(),
            replacement_char_count: text.chars().filter(|&c| c == '\u{FFFD}').count(),
            encrypted: false,
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
    pub fn is_good(&self) -> bool {
        self.char_count > 0 && self.replacement_char_ratio() < 0.3
    }

    /// Returns a human-readable warning if extraction quality is poor, or `None` if good.
    pub fn warning_message(&self) -> Option<String> {
        if self.encrypted {
            return Some(
                "Warning: PDF is encrypted. Text extraction may be incomplete or unavailable."
                    .to_string(),
            );
        }
        if self.char_count == 0 {
            return Some(
                "Warning: No text was extracted. Possible causes: scanned/image-based PDF, \
                 encrypted PDF, unsupported font encoding"
                    .to_string(),
            );
        }
        if self.replacement_char_ratio() >= 0.3 {
            return Some(format!(
                "Warning: Low extraction quality ({} of {} chars are replacement characters). \
                 The PDF may use unsupported font encodings.",
                self.replacement_char_count, self.char_count
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

    pub fn finalize(self) -> ExtractionQuality {
        ExtractionQuality {
            char_count: self.char_count,
            word_count: self.word_count,
            replacement_char_count: self.replacement_char_count,
            encrypted: false,
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
