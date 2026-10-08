//! Glyph advance widths, as a font dictionary declares them.
//!
//! A text-showing operator moves the text position by each glyph's advance
//! (ISO 32000-1 §9.4.4). Without those advances a run has no extent: the next
//! run's position can only be compared against a guess, and a guess that is off
//! by a fraction of an em is enough to split a word (`m echanism s`) when a
//! producer draws one glyph per operator, or to swallow a column gap when it
//! draws whole lines. The widths are in the font dictionary — this module turns
//! them into per-code advances.
//!
//! Backend-agnostic: the backend reads the dictionary and builds a
//! [`FontMetrics`]; nothing here knows about PDF objects.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::backend::GlyphAdvance;
use super::cmap::CMap;

/// The advance widths a font dictionary declares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FontMetrics {
    /// A simple font: one byte per code, `/Widths` indexed from `/FirstChar`.
    Simple {
        first_char: u32,
        widths: Vec<f32>,
        missing_width: f32,
    },
    /// A composite font in horizontal writing mode, widths from the CIDFont's `/W`
    /// with `/DW` as the default, keyed by the CIDs `coding` resolves the codes to.
    Cid {
        widths: BTreeMap<u32, f32>,
        default_width: f32,
        coding: CidCoding,
    },
}

/// How a composite font's character codes become CIDs (ISO 32000-1 §9.7.5).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CidCoding {
    /// `Identity-H`: every code is two bytes, and the code is the CID.
    Identity,
    /// A CMap — predefined (`UniKS-UCS2-H`, `KSC-EUC-H`) or embedded in the file — for a
    /// CIDFont of the given `/CIDSystemInfo` ordering.
    Map { cmap: Arc<CMap>, ordering: String },
}

impl FontMetrics {
    /// Advances for every code in `bytes`, in order.
    pub(crate) fn advances(&self, bytes: &[u8]) -> Vec<GlyphAdvance> {
        match self {
            FontMetrics::Simple {
                first_char,
                widths,
                missing_width,
            } => bytes
                .iter()
                .map(|&b| {
                    let code = u32::from(b);
                    let width = code
                        .checked_sub(*first_char)
                        .and_then(|i| widths.get(i as usize))
                        .copied()
                        .unwrap_or(*missing_width);
                    GlyphAdvance {
                        width,
                        is_word_space: b == b' ',
                    }
                })
                .collect(),
            FontMetrics::Cid {
                widths,
                default_width,
                coding,
            } => {
                let advance = |cid: u32, is_word_space: bool| GlyphAdvance {
                    width: widths.get(&cid).copied().unwrap_or(*default_width),
                    is_word_space,
                };
                match coding {
                    CidCoding::Identity => bytes
                        .chunks(2)
                        .map(|pair| {
                            let cid = match pair {
                                [hi, lo] => u32::from(*hi) << 8 | u32::from(*lo),
                                [lone] => u32::from(*lone),
                                _ => 0,
                            };
                            advance(cid, false)
                        })
                        .collect(),
                    // The backend builds this variant only for a CMap that resolves.
                    CidCoding::Map { cmap, ordering } => cmap
                        .codes(ordering, bytes)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|code| advance(code.cid, code.is_word_space))
                        .collect(),
                }
            }
        }
    }
}

/// One element of a CIDFont `/W` array, already resolved to plain values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WEntry {
    Number(f32),
    Array(Vec<f32>),
}

/// Expand a `/W` array into a CID → width map.
///
/// The array mixes two forms (§9.7.4.3): `c [w1 w2 …]` gives consecutive CIDs
/// from `c`, and `c_first c_last w` gives one width to a range. Anything that
/// fits neither form ends the scan — what was read before it is kept.
pub(crate) fn expand_w_array(entries: &[WEntry]) -> BTreeMap<u32, f32> {
    /// A range wider than any real font's CID space is malformed, not a request
    /// to allocate millions of entries.
    const MAX_RANGE: u32 = 0x1_0000;

    let mut widths = BTreeMap::new();
    let mut i = 0;
    while i < entries.len() {
        let WEntry::Number(first) = entries[i] else {
            break;
        };
        let first = first as u32;
        match entries.get(i + 1) {
            Some(WEntry::Array(list)) => {
                for (offset, &w) in list.iter().enumerate() {
                    widths.insert(first + offset as u32, w);
                }
                i += 2;
            }
            Some(WEntry::Number(last)) => {
                let Some(WEntry::Number(w)) = entries.get(i + 2) else {
                    break;
                };
                let last = *last as u32;
                if last < first || last - first > MAX_RANGE {
                    break;
                }
                for cid in first..=last {
                    widths.insert(cid, *w);
                }
                i += 3;
            }
            None => break,
        }
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_font_indexes_widths_from_first_char() {
        let m = FontMetrics::Simple {
            first_char: 32,
            widths: vec![278.0, 333.0, 474.0],
            missing_width: 500.0,
        };
        let adv = m.advances(b" !\"#");
        let widths: Vec<f32> = adv.iter().map(|a| a.width).collect();
        assert_eq!(widths, vec![278.0, 333.0, 474.0, 500.0]);
        assert!(adv[0].is_word_space);
        assert!(!adv[1].is_word_space);
    }

    #[test]
    fn a_code_below_first_char_takes_the_missing_width() {
        let m = FontMetrics::Simple {
            first_char: 65,
            widths: vec![722.0],
            missing_width: 0.0,
        };
        assert_eq!(m.advances(b"@A")[0].width, 0.0);
        assert_eq!(m.advances(b"@A")[1].width, 722.0);
    }

    #[test]
    fn cid_font_reads_two_byte_codes_and_falls_back_to_dw() {
        let m = FontMetrics::Cid {
            widths: BTreeMap::from([(0x50, 889.0)]),
            default_width: 1000.0,
            coding: CidCoding::Identity,
        };
        let adv = m.advances(&[0x00, 0x50, 0x00, 0x03]);
        assert_eq!(adv.len(), 2);
        assert_eq!(adv[0].width, 889.0);
        assert_eq!(adv[1].width, 1000.0);
        // Tw never applies to a multi-byte code, even one that maps to a space.
        assert!(!adv[1].is_word_space);
    }

    #[test]
    fn a_predefined_cmap_resolves_codes_to_cids_before_the_widths() {
        // Adobe-Korea1: `A` is CID 34 under UniKS-UCS2, CID 8127 under KSC-EUC.
        let widths = BTreeMap::from([(34, 700.0), (8127, 500.0)]);
        let unicode = FontMetrics::Cid {
            widths: widths.clone(),
            default_width: 1000.0,
            coding: CidCoding::Map {
                cmap: Arc::new(CMap::predefined("UniKS-UCS2-H")),
                ordering: "Korea1".into(),
            },
        };
        let adv = unicode.advances(&[0x00, 0x41, 0xD5, 0x5C]);
        let got: Vec<f32> = adv.iter().map(|a| a.width).collect();
        assert_eq!(got, vec![700.0, 1000.0]);

        let legacy = FontMetrics::Cid {
            widths,
            default_width: 1000.0,
            coding: CidCoding::Map {
                cmap: Arc::new(CMap::predefined("KSC-EUC-H")),
                ordering: "Korea1".into(),
            },
        };
        // `A`, a one-byte space, and 한 (C7D1).
        let adv = legacy.advances(&[0x41, 0x20, 0xC7, 0xD1]);
        assert_eq!(adv.len(), 3);
        assert_eq!(adv[0].width, 500.0);
        // Tw applies to the single-byte code 32 of a composite font (§9.3.3).
        assert!(adv[1].is_word_space);
        assert!(!adv[2].is_word_space);
    }

    #[test]
    fn w_array_reads_both_forms() {
        let w = expand_w_array(&[
            WEntry::Number(3.0),
            WEntry::Array(vec![278.0, 333.0]),
            WEntry::Number(19.0),
            WEntry::Number(21.0),
            WEntry::Number(556.0),
        ]);
        assert_eq!(w.get(&3), Some(&278.0));
        assert_eq!(w.get(&4), Some(&333.0));
        assert_eq!(w.get(&5), None);
        assert_eq!(w.get(&19), Some(&556.0));
        assert_eq!(w.get(&21), Some(&556.0));
        assert_eq!(w.get(&22), None);
    }

    #[test]
    fn a_malformed_w_array_keeps_what_was_read() {
        let w = expand_w_array(&[
            WEntry::Number(1.0),
            WEntry::Array(vec![500.0]),
            WEntry::Array(vec![600.0]),
        ]);
        assert_eq!(w, BTreeMap::from([(1, 500.0)]));
    }

    #[test]
    fn an_absurd_w_range_is_rejected_not_allocated() {
        let w = expand_w_array(&[
            WEntry::Number(0.0),
            WEntry::Number(4_000_000_000.0),
            WEntry::Number(500.0),
        ]);
        assert!(w.is_empty());
    }
}
