//! Detection of low-confidence OCR text layers.
//!
//! A searchable scan carries an invisible text layer produced by OCR. When the
//! source is a drawing, a stamp, or a poor scan, that layer can be meaningless even
//! though every character decodes correctly — the OCR engine simply recognised
//! nothing real:
//!
//! ```text
//! 검 ,φ 끄 Φ ¸ ㅓ Φ Φ ,φ ∽ ㄱ υ Φ
//! ```
//!
//! This module answers only the statistical half of the question: *does this text
//! read like language?* The structural half — is it an invisible layer over a
//! full-page scan — is decided by the caller. Both are required before any text is
//! dropped, so ordinary visible text can never be suppressed by this heuristic.

/// Below this many characters a page carries too little evidence to judge.
const MIN_CHARS: usize = 40;

/// Thresholds sit roughly 1.4× below the lowest value measured across the test
/// corpus (word-like 0.556, coherent 0.770), while the OCR garbage that motivated
/// this gate scores 0.231 and 0.293. Both must be crossed to call text incoherent.
const MAX_WORD_LIKE_RATIO: f32 = 0.40;
const MAX_COHERENT_CHAR_RATIO: f32 = 0.55;

/// Whether the text reads as meaningless — no word structure and few letters.
pub fn is_incoherent_text(text: &str) -> bool {
    let m = match TextMetrics::of(text) {
        Some(m) => m,
        None => return false,
    };

    m.word_like_ratio < MAX_WORD_LIKE_RATIO && m.coherent_char_ratio < MAX_COHERENT_CHAR_RATIO
}

struct TextMetrics {
    /// Share of whitespace-delimited tokens that are at least two characters long
    /// and contain a letter. Real prose is mostly words; OCR noise is mostly
    /// isolated marks.
    word_like_ratio: f32,
    /// Share of characters that carry meaning in some script, as opposed to the
    /// stray symbols, radicals and isolated jamo that OCR emits when it fails.
    coherent_char_ratio: f32,
}

impl TextMetrics {
    /// `None` when the text is too short to judge.
    fn of(text: &str) -> Option<Self> {
        let chars = text.chars().filter(|c| !c.is_whitespace()).count();
        if chars < MIN_CHARS {
            return None;
        }

        let coherent = text.chars().filter(|c| is_coherent(*c)).count();

        let tokens = text.split_whitespace();
        let (total, word_like) = tokens.fold((0usize, 0usize), |(total, word_like), token| {
            let is_word = token.chars().count() >= 2 && token.chars().any(is_coherent);
            (total + 1, word_like + usize::from(is_word))
        });
        if total == 0 {
            return None;
        }

        Some(Self {
            word_like_ratio: word_like as f32 / total as f32,
            coherent_char_ratio: coherent as f32 / chars as f32,
        })
    }
}

/// A character that carries meaning in running text.
///
/// Deliberately excludes isolated Hangul jamo (`ㄱ`, `ㅓ`), which appear in real
/// Korean only as pronunciation notes but flood failed OCR output, along with the
/// symbol, radical and Greek blocks that OCR falls back to on unrecognised strokes.
fn is_coherent(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0xAC00..=0xD7A3   // Hangul syllables
        | 0x4E00..=0x9FFF // CJK ideographs
        | 0x3040..=0x30FF // Kana
        | 0x0400..=0x04FF // Cyrillic
        | 0x0590..=0x06FF // Hebrew, Arabic
        | 0x0E00..=0x0E7F // Thai
    ) || c.is_ascii_alphanumeric()
        || ((0x00C0..=0x024F).contains(&cp) && c.is_alphabetic()) // Latin with diacritics
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extracted from the Canon SC1011 scan of an engineering drawing that motivated
    /// this gate: every character decodes correctly, none of it means anything.
    const OCR_GARBAGE: &str = "검 ,φ 끄 Φ ¸ ㅓ Φ Φ ,φ ∽ ㄱ υ Φ σ Φ ' OΦ ⊃ O::ⅱ OΦ \
                               ° – ° =→ ↔ :。 , ㅂ ¸ ¸ :'Φ φ 0、 0ㄲ – ∽ 0 '呂 ㅒ 句 ㄱ";

    const KOREAN_PROSE: &str = "이 문서는 제품 설치와 운영 절차를 설명한다. 설치 전에 \
                                시스템 요구사항을 확인하고, 필요한 패키지를 미리 준비한다. \
                                각 단계는 순서대로 수행해야 한다.";

    const ENGLISH_PROSE: &str = "The parser resolves each character code through the \
                                 font's CMap before layout analysis runs. When no CMap \
                                 is available the text is dropped rather than guessed.";

    #[test]
    fn flags_ocr_garbage() {
        assert!(is_incoherent_text(OCR_GARBAGE));
    }

    #[test]
    fn accepts_prose() {
        assert!(!is_incoherent_text(KOREAN_PROSE));
        assert!(!is_incoherent_text(ENGLISH_PROSE));
    }

    /// A drawing's text layer is mostly numbers and units — sparse, but real.
    #[test]
    fn accepts_dimension_labels() {
        let labels = "700A 1K 32EA SS275 SOFF 1200 x 800 mm t=6 SCALE 1:50 REV B \
                      2024-08-12 DWG No. 1927";
        assert!(!is_incoherent_text(labels));
    }

    #[test]
    fn ignores_text_too_short_to_judge() {
        assert!(!is_incoherent_text("φ ∽ ㄱ υ Φ"));
        assert!(!is_incoherent_text(""));
    }
}
