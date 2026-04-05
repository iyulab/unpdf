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
