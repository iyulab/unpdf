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

use super::backend::{GlyphAdvance, VerticalAdvance};
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
    /// A composite font: widths from the CIDFont's `/W` with `/DW` as the default, keyed by
    /// the CIDs `coding` resolves the codes to. In vertical writing mode `vertical` adds
    /// the displacements of `/W2` and `/DW2`.
    Cid {
        widths: BTreeMap<u32, f32>,
        default_width: f32,
        coding: CidCoding,
        vertical: Option<VerticalFont>,
    },
}

/// A CIDFont's vertical metrics (ISO 32000-1 §9.7.4.3): `/DW2` and `/W2`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VerticalFont {
    /// `/DW2`: the position vector's `vy` and the displacement `w1y` of a CID `/W2` omits.
    pub default: (f32, f32),
    /// `/W2`, by CID: `[w1y, vx, vy]`.
    pub widths: BTreeMap<u32, [f32; 3]>,
}

impl VerticalFont {
    /// `/DW2` when the font declares none.
    pub(crate) const DEFAULT_DW2: (f32, f32) = (880.0, -1000.0);

    /// The vertical displacement of `cid`, whose horizontal width is `w0`. A CID `/W2`
    /// omits moves by `/DW2`'s `w1y` from a position vector of `(w0 / 2, vy)`.
    fn of(&self, cid: u32, w0: f32) -> VerticalAdvance {
        match self.widths.get(&cid) {
            Some(&[advance, vx, vy]) => VerticalAdvance {
                advance,
                origin: (vx, vy),
            },
            None => VerticalAdvance {
                advance: self.default.1,
                origin: (w0 / 2.0, self.default.0),
            },
        }
    }
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
    /// Whether the font writes vertically: its glyphs advance down the page.
    pub(crate) fn is_vertical(&self) -> bool {
        matches!(
            self,
            FontMetrics::Cid {
                vertical: Some(_),
                ..
            }
        )
    }

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
                        vertical: None,
                        is_word_space: b == b' ',
                    }
                })
                .collect(),
            FontMetrics::Cid {
                widths,
                default_width,
                coding,
                vertical,
            } => {
                let advance = |cid: u32, is_word_space: bool| {
                    let width = widths.get(&cid).copied().unwrap_or(*default_width);
                    GlyphAdvance {
                        width,
                        vertical: vertical.as_ref().map(|v| v.of(cid, width)),
                        is_word_space,
                    }
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
    expand_cid_array::<1>(entries)
        .into_iter()
        .map(|(cid, [w])| (cid, w))
        .collect()
}

/// Expand a `/W2` array into a CID → `[w1y, vx, vy]` map: the same two forms as `/W`, with
/// three numbers per CID (`c [w1y vx vy …]`, `c_first c_last w1y vx vy`).
pub(crate) fn expand_w2_array(entries: &[WEntry]) -> BTreeMap<u32, [f32; 3]> {
    expand_cid_array::<3>(entries)
}

/// The shared scan of `/W` and `/W2`, whose entries carry `N` numbers per CID.
fn expand_cid_array<const N: usize>(entries: &[WEntry]) -> BTreeMap<u32, [f32; N]> {
    /// A range wider than any real font's CID space is malformed, not a request
    /// to allocate millions of entries.
    const MAX_RANGE: u32 = 0x1_0000;

    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < entries.len() {
        let WEntry::Number(first) = entries[i] else {
            break;
        };
        let first = first as u32;
        match entries.get(i + 1) {
            Some(WEntry::Array(list)) => {
                // A trailing partial group is not an entry.
                for (offset, group) in list.as_chunks::<N>().0.iter().enumerate() {
                    let Some(cid) = u32::try_from(offset)
                        .ok()
                        .and_then(|offset| first.checked_add(offset))
                    else {
                        break;
                    };
                    map.insert(cid, *group);
                }
                i += 2;
            }
            Some(WEntry::Number(last)) => {
                let mut values = [0.0; N];
                for (k, slot) in values.iter_mut().enumerate() {
                    let Some(WEntry::Number(v)) = entries.get(i + 2 + k) else {
                        return map;
                    };
                    *slot = *v;
                }
                let last = *last as u32;
                if last < first || last - first > MAX_RANGE {
                    break;
                }
                for cid in first..=last {
                    map.insert(cid, values);
                }
                i += 2 + N;
            }
            None => break,
        }
    }
    map
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
            vertical: None,
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
            vertical: None,
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
            vertical: None,
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

    #[test]
    fn w2_array_reads_both_forms() {
        let w = expand_w2_array(&[
            WEntry::Number(3.0),
            WEntry::Array(vec![-500.0, 250.0, 700.0, -600.0, 300.0, 800.0]),
            WEntry::Number(19.0),
            WEntry::Number(21.0),
            WEntry::Number(-400.0),
            WEntry::Number(260.0),
            WEntry::Number(710.0),
        ]);
        assert_eq!(w.get(&3), Some(&[-500.0, 250.0, 700.0]));
        assert_eq!(w.get(&4), Some(&[-600.0, 300.0, 800.0]));
        assert_eq!(w.get(&5), None);
        assert_eq!(w.get(&19), Some(&[-400.0, 260.0, 710.0]));
        assert_eq!(w.get(&21), Some(&[-400.0, 260.0, 710.0]));
        assert_eq!(w.get(&22), None);
    }

    #[test]
    fn a_malformed_w2_array_keeps_what_was_read_and_never_panics() {
        // A trailing partial triple and a range cut short.
        let w = expand_w2_array(&[
            WEntry::Number(1.0),
            WEntry::Array(vec![-500.0, 250.0, 700.0, -1.0, -2.0]),
            WEntry::Number(9.0),
            WEntry::Number(12.0),
            WEntry::Number(-1.0),
            WEntry::Number(2.0),
        ]);
        assert_eq!(w, BTreeMap::from([(1, [-500.0, 250.0, 700.0])]));
        // An absurd range is rejected, not allocated.
        assert!(expand_w2_array(&[
            WEntry::Number(0.0),
            WEntry::Number(4_000_000_000.0),
            WEntry::Number(-500.0),
            WEntry::Number(250.0),
            WEntry::Number(700.0),
        ])
        .is_empty());
        // A first CID at the top of the u32 space leaves room for one entry, not nine.
        let top = expand_w2_array(&[WEntry::Number(f32::MAX), WEntry::Array(vec![-1.0; 9])]);
        assert_eq!(top.len(), 1);
        let top = expand_w_array(&[WEntry::Number(f32::MAX), WEntry::Array(vec![1.0; 4])]);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn a_vertical_font_reports_w2_then_dw2_with_a_default_origin_at_half_the_width() {
        let m = FontMetrics::Cid {
            widths: BTreeMap::from([(0x50, 800.0)]),
            default_width: 1000.0,
            coding: CidCoding::Identity,
            vertical: Some(VerticalFont {
                default: (900.0, -1100.0),
                widths: BTreeMap::from([(0x50, [-500.0, 250.0, 700.0])]),
            }),
        };
        assert!(m.is_vertical());
        let adv = m.advances(&[0x00, 0x50, 0x00, 0x03]);
        assert_eq!(
            adv[0].vertical,
            Some(VerticalAdvance {
                advance: -500.0,
                origin: (250.0, 700.0)
            })
        );
        assert_eq!(
            adv[1].vertical,
            Some(VerticalAdvance {
                advance: -1100.0,
                origin: (500.0, 900.0)
            })
        );
    }
}
