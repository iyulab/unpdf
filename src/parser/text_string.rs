//! Decoding of PDF *text strings* — document metadata, outline titles, form field names
//! and values (PDF 32000-1 §7.9.2.2).
//!
//! The specification allows two encodings, told apart by a byte-order mark: UTF-16BE
//! preceded by `FE FF`, or PDFDocEncoding. PDF 2.0 adds UTF-8 preceded by `EF BB BF`.
//! Producers do not keep to that, in three ways this module handles:
//!
//! - UTF-16BE without the mark ([`looks_like_bomless_utf16be`]);
//! - UTF-8 without the mark — taken when the bytes are valid UTF-8 and not plain ASCII,
//!   which PDFDocEncoded text practically never is;
//! - a legacy code page of the producer's locale — CP949, Shift_JIS, GBK, Big5 — written
//!   as raw bytes ([`decode_legacy_cjk`]). Korean and Japanese tooling does this routinely
//!   for the document title.
//!
//! Everything else is PDFDocEncoding, decoded with its own table rather than as Latin-1:
//! the two differ in `18`–`1F` and `80`–`A0`, where PDFDocEncoding puts the bullet, the
//! dashes, typographic quotes and the euro sign.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, BIG5, EUC_JP, EUC_KR, GB18030, GBK, SHIFT_JIS};

/// Decode a PDF text string.
///
/// `None` only for a string that announces its encoding with a byte-order mark and then
/// fails to decode in it: a caller reporting metadata should say "no title" rather than
/// show mojibake. Every unmarked string decodes to something — see
/// [`decode_text_string_lossy`] for callers that need a value regardless.
pub(crate) fn decode_text_string(bytes: &[u8]) -> Option<String> {
    if let Some(payload) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16be_payload(payload);
    }
    if let Some(payload) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(payload.to_vec()).ok();
    }
    if looks_like_bomless_utf16be(bytes) {
        // A guess, so a failure falls through to the single-byte readings instead of
        // turning a decodable string into `None`.
        if let Some(s) = decode_utf16be_payload(bytes) {
            return Some(s);
        }
    }
    Some(decode_unmarked_single_or_multibyte(bytes))
}

/// [`decode_text_string`], with a string whose byte-order mark lies decoded as
/// PDFDocEncoding instead of dropped — for values that must not disappear, such as form
/// field names.
pub(crate) fn decode_text_string_lossy(bytes: &[u8]) -> String {
    decode_text_string(bytes).unwrap_or_else(|| decode_pdf_doc_encoding(bytes))
}

fn decode_unmarked_single_or_multibyte(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        // ASCII is the same in every reading below; skip the detector.
        return bytes.iter().map(|&b| char::from(b)).collect();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    decode_legacy_cjk(bytes).unwrap_or_else(|| decode_pdf_doc_encoding(bytes))
}

/// A legacy CJK code page, described by the byte ranges of its *core* character set — the
/// national standard every producer of that locale writes (KS X 1001, GB 2312, Big5,
/// JIS X 0208), as opposed to the vendor extensions layered on top (CP949's UHC area,
/// GBK's extension area).
struct LegacyCodePage {
    encoding: &'static Encoding,
    /// `b` is a character of the core set on its own (ASCII, half-width katakana).
    single: fn(u8) -> bool,
    /// `(lead, trail)` is a two-byte character of the core set.
    pair: fn(u8, u8) -> bool,
}

fn ascii(b: u8) -> bool {
    b < 0x80
}

fn ascii_or_halfwidth_kana(b: u8) -> bool {
    b < 0x80 || (0xA1..=0xDF).contains(&b)
}

/// KS X 1001 in EUC-KR form, and JIS X 0208 in EUC-JP form.
fn euc_pair(lead: u8, trail: u8) -> bool {
    (0xA1..=0xFE).contains(&lead) && (0xA1..=0xFE).contains(&trail)
}

fn gb2312_pair(lead: u8, trail: u8) -> bool {
    (0xA1..=0xF7).contains(&lead) && (0xA1..=0xFE).contains(&trail)
}

fn big5_pair(lead: u8, trail: u8) -> bool {
    (0xA1..=0xF9).contains(&lead)
        && ((0x40..=0x7E).contains(&trail) || (0xA1..=0xFE).contains(&trail))
}

fn shift_jis_pair(lead: u8, trail: u8) -> bool {
    ((0x81..=0x9F).contains(&lead) || (0xE0..=0xEF).contains(&lead))
        && ((0x40..=0x7E).contains(&trail) || (0x80..=0xFC).contains(&trail))
}

/// Legacy multi-byte code pages a producer may have written a text string in. `EUC_KR`
/// is the WHATWG `EUC-KR`, i.e. CP949; `GBK` covers GB 2312.
const LEGACY_CJK: [LegacyCodePage; 5] = [
    LegacyCodePage {
        encoding: EUC_KR,
        single: ascii,
        pair: euc_pair,
    },
    LegacyCodePage {
        encoding: GBK,
        single: ascii,
        pair: gb2312_pair,
    },
    LegacyCodePage {
        encoding: BIG5,
        single: ascii,
        pair: big5_pair,
    },
    LegacyCodePage {
        encoding: SHIFT_JIS,
        single: ascii_or_halfwidth_kana,
        pair: shift_jis_pair,
    },
    LegacyCodePage {
        encoding: EUC_JP,
        single: ascii,
        pair: euc_pair,
    },
];

/// `bytes` read against one code page's core set.
struct CoreReading {
    /// The bytes that form core characters, in order — what the language detector sees.
    core: Vec<u8>,
    /// The decoded text with every sequence outside the core set as one U+FFFD.
    text: String,
    multibyte: usize,
    malformed: usize,
}

fn read_core(bytes: &[u8], page: &LegacyCodePage) -> CoreReading {
    let mut reading = CoreReading {
        core: Vec::new(),
        text: String::new(),
        multibyte: 0,
        malformed: 0,
    };
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let unit = if (page.single)(b) {
            1
        } else if bytes.get(i + 1).is_some_and(|&t| (page.pair)(b, t)) {
            reading.multibyte += 1;
            2
        } else {
            // One damaged character: the byte, and its would-be trail byte unless that is
            // ASCII (which starts a character of its own).
            reading.malformed += 1;
            reading.text.push('\u{FFFD}');
            i += if bytes.get(i + 1).is_some_and(|&t| t >= 0x80) {
                2
            } else {
                1
            };
            continue;
        };
        let sequence = &bytes[i..i + unit];
        reading.core.extend_from_slice(sequence);
        reading
            .text
            .push_str(&page.encoding.decode_without_bom_handling(sequence).0);
        i += unit;
    }
    reading
}

fn detected_encoding(bytes: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    detector.guess(None, Utf8Detection::Deny)
}

/// Whether the detector names `page`'s encoding (GB 18030 is GBK's superset).
fn names(page: &LegacyCodePage, detected: &'static Encoding) -> bool {
    detected == page.encoding || (page.encoding == GBK && detected == GB18030)
}

/// Decode `bytes` in a legacy CJK code page, when that is what they are.
///
/// Every byte sequence is valid PDFDocEncoding, so "it decodes" proves nothing on that
/// side; the question is whether the bytes are *more plausibly* CP949 / GB 2312 / Big5 /
/// Shift_JIS text. That is the question a browser asks of an unlabelled legacy page, and
/// `chardetng` is the detector Firefox answers it with — but it answers by first ruling
/// out every encoding the bytes are malformed in, and a producer that damaged one byte
/// thereby rules out the right answer. Measured on a real report: a Korean title with two
/// damaged trail bytes (`C8 95`, `BC 95` where `C8 AD` 화 and `BC AD` 서 belong; the same
/// two characters are U+FFFD in the producer's own XMP copy) is malformed as CP949 and
/// well-formed as GBK, and the detector returns fluent-looking, wrong Chinese.
///
/// So each code page is asked separately, about its own core characters only:
///
/// 1. Read the bytes against the page's core set; sequences outside it are *damage*.
/// 2. The page is a candidate when the detector, shown only the core bytes, names that
///    very page, and damage is at most a quarter of the multi-byte characters.
/// 3. The least damaged candidate wins. Its text is the full decode when that is clean
///    (vendor-extension characters are real text), and otherwise the core reading with
///    one U+FFFD per damaged character — the loss stays visible instead of becoming a
///    plausible wrong character.
///
/// A string where no page qualifies falls back to the detector's plain verdict, accepted
/// only when it names one of these pages and the bytes decode in it without an error —
/// which covers text that uses vendor-extension characters heavily. Anything else is left
/// to PDFDocEncoding, including accented Latin text (`Café` is guessed windows-1252).
///
/// Very short strings carry little evidence: a four-byte CP949 title (`제목`) is guessed
/// as windows-1253 and stays PDFDocEncoded — the reading it had before, so the rule can
/// leave a case unfixed but does not newly break one.
fn decode_legacy_cjk(bytes: &[u8]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for page in &LEGACY_CJK {
        let reading = read_core(bytes, page);
        if reading.multibyte == 0
            || reading.malformed * 4 > reading.multibyte
            || !names(page, detected_encoding(&reading.core))
            || best
                .as_ref()
                .is_some_and(|(least, _)| *least <= reading.malformed)
        {
            continue;
        }
        let (full, had_errors) = page.encoding.decode_without_bom_handling(bytes);
        let text = if had_errors {
            reading.text
        } else {
            full.into_owned()
        };
        best = Some((reading.malformed, text));
    }
    if let Some((_, text)) = best {
        return Some(text);
    }

    let detected = detected_encoding(bytes);
    if !LEGACY_CJK.iter().any(|page| names(page, detected)) {
        return None;
    }
    let (text, had_errors) = detected.decode_without_bom_handling(bytes);
    (!had_errors).then(|| text.into_owned())
}

/// Decode PDFDocEncoding (PDF 32000-1 Annex D.2).
///
/// Byte values the table leaves undefined (`7F`, `9F`, `AD`) keep their Latin-1 code
/// point, which is what they decoded to before this table existed.
pub(crate) fn decode_pdf_doc_encoding(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| pdf_doc_char(b)).collect()
}

fn pdf_doc_char(b: u8) -> char {
    match b {
        0x18 => '\u{02D8}', // breve
        0x19 => '\u{02C7}', // caron
        0x1A => '\u{02C6}', // circumflex
        0x1B => '\u{02D9}', // dot above
        0x1C => '\u{02DD}', // double acute
        0x1D => '\u{02DB}', // ogonek
        0x1E => '\u{02DA}', // ring above
        0x1F => '\u{02DC}', // small tilde
        0x80 => '\u{2022}', // bullet
        0x81 => '\u{2020}', // dagger
        0x82 => '\u{2021}', // double dagger
        0x83 => '\u{2026}', // ellipsis
        0x84 => '\u{2014}', // em dash
        0x85 => '\u{2013}', // en dash
        0x86 => '\u{0192}', // florin
        0x87 => '\u{2044}', // fraction slash
        0x88 => '\u{2039}', // single left angle quote
        0x89 => '\u{203A}', // single right angle quote
        0x8A => '\u{2212}', // minus
        0x8B => '\u{2030}', // per mille
        0x8C => '\u{201E}', // double low-9 quote
        0x8D => '\u{201C}', // left double quote
        0x8E => '\u{201D}', // right double quote
        0x8F => '\u{2018}', // left single quote
        0x90 => '\u{2019}', // right single quote
        0x91 => '\u{201A}', // single low-9 quote
        0x92 => '\u{2122}', // trademark
        0x93 => '\u{FB01}', // fi ligature
        0x94 => '\u{FB02}', // fl ligature
        0x95 => '\u{0141}', // L with stroke
        0x96 => '\u{0152}', // OE
        0x97 => '\u{0160}', // S with caron
        0x98 => '\u{0178}', // Y with diaeresis
        0x99 => '\u{017D}', // Z with caron
        0x9A => '\u{0131}', // dotless i
        0x9B => '\u{0142}', // l with stroke
        0x9C => '\u{0153}', // oe
        0x9D => '\u{0161}', // s with caron
        0x9E => '\u{017E}', // z with caron
        0xA0 => '\u{20AC}', // euro
        _ => char::from(b),
    }
}

/// Decode UTF-16BE code units, rejecting an odd length or unpaired surrogates.
pub(crate) fn decode_utf16be_payload(payload: &[u8]) -> Option<String> {
    if !payload.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = payload
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(u16::from_be_bytes)
        .collect();
    String::from_utf16(&units).ok()
}

/// Whether `bytes` is UTF-16BE written without the leading byte-order mark, decided by
/// the one test that cannot be wrong: **every even-offset byte is zero.**
///
/// Producers do omit the mark, and then an all-ASCII string arrives as its characters
/// interleaved with zero high bytes. That pattern — NUL at position 0, 2, 4 … without
/// exception — has no single-byte reading that is text, so taking it as UTF-16BE cannot
/// corrupt a legitimate string.
///
/// Anything looser does corrupt one. Requiring merely *some* even-offset NUL, even
/// combined with "the UTF-16BE reading contains no control characters", accepts
/// `CHAP\0TER` (`43 48 41 50 00 54 45 52`) and rewrites it as `䍈䅐T䕒`. A stray NUL in
/// otherwise fine text is exactly what damaged page content looks like, so that
/// mis-reading is not hypothetical: that exact input is an outline title in this
/// repository's own test suite, and the looser rule broke it. Density thresholds fail
/// the same way, only less predictably.
///
/// Consequences of staying narrow, both accepted deliberately:
///
/// - UTF-16BE with no mark that mixes ASCII with anything above U+00FF (`이름(name)`)
///   is not detected — its even bytes are not all zero.
/// - UTF-16BE with no mark and nothing below U+0100 (`성명` is `C1 31 CA 85`) carries no
///   NUL at all and is indistinguishable from single-byte text; `Café` (`43 61 66 E9`)
///   reads as valid UTF-16BE too, so guessing would break real Latin-1.
///
/// Both are resolved by the byte-order mark the specification already requires.
pub(crate) fn looks_like_bomless_utf16be(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes.len().is_multiple_of(2) && bytes.iter().step_by(2).all(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode `s` as UTF-16BE, optionally with the byte-order mark.
    fn utf16be(s: &str, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(&[0xFE, 0xFF]);
        }
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }

    fn encode(text: &str, encoding: &'static Encoding) -> Vec<u8> {
        let (bytes, _, had_errors) = encoding.encode(text);
        assert!(
            !had_errors,
            "{text} is not representable in {}",
            encoding.name()
        );
        bytes.into_owned()
    }

    // --- legacy CJK code pages --------------------------------------------------------

    /// The title from the report: a Korean producer wrote it as raw CP949 bytes. It used to
    /// come back as `2024³â 3¿ù …`.
    #[test]
    fn a_cp949_title_decodes_as_korean() {
        let title =
            "2024년 3월 통화신용정책보고서(안)_최종 게시용 송부_진짜최종_188x257-확인용.PDF";
        assert_eq!(
            decode_text_string(&encode(title, EUC_KR)).as_deref(),
            Some(title)
        );
    }

    #[test]
    fn short_legacy_cjk_titles_decode() {
        for (title, encoding) in [
            ("보고서", EUC_KR),
            ("한국은행", EUC_KR),
            ("令和6年度 事業報告書", SHIFT_JIS),
            ("報告書", SHIFT_JIS),
            ("2024年货币政策执行报告", GBK),
            ("年度報告", BIG5),
        ] {
            assert_eq!(
                decode_text_string(&encode(title, encoding)).as_deref(),
                Some(title),
                "{} title",
                encoding.name()
            );
        }
    }

    /// Bytes measured from a real report's Info `/Title`: CP949, with the trail byte of
    /// 화 and of 서 damaged to `95` by the producer. Read as-is it is well-formed GBK, and a
    /// detector asked about the whole string returns fluent, wrong Chinese.
    const DAMAGED_CP949_TITLE: &str = "32 30 32 34 b3 e2 20 33 bf f9 20 c5 eb c8 95 bd c5 bf eb c1 a4 c3 a5 ba b8 \
        b0 ed bc 95 28 be c8 29 5f c3 d6 c1 be 20 b0 d4 bd c3 bf eb 20 bc db ba ce 5f c1 f8 c2 a5 c3 d6 c1 \
        be 5f 31 38 38 78 32 35 37 2d c8 ae c0 ce bf eb 2e 50 44 46";

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    #[test]
    fn a_damaged_cp949_title_reads_as_korean_with_the_loss_marked() {
        let bytes = hex(DAMAGED_CP949_TITLE);
        // The trap this guards against: the bytes are clean GBK.
        assert!(!GBK.decode_without_bom_handling(&bytes).1);
        assert_eq!(
            decode_text_string(&bytes).as_deref(),
            Some("2024년 3월 통\u{FFFD}신용정책보고\u{FFFD}(안)_최종 게시용 송부_진짜최종_188x257-확인용.PDF")
        );
    }

    /// Vendor-extension characters are real text: CP949's UHC area holds Hangul syllables
    /// KS X 1001 lacks, and a title using one must not lose it to the core reading.
    #[test]
    fn a_title_with_a_cp949_extension_syllable_keeps_it() {
        let title = "똠양꿍 요리 보고서";
        let bytes = encode(title, EUC_KR);
        assert!(
            bytes.iter().any(|&b| (0x81..0xA1).contains(&b)),
            "uses the UHC area"
        );
        assert_eq!(decode_text_string(&bytes).as_deref(), Some(title));
    }

    /// The accepted limit, asserted so it stays a decision: four bytes are too little
    /// evidence, and the string keeps the single-byte reading it always had.
    #[test]
    fn a_two_syllable_cp949_string_is_left_to_pdf_doc_encoding() {
        let bytes = encode("제목", EUC_KR);
        assert_eq!(
            decode_text_string(&bytes).as_deref(),
            Some(decode_pdf_doc_encoding(&bytes).as_str())
        );
    }

    /// The other side of the rule: accented Latin text must never be read as CJK.
    #[test]
    fn latin_text_is_not_mistaken_for_a_cjk_code_page() {
        for text in [
            "Café",
            "Résumé – naïve façade",
            "Größe",
            "São Paulo",
            "é",
            "Ñ",
            "Ångström",
        ] {
            let bytes = encode(text, encoding_rs::WINDOWS_1252);
            let decoded = decode_text_string(&bytes).expect("unmarked strings always decode");
            assert_eq!(decoded, decode_pdf_doc_encoding(&bytes), "{text}");
        }
    }

    // --- PDFDocEncoding ---------------------------------------------------------------

    #[test]
    fn pdf_doc_encoding_is_not_latin_1() {
        // Bullet, en dash, typographic quotes, trademark, euro: the code points where the
        // two tables differ, and where reading the bytes as Latin-1 gave C1 controls.
        let bytes = [
            0x80, b' ', 0x85, b' ', 0x8D, b'q', 0x8E, b' ', 0x92, b' ', 0xA0,
        ];
        assert_eq!(decode_text_string(&bytes).as_deref(), Some("• – “q” ™ €"));
    }

    #[test]
    fn pdf_doc_encoding_matches_latin_1_where_the_tables_agree() {
        assert_eq!(decode_pdf_doc_encoding(&[0xE9, 0xFC, 0xC5]), "éüÅ");
    }

    // --- byte-order marks -------------------------------------------------------------

    #[test]
    fn a_utf8_bom_is_honoured() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Título 제목".as_bytes());
        assert_eq!(decode_text_string(&bytes).as_deref(), Some("Título 제목"));
    }

    #[test]
    fn a_string_that_lies_about_its_bom_is_none_but_the_lossy_reading_keeps_it() {
        let bytes = [0xFE, 0xFF, 0x00]; // odd payload
        assert_eq!(decode_text_string(&bytes), None);
        assert_eq!(decode_text_string_lossy(&bytes), "þÿ\u{0}");
    }

    #[test]
    fn unmarked_utf8_is_taken_as_utf8() {
        assert_eq!(
            decode_text_string("Title 제목".as_bytes()).as_deref(),
            Some("Title 제목")
        );
    }

    // --- UTF-16BE ---------------------------------------------------------------------

    #[test]
    fn bomless_utf16be_ascii_decodes_instead_of_leaving_nuls() {
        let bytes = utf16be("topmostSubform", false);
        assert!(looks_like_bomless_utf16be(&bytes));
        assert_eq!(decode_text_string_lossy(&bytes), "topmostSubform");
    }

    /// The counterexample that decided the rule. A looser test — "some even-offset NUL
    /// and the UTF-16BE reading has no control characters" — accepts this and rewrites a
    /// perfectly recoverable outline title as CJK. A stray NUL in otherwise fine text is
    /// what damaged page content looks like, so this is not a hypothetical input.
    #[test]
    fn ascii_with_one_interior_nul_is_not_read_as_utf16() {
        let bytes = b"CHAP\0TER".to_vec();
        // A NUL does sit at an even offset (4), and the UTF-16BE reading does decode
        // without control characters — it decodes to U+4348 U+4150 U+0054 U+4552.
        assert!(bytes.iter().step_by(2).any(|&b| b == 0));
        assert_eq!(decode_utf16be_payload(&bytes).as_deref(), Some("䍈䅐T䕒"));

        assert!(!looks_like_bomless_utf16be(&bytes));
        assert_eq!(decode_text_string_lossy(&bytes), "CHAP\0TER");
    }

    /// Limits of the narrow rule, asserted so they are decisions rather than surprises.
    #[test]
    fn bomless_utf16be_is_only_detected_when_every_even_byte_is_zero() {
        // Mixed ASCII and non-ASCII: `이름(name)` has non-zero high bytes for the Korean.
        let mixed = utf16be("이름(name)", false);
        assert!(!looks_like_bomless_utf16be(&mixed));

        // Nothing below U+0100 at all: `성명` is `C1 31 CA 85`, no NUL anywhere.
        let no_nul = utf16be("성명", false);
        assert!(!no_nul.contains(&0));
        assert!(!looks_like_bomless_utf16be(&no_nul));

        // The mark the spec requires resolves both.
        assert_eq!(
            decode_text_string_lossy(&utf16be("이름(name)", true)),
            "이름(name)"
        );
        assert_eq!(decode_text_string_lossy(&utf16be("성명", true)), "성명");
    }

    #[test]
    fn bom_carrying_utf16be_still_decodes() {
        let bytes = utf16be("Title 제목", true);
        assert_eq!(decode_text_string(&bytes).as_deref(), Some("Title 제목"));
    }

    /// Damaged page text looks like a single-byte string with a stray NUL. Treating it
    /// as UTF-16BE would replace a recoverable string with garbage.
    #[test]
    fn single_byte_string_with_a_stray_nul_is_not_read_as_utf16() {
        let bytes = b"HELLO\0WORLD\0".to_vec();
        assert!(!looks_like_bomless_utf16be(&bytes));
        // The NULs remain for `sanitize_extracted_text` to remove — the text is intact.
        assert_eq!(decode_text_string_lossy(&bytes), "HELLO\0WORLD\0");
    }

    #[test]
    fn ordinary_single_byte_strings_are_untouched() {
        for s in ["FirstName", "", "Agree", "a"] {
            assert_eq!(decode_text_string_lossy(s.as_bytes()), s);
        }
    }

    #[test]
    fn utf16le_bom_is_left_to_the_single_byte_path() {
        // Not a PDF text string. Reading it as UTF-16BE would yield a byte-swapped
        // result, which is worse than the honest single-byte reading.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&[0x41, 0x00, 0x42, 0x00]);
        assert!(!looks_like_bomless_utf16be(&bytes));
    }

    #[test]
    fn unpaired_surrogates_are_rejected_rather_than_replaced() {
        // A lone high surrogate: `String::from_utf16` refuses it, and the caller falls
        // back rather than emitting U+FFFD, which would pollute the font-decode metric.
        let bytes = vec![0xD8, 0x00, 0x00, 0x41];
        assert!(decode_utf16be_payload(&bytes).is_none());
    }

    #[test]
    fn odd_length_is_never_utf16() {
        let bytes = b"\0a\0".to_vec();
        assert!(!looks_like_bomless_utf16be(&bytes));
        assert!(decode_utf16be_payload(&bytes).is_none());
    }
}
