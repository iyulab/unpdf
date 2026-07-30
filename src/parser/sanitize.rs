//! Output hygiene for extracted text.
//!
//! Every string unpdf reports as *text* — page content, document metadata,
//! outline titles, form field names and values — passes through here. The
//! invariant it enforces:
//!
//! > Extracted text contains no C0/C1 control characters other than the
//! > whitespace that text legitimately uses (`\n`, `\r`, `\t`).
//!
//! PDF permits control bytes inside string literals (`\000` is a legal octal
//! escape), and some producers leave NUL padding in the text layer. Accepting
//! such a file is correct — parsing is not the problem. Reporting the control
//! byte back as if it were text is: a raw NUL makes the Markdown/text output a
//! file many downstream sinks reject, and it cannot cross the C ABI at all,
//! where `CString::new` fails and the caller loses the whole page.
//!
//! # Why strip rather than substitute
//!
//! Mapping these to `U+FFFD` would be visible, but `U+FFFD` already means
//! something specific here: [`ExtractionQuality::replacement_char_count`] counts
//! it as evidence of *font decoding failure* and feeds `is_good()`. Injecting it
//! for transport reasons would report clean documents as badly decoded.
//!
//! [`ExtractionQuality::replacement_char_count`]: crate::model::ExtractionQuality::replacement_char_count
//!
//! # Order matters: judge before stripping
//!
//! Control-character *density* is how [`is_likely_binary`] recognises a decode
//! that went wrong — a CID font read as Latin-1 produces mostly control bytes.
//! Sanitising before that judgement would erase the evidence and turn "emit
//! nothing" into "emit garbage-derived letters". So sanitising happens on the
//! way out, after any such decision has been made.
//!
//! [`is_likely_binary`]: super::font::is_likely_binary

/// True for characters that are not text: C0 and C1 control codes, plus DEL.
///
/// `\n`, `\r` and `\t` are excluded — they carry structure, not noise.
fn is_non_text_control(c: char) -> bool {
    match c {
        '\n' | '\r' | '\t' => false,
        // C0 controls and DEL
        '\u{0}'..='\u{1F}' | '\u{7F}' => true,
        // C1 controls. Never legitimate text; a producer that meant CP1252
        // punctuation here has already lost the byte to a mis-chosen encoding,
        // and an invisible control character is not a better answer than none.
        '\u{80}'..='\u{9F}' => true,
        _ => false,
    }
}

/// Enforce the module invariant on a string that is about to be reported as text.
///
/// Allocation-free: the common case (no control characters) returns the string
/// untouched, and removal is done in place.
pub(crate) fn sanitize_extracted_text(mut text: String) -> String {
    if text.chars().any(is_non_text_control) {
        text.retain(|c| !is_non_text_control(c));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_interior_nul() {
        assert_eq!(
            sanitize_extracted_text("HELLO\u{0}WORLD".into()),
            "HELLOWORLD"
        );
    }

    #[test]
    fn keeps_structural_whitespace() {
        let text = "line\nnext\r\ncol\tcol";
        assert_eq!(sanitize_extracted_text(text.into()), text);
    }

    #[test]
    fn strips_other_c0_and_del_and_c1() {
        assert_eq!(
            sanitize_extracted_text("A\u{1}B\u{C}C\u{1B}D\u{7F}E\u{85}F".into()),
            "ABCDEF"
        );
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        // Includes NBSP and CJK, which are text and must survive.
        let text = "한글 ok\u{A0}fine — ünïcode";
        assert_eq!(sanitize_extracted_text(text.into()), text);
    }

    #[test]
    fn utf16be_mistaken_for_latin1_loses_only_the_padding() {
        // A BOM-less UTF-16BE string decoded byte-wise: every other byte is NUL.
        // Stripping leaves the ASCII readable — but the real fix for that input
        // is to decode it as UTF-16BE, not to rely on this net.
        assert_eq!(
            sanitize_extracted_text("\u{0}N\u{0}a\u{0}m\u{0}e".into()),
            "Name"
        );
    }
}
