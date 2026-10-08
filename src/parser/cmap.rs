//! Code → CID CMaps of composite fonts (ISO 32000-1 §9.7.5).
//!
//! A Type 0 font's `/Encoding` says how the bytes of a shown string split into character
//! codes and which CID each code selects. It is a predefined CMap named by the font
//! (`UniKS-UCS2-H`, `KSC-EUC-H`, `Identity-H`) or a CMap stream embedded in the file —
//! which may itself build on another CMap through `/UseCMap`. [`CMap`] is the one
//! resolved form of all of these: text decoding and glyph widths both read codes
//! through it, so they cannot disagree about where a code ends or which CID it is.
//!
//! The stream parser here reads the `codespacerange`, `cidrange`, `cidchar`, `usecmap`
//! and `WMode` constructs of a CMap program. It scans text, never executes it, and a
//! stream it cannot make sense of yields `None` — the caller reports the font as
//! unreadable rather than guessing.

use super::font::{next_angle_token, parse_hex};
use super::predefined_cmap::{self, PredefinedCode};

/// How many `/UseCMap` links are followed before the chain is taken for a loop.
pub(crate) const MAX_USE_CMAP_DEPTH: usize = 8;
/// Entries one CMap stream may declare; anything beyond is not read.
const MAX_ENTRIES: usize = 200_000;
/// Code space ranges one CMap stream may declare.
const MAX_CODESPACES: usize = 64;
/// Longest character code in bytes (§9.7.6.2).
const MAX_CODE_LEN: usize = 4;

/// One `codespacerange` entry: each byte of a code lies within the matching byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Codespace {
    len: usize,
    lo: [u8; MAX_CODE_LEN],
    hi: [u8; MAX_CODE_LEN],
}

impl Codespace {
    fn matches(&self, code: &[u8]) -> bool {
        code.len() >= self.len
            && (0..self.len).all(|i| (self.lo[i]..=self.hi[i]).contains(&code[i]))
    }
}

/// A run of consecutive codes of one length that select consecutive CIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CidRange {
    len: usize,
    lo: u32,
    hi: u32,
    cid: u32,
}

/// What a single CMap stream declares on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CMapDef {
    /// `/WMode`: 1 for vertical writing.
    pub(crate) wmode: Option<u8>,
    /// The CMap it builds on, by name (`/Name usecmap`).
    pub(crate) use_cmap: Option<String>,
    codespaces: Vec<Codespace>,
    /// Sorted by `(len, lo)`.
    ranges: Vec<CidRange>,
}

/// The sections between `begin` and `end` markers, in order.
fn blocks<'a>(text: &'a str, begin: &str, end: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = text[at..].find(begin) {
        let from = at + start + begin.len();
        let Some(len) = text[from..].find(end) else {
            break;
        };
        found.push(&text[from..from + len]);
        at = from + len + end.len();
    }
    found
}

/// The unsigned decimal at the start of `s` and what follows it.
fn next_number(s: &str) -> Option<(u32, &str)> {
    let s = s.trim_start();
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let n = s[..digits].parse().ok()?;
    Some((n, &s[digits..]))
}

/// A code in a hex token: its byte count and big-endian value.
fn hex_code(hex: &str) -> Option<(usize, u32)> {
    let len = hex.len() / 2;
    if !hex.len().is_multiple_of(2) || !(1..=MAX_CODE_LEN).contains(&len) {
        return None;
    }
    Some((len, parse_hex(hex)?))
}

fn code_bytes(len: usize, value: u32) -> [u8; MAX_CODE_LEN] {
    let mut bytes = [0; MAX_CODE_LEN];
    for (i, b) in bytes.iter_mut().take(len).enumerate() {
        *b = (value >> (8 * (len - 1 - i))) as u8;
    }
    bytes
}

/// A code's bytes as a big-endian number.
fn code_value(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |acc, &b| acc << 8 | u32::from(b))
}

/// Read a CMap stream's own declarations. `None` when it declares nothing a CMap would.
pub(crate) fn parse(data: &[u8]) -> Option<CMapDef> {
    let text = String::from_utf8_lossy(data);
    let mut def = CMapDef::default();

    for block in blocks(&text, "begincodespacerange", "endcodespacerange") {
        let mut rest = block;
        while let Some((lo, r)) = next_angle_token(rest) {
            let Some((hi, r)) = next_angle_token(r) else {
                break;
            };
            rest = r;
            if let (Some((len, lo)), Some((hi_len, hi))) = (hex_code(lo), hex_code(hi)) {
                if len == hi_len && def.codespaces.len() < MAX_CODESPACES {
                    def.codespaces.push(Codespace {
                        len,
                        lo: code_bytes(len, lo),
                        hi: code_bytes(len, hi),
                    });
                }
            }
        }
    }

    for block in blocks(&text, "begincidrange", "endcidrange") {
        let mut rest = block;
        while let Some((lo, r)) = next_angle_token(rest) {
            let Some((hi, r)) = next_angle_token(r) else {
                break;
            };
            let Some((cid, r)) = next_number(r) else {
                break;
            };
            rest = r;
            if let (Some((len, lo)), Some((hi_len, hi))) = (hex_code(lo), hex_code(hi)) {
                if len == hi_len && lo <= hi && def.ranges.len() < MAX_ENTRIES {
                    def.ranges.push(CidRange { len, lo, hi, cid });
                }
            }
        }
    }
    for block in blocks(&text, "begincidchar", "endcidchar") {
        let mut rest = block;
        while let Some((code, r)) = next_angle_token(rest) {
            let Some((cid, r)) = next_number(r) else {
                break;
            };
            rest = r;
            if let Some((len, code)) = hex_code(code) {
                if def.ranges.len() < MAX_ENTRIES {
                    def.ranges.push(CidRange {
                        len,
                        lo: code,
                        hi: code,
                        cid,
                    });
                }
            }
        }
    }
    def.ranges.sort_by_key(|r| (r.len, r.lo));

    if let Some(at) = text.find("usecmap") {
        def.use_cmap = text[..at]
            .split_whitespace()
            .next_back()
            .and_then(|token| token.strip_prefix('/'))
            .filter(|name| !name.is_empty() && name.len() <= 64)
            .map(str::to_owned);
    }
    if let Some(at) = text.find("/WMode") {
        def.wmode = next_number(&text[at + "/WMode".len()..]).map(|(n, _)| u8::from(n == 1));
    }

    let declares_something =
        !def.codespaces.is_empty() || !def.ranges.is_empty() || def.use_cmap.is_some();
    declares_something.then_some(def)
}

impl CMapDef {
    fn cid_of(&self, len: usize, code: u32) -> Option<u32> {
        let below = self
            .ranges
            .partition_point(|r| (r.len, r.lo) <= (len, code));
        let range = self.ranges.get(below.checked_sub(1)?)?;
        (range.len == len && code <= range.hi).then(|| range.cid + (code - range.lo))
    }
}

/// What a CMap builds on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Parent {
    /// Nothing: codes the CMap does not map are CID 0.
    None,
    /// `Identity-H` / `Identity-V`: two-byte codes that are their own CIDs.
    Identity,
    /// A predefined CMap that ships with the crate, by name.
    Predefined(String),
    /// Another CMap stream, resolved in turn.
    Stream(Box<CMap>),
}

impl Parent {
    /// The parent a CMap names (`/UseCMap /Name`, `/Name usecmap`).
    pub(crate) fn named(name: &str) -> Self {
        if matches!(name, "Identity-H" | "Identity-V") {
            Parent::Identity
        } else {
            Parent::Predefined(name.to_owned())
        }
    }
}

/// One character code of a shown string, resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Code {
    /// The CID the code selects; 0 (`.notdef`) when the CMap does not map it.
    pub cid: u32,
    /// The character, when a Unicode CMap carries it in the code itself.
    pub text: Option<char>,
    /// The single-byte code 32, the only code word spacing applies to (§9.3.3).
    pub is_word_space: bool,
    /// A one-byte code's value, for the ASCII a collection leaves unmapped.
    single_byte: Option<u8>,
}

/// A composite font's resolved `/Encoding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CMap {
    own: CMapDef,
    parent: Parent,
}

impl CMap {
    /// A predefined CMap, by name.
    pub(crate) fn predefined(name: &str) -> Self {
        Self::new(CMapDef::default(), Parent::named(name))
    }

    pub(crate) fn new(own: CMapDef, parent: Parent) -> Self {
        Self { own, parent }
    }

    /// Whether the CMap sets vertical writing (`/WMode 1`, or a `-V` name); the nearest
    /// declaration in the chain decides.
    pub(crate) fn is_vertical(&self) -> bool {
        match (&self.own.wmode, &self.parent) {
            (Some(mode), _) => *mode == 1,
            (None, Parent::Stream(parent)) => parent.is_vertical(),
            (None, Parent::Predefined(name)) => name.ends_with("-V") || name == "V",
            (None, Parent::Identity | Parent::None) => false,
        }
    }

    /// The code space of the whole chain: this CMap's ranges, then its parents'.
    fn codespaces(&self) -> Vec<&Codespace> {
        let mut spaces: Vec<&Codespace> = self.own.codespaces.iter().collect();
        if let Parent::Stream(parent) = &self.parent {
            spaces.extend(parent.codespaces());
        }
        spaces
    }

    /// The identity, predefined CMap or nothing at the end of the chain.
    fn root(&self) -> &Parent {
        match &self.parent {
            Parent::Stream(parent) => parent.root(),
            other => other,
        }
    }

    fn own_cid(&self, len: usize, code: u32) -> Option<u32> {
        self.own.cid_of(len, code).or_else(|| match &self.parent {
            Parent::Stream(parent) => parent.own_cid(len, code),
            _ => None,
        })
    }

    /// Whether [`Self::codes`] can resolve codes for a CIDFont of `ordering`.
    pub(crate) fn resolves(&self, ordering: &str) -> bool {
        match self.root() {
            Parent::Predefined(name) => predefined_cmap::resolves_cids(name, ordering),
            Parent::Identity => true,
            Parent::None | Parent::Stream(_) => !self.codespaces().is_empty(),
        }
    }

    /// One code, resolved: this chain's own ranges first, then what the root gives it.
    fn code(&self, len: usize, value: u32, root: Option<PredefinedCode>) -> Code {
        let cid = self
            .own_cid(len, value)
            .unwrap_or_else(|| match self.root() {
                Parent::Identity if len == 2 => value,
                Parent::Predefined(_) => root.map_or(0, |p| p.cid),
                _ => 0,
            });
        Code {
            cid,
            text: root.and_then(|p| p.scalar),
            is_word_space: len == 1 && value == 0x20,
            single_byte: (len == 1).then_some(value as u8),
        }
    }

    /// The codes of `bytes`, each resolved. Every code shown advances the text position,
    /// so an unmapped one is still an entry, as CID 0. `None` when the chain ends in a
    /// predefined CMap the crate does not carry, or declares no code space to split by.
    pub(crate) fn codes(&self, ordering: &str, bytes: &[u8]) -> Option<Vec<Code>> {
        let spaces = self.codespaces();

        // No code space of its own: the CMap underneath decides where codes end.
        if spaces.is_empty() {
            return match self.root() {
                Parent::Predefined(name) => Some(
                    predefined_cmap::split(name, ordering, bytes)?
                        .into_iter()
                        .map(|p| self.code(p.len, p.value, Some(p)))
                        .collect(),
                ),
                Parent::Identity => Some(
                    bytes
                        .chunks(2)
                        .map(|pair| self.code(pair.len(), code_value(pair), None))
                        .collect(),
                ),
                Parent::None | Parent::Stream(_) => None,
            };
        }

        let shortest = spaces.iter().map(|s| s.len).min()?;
        let mut codes = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let rest = &bytes[i..];
            // A code outside every range costs the shortest code length (§9.7.6.3).
            let len = (1..=MAX_CODE_LEN.min(rest.len()))
                .find(|&len| {
                    spaces
                        .iter()
                        .any(|s| s.len == len && s.matches(&rest[..len]))
                })
                .unwrap_or(shortest.min(rest.len()));
            let part = match self.root() {
                Parent::Predefined(name) => predefined_cmap::split(name, ordering, &rest[..len])
                    .and_then(|parts| parts.into_iter().next())
                    .filter(|p| p.len == len),
                _ => None,
            };
            codes.push(self.code(len, code_value(&rest[..len]), part));
            i += len;
        }
        Some(codes)
    }

    /// The text of `bytes`: a Unicode CMap's own characters, otherwise the CIDFont's
    /// collection (`registry`, `ordering`) read at each code's CID. A code that resolves
    /// to nothing is skipped; `None` means no code resolved at all.
    pub(crate) fn decode(&self, registry: &str, ordering: &str, bytes: &[u8]) -> Option<String> {
        let text: String = self
            .codes(ordering, bytes)?
            .into_iter()
            .filter_map(|code| {
                code.text
                    .or_else(|| super::cmap_table::lookup_cid(registry, ordering, code.cid))
                    .or_else(|| predefined_cmap::ascii_fallback(u16::from(code.single_byte?), 1))
            })
            .collect();
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap
/CMapName /Test-H def /WMode 0 def
2 begincodespacerange <00> <7F> <8000> <FFFF> endcodespacerange
1 begincidrange <20> <7E> 1 endcidrange
2 begincidchar <8001> 3296 <8002> 1238 endcidchar
endcmap end end";

    fn parsed(data: &[u8]) -> CMap {
        CMap::new(parse(data).expect("a CMap"), Parent::None)
    }

    #[test]
    fn a_stream_cmap_splits_by_its_code_space_and_maps_by_range() {
        let map = parsed(FULL);
        let bytes = [0x41, 0x80, 0x01, 0x20, 0x80, 0x02];
        let codes = map.codes("Korea1", &bytes).unwrap();
        let cids: Vec<u32> = codes.iter().map(|c| c.cid).collect();
        // `A` is code 0x41, 0x21 past the range start CID 1; the two-byte codes follow.
        assert_eq!(cids, [34, 3296, 1, 1238]);
        assert!(!codes[0].is_word_space);
        assert!(codes[2].is_word_space);
        assert_eq!(
            map.decode("Adobe", "Korea1", &bytes).as_deref(),
            Some("A한 글")
        );
    }

    #[test]
    fn a_code_outside_the_code_space_costs_the_shortest_code_and_is_cid_zero() {
        let map = parsed(FULL);
        let codes = map.codes("Korea1", &[0xFF]).unwrap();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].cid, 0);
    }

    #[test]
    fn use_cmap_and_wmode_are_read_from_the_stream() {
        let def = parse(b"/UniKS-UCS2-V usecmap /WMode 1 def").unwrap();
        assert_eq!(def.use_cmap.as_deref(), Some("UniKS-UCS2-V"));
        let map = CMap::new(def, Parent::named("UniKS-UCS2-V"));
        assert!(map.is_vertical());
        assert!(!CMap::predefined("UniKS-UCS2-H").is_vertical());
    }

    #[test]
    fn a_cmap_over_a_unicode_cmap_without_a_code_space_reads_the_unicode() {
        let def = parse(b"/UniKS-UCS2-H usecmap").unwrap();
        let map = CMap::new(def, Parent::named("UniKS-UCS2-H"));
        assert_eq!(
            map.decode("Adobe", "Korea1", &[0xD5, 0x5C, 0x00, 0x20])
                .as_deref(),
            Some("한 ")
        );
    }

    #[test]
    fn own_ranges_override_the_cmap_underneath() {
        let def = parse(b"/KSC-EUC-H usecmap 1 begincidchar <B1DB> 3296 endcidchar").unwrap();
        let map = CMap::new(def, Parent::named("KSC-EUC-H"));
        // 글 (B1DB) is redirected to the CID of 한; 한 (C7D1) keeps the base mapping.
        assert_eq!(
            map.decode("Adobe", "Korea1", &[0xB1, 0xDB, 0xC7, 0xD1])
                .as_deref(),
            Some("한한")
        );
    }

    #[test]
    fn identity_underneath_makes_two_byte_codes_their_own_cids() {
        let map = CMap::new(parse(b"/Identity-H usecmap").unwrap(), Parent::Identity);
        let codes = map.codes("Korea1", &[0x0C, 0xE0, 0x00, 0x01]).unwrap();
        let cids: Vec<u32> = codes.iter().map(|c| c.cid).collect();
        assert_eq!(cids, [0x0CE0, 1]);
    }

    #[test]
    fn garbage_and_empty_streams_are_not_cmaps() {
        assert!(parse(&[0xFF, 0x00, 0x13, 0x80, 0x99]).is_none());
        assert!(parse(b"").is_none());
        assert!(parse(b"begincmap endcmap").is_none());
    }

    #[test]
    fn malformed_entries_are_dropped_without_panicking() {
        let def = parse(
            b"begincodespacerange <00 <FF endcodespacerange
              begincidrange <0000> <FFFFFFFFFF> 1 <10> <05> 2 <1> <2> 3 <20> endcidrange
              begincidchar <41> endcidchar /WMode",
        );
        // Whatever was recoverable is kept; nothing here is a usable range.
        if let Some(def) = def {
            assert!(def.ranges.is_empty());
        }
        let map = parsed(b"begincidchar <41> 1 endcidchar");
        // Ranges without a code space cannot split a string: unresolved, not guessed.
        assert!(map.codes("Korea1", b"A").is_none());
        assert!(!map.resolves("Korea1"));
    }

    #[test]
    fn a_range_lookup_stays_within_its_length_and_bounds() {
        let def =
            parse(b"begincidrange <0041> <0043> 10 endcidrange begincidchar <41> 99 endcidchar")
                .unwrap();
        assert_eq!(def.cid_of(2, 0x42), Some(11));
        assert_eq!(def.cid_of(2, 0x44), None);
        assert_eq!(def.cid_of(1, 0x41), Some(99));
        assert_eq!(def.cid_of(1, 0x42), None);
    }
}
