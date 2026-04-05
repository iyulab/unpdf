//! BiDi (bidirectional) text reordering for RTL script support.

use unicode_bidi::BidiInfo;
use unicode_normalization::UnicodeNormalization;

/// Check if text contains RTL characters (Arabic, Hebrew, etc.)
pub fn contains_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{0590}'..='\u{05FF}' |  // Hebrew
            '\u{0600}'..='\u{06FF}' |  // Arabic
            '\u{0700}'..='\u{074F}' |  // Syriac
            '\u{0750}'..='\u{077F}' |  // Arabic Supplement
            '\u{0780}'..='\u{07BF}' |  // Thaana
            '\u{07C0}'..='\u{07FF}' |  // NKo
            '\u{0800}'..='\u{083F}' |  // Samaritan
            '\u{0840}'..='\u{085F}' |  // Mandaic
            '\u{08A0}'..='\u{08FF}' |  // Arabic Extended-A
            '\u{FB50}'..='\u{FDFF}' |  // Arabic Presentation Forms-A
            '\u{FE70}'..='\u{FEFF}'    // Arabic Presentation Forms-B
        )
    })
}

/// Apply BiDi reordering to convert from visual to logical order,
/// then normalize Arabic presentation forms via NFKC.
pub fn reorder_bidi(text: &str) -> String {
    if !contains_rtl(text) {
        return text.to_string();
    }

    let bidi_info = BidiInfo::new(text, None);
    let mut result = String::new();

    for para in &bidi_info.paragraphs {
        let line = para.range.clone();
        let reordered = bidi_info.reorder_line(para, line);
        result.push_str(&reordered);
    }

    // Normalize Arabic presentation forms (U+FB50-U+FEFF) to base characters
    result.nfkc().collect()
}
