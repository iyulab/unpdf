//! The text state and the text matrices of a content stream (ISO 32000-1 §9.3–9.4).
//!
//! Text extraction and page rendering interpret the same text operators. Both read them
//! here, so the text one places and the glyphs the other paints stand on the same spot.

use std::ops::{Add, AddAssign};

use super::backend::{get_number_from_value, ContentOp, GlyphAdvance};

/// How far showing text moves the text position, in text space: along x in horizontal
/// writing mode, along y in vertical (§9.4.4).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Shift {
    pub tx: f32,
    pub ty: f32,
}

impl Add for Shift {
    type Output = Shift;

    fn add(self, other: Shift) -> Shift {
        Shift {
            tx: self.tx + other.tx,
            ty: self.ty + other.ty,
        }
    }
}

impl AddAssign for Shift {
    fn add_assign(&mut self, other: Shift) {
        *self = *self + other;
    }
}

/// The text state parameters (§9.3), less the font — part of the graphics state, saved and
/// restored by `q`/`Q` and kept across `BT`/`ET`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextParams {
    /// `Tfs`, in unscaled text space units.
    pub size: f32,
    /// `Tc`, in unscaled text space units.
    pub char_spacing: f32,
    /// `Tw`, in unscaled text space units.
    pub word_spacing: f32,
    /// `Th`: `Tz` / 100.
    pub horizontal_scale: f32,
    /// `TL`, in unscaled text space units.
    pub leading: f32,
    /// `Trise`, in unscaled text space units.
    pub rise: f32,
    /// `Tmode`: 0 fills, 3 paints nothing (the mode of an OCR layer over a scan).
    pub render_mode: i64,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            // A content stream must select a font before showing text; 12 keeps text shown
            // without one measurable rather than zero-sized.
            size: 12.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

impl TextParams {
    /// Apply `op` if it sets text state: `Tf`'s size, `Tc`, `Tw`, `Tz`, `TL`, `Ts`, `Tr`,
    /// the leading `TD` sets and the spacing `"` sets. An operator missing an operand
    /// changes nothing. The font `Tf` names is the caller's to resolve.
    pub fn apply(&mut self, op: &ContentOp) {
        let n = |i: usize| op.operands.get(i).and_then(get_number_from_value);
        match op.operator.as_str() {
            "Tf" => set(&mut self.size, n(1)),
            "Tc" => set(&mut self.char_spacing, n(0)),
            "Tw" => set(&mut self.word_spacing, n(0)),
            "Tz" => set(&mut self.horizontal_scale, n(0).map(|v| v / 100.0)),
            "TL" => set(&mut self.leading, n(0)),
            "Ts" => set(&mut self.rise, n(0)),
            "Tr" => set(&mut self.render_mode, n(0).map(|v| v as i64)),
            "TD" => {
                if let (Some(_), Some(ty)) = (n(0), n(1)) {
                    self.leading = -ty;
                }
            }
            "\"" => {
                if let (Some(aw), Some(ac)) = (n(0), n(1)) {
                    self.word_spacing = aw;
                    self.char_spacing = ac;
                }
            }
            _ => {}
        }
    }

    /// How far one glyph moves the text position, in text space (§9.4.4). Horizontally
    /// `tx = ((w0 / 1000) × Tfs + Tc + Tw) × Th`, vertically
    /// `ty = (w1 / 1000) × Tfs + Tc + Tw` — `Th` scales only x. `Tw` applies to a word
    /// space only.
    pub fn glyph_advance(&self, glyph: &GlyphAdvance) -> Shift {
        let word = if glyph.is_word_space {
            self.word_spacing
        } else {
            0.0
        };
        match glyph.vertical {
            None => Shift {
                tx: (glyph.width / 1000.0 * self.size + self.char_spacing + word)
                    * self.horizontal_scale,
                ty: 0.0,
            },
            Some(v) => Shift {
                tx: 0.0,
                ty: v.advance / 1000.0 * self.size + self.char_spacing + word,
            },
        }
    }

    /// How far showing `glyphs` moves the text position, in text space.
    pub fn advance_of(&self, glyphs: &[GlyphAdvance]) -> Shift {
        glyphs
            .iter()
            .fold(Shift::default(), |sum, g| sum + self.glyph_advance(g))
    }

    /// How far a number in a `TJ` array moves the text position: thousandths of text space,
    /// subtracted from the coordinate the font writes along.
    pub fn adjustment(&self, n: f32, vertical: bool) -> Shift {
        if vertical {
            Shift {
                tx: 0.0,
                ty: -n / 1000.0 * self.size,
            }
        } else {
            Shift {
                tx: -n / 1000.0 * self.size * self.horizontal_scale,
                ty: 0.0,
            }
        }
    }

    /// Where `glyph`'s horizontal-mode origin lies relative to the text position, in text
    /// space: a vertical glyph is placed by its position vector (§9.7.4.3), so its origin
    /// is `(vx, vy)` before the text position. Zero for a horizontal glyph.
    pub fn origin_offset(&self, glyph: &GlyphAdvance) -> Shift {
        match glyph.vertical {
            None => Shift::default(),
            Some(v) => Shift {
                tx: -v.origin.0 / 1000.0 * self.size * self.horizontal_scale,
                ty: -v.origin.1 / 1000.0 * self.size,
            },
        }
    }
}

fn set<T>(slot: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *slot = value;
    }
}

/// The text matrix `Tm` and the text line matrix `Tlm` (§9.4.2), `[a, b, c, d, e, f]`.
///
/// Showing text moves `Tm` along the line; `Td`, `TD`, `T*` and friends move to a new
/// line relative to `Tlm`, the start of the current one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextMatrix {
    pub tm: [f32; 6],
    pub tlm: [f32; 6],
}

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

impl Default for TextMatrix {
    fn default() -> Self {
        Self {
            tm: IDENTITY,
            tlm: IDENTITY,
        }
    }
}

impl TextMatrix {
    /// Apply `op` if it positions text: `BT`, `Td`, `TD`, `Tm`, `T*`, and the move to the
    /// next line that `'` and `"` make before showing. An operator missing an operand
    /// changes nothing. `params` gives the leading — apply [`TextParams::apply`] for the
    /// same operator first, as `TD` sets it.
    pub fn apply(&mut self, op: &ContentOp, params: &TextParams) {
        let n = |i: usize| op.operands.get(i).and_then(get_number_from_value);
        match op.operator.as_str() {
            "BT" => *self = Self::default(),
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (n(0), n(1)) {
                    self.translate(tx, ty);
                }
            }
            "Tm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.tm = [a, b, c, d, e, f];
                    self.tlm = self.tm;
                }
            }
            "T*" | "'" | "\"" => self.translate(0.0, -params.leading),
            _ => {}
        }
    }

    /// Start a new line offset `(tx, ty)` from the start of the current one.
    fn translate(&mut self, tx: f32, ty: f32) {
        let [a, b, c, d, e, f] = self.tlm;
        self.tlm = [a, b, c, d, e + tx * a + ty * c, f + tx * b + ty * d];
        self.tm = self.tlm;
    }

    /// Move the text position by `by`, as showing text does.
    pub fn advance(&mut self, by: Shift) {
        let (e, f) = self.point(by.tx, by.ty);
        self.tm[4] = e;
        self.tm[5] = f;
    }

    /// The point `(tx, ty)` of text space relative to the text position — `ty` the rise.
    pub fn point(&self, tx: f32, ty: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.tm;
        (e + tx * a + ty * c, f + tx * b + ty * d)
    }

    /// How much the text matrix scales text vertically: the length its y axis maps to.
    pub fn vertical_scale(&self) -> f32 {
        let [_, _, c, d, ..] = self.tm;
        c.hypot(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::backend::{PdfValue, VerticalAdvance};

    fn op(operator: &str, operands: &[f32]) -> ContentOp {
        ContentOp::new(
            operator,
            operands.iter().map(|&v| PdfValue::Real(v)).collect(),
        )
    }

    #[test]
    fn td_sets_the_leading_that_t_star_then_uses() {
        let (mut params, mut m) = (TextParams::default(), TextMatrix::default());
        for o in [
            op("Tm", &[1.0, 0.0, 0.0, 1.0, 72.0, 700.0]),
            op("TD", &[0.0, -14.0]),
        ] {
            params.apply(&o);
            m.apply(&o, &params);
        }
        assert_eq!(params.leading, 14.0);
        m.apply(&op("T*", &[]), &params);
        assert_eq!(m.point(0.0, 0.0), (72.0, 672.0));
    }

    #[test]
    fn an_operator_missing_an_operand_changes_nothing() {
        let mut params = TextParams::default();
        params.apply(&op("Tz", &[80.0]));
        params.apply(&op("Tz", &[]));
        params.apply(&op("Tc", &[]));
        assert_eq!(params.horizontal_scale, 0.8);
        assert_eq!(params.char_spacing, 0.0);
    }

    #[test]
    fn the_rise_lifts_the_point_along_the_text_y_axis() {
        let mut m = TextMatrix::default();
        m.apply(
            &op("Tm", &[10.0, 0.0, 0.0, 10.0, 100.0, 500.0]),
            &TextParams::default(),
        );
        assert_eq!(m.point(0.0, 0.3), (100.0, 503.0));
    }

    /// A text matrix that narrows glyphs leaves their height alone.
    #[test]
    fn a_horizontally_scaled_matrix_keeps_its_vertical_scale() {
        let mut m = TextMatrix::default();
        m.apply(
            &op("Tm", &[8.0, 0.0, 0.0, 10.0, 0.0, 0.0]),
            &TextParams::default(),
        );
        assert_eq!(m.vertical_scale(), 10.0);
    }

    #[test]
    fn a_glyph_advances_by_its_width_spacing_and_scale() {
        let mut params = TextParams::default();
        for o in [
            op("Tf", &[0.0, 10.0]),
            op("Tc", &[1.0]),
            op("Tw", &[2.0]),
            op("Tz", &[50.0]),
        ] {
            params.apply(&o);
        }
        let glyph = |word| GlyphAdvance {
            width: 500.0,
            vertical: None,
            is_word_space: word,
        };
        // (500/1000 × 10 + 1 + 2) × 0.5
        assert_eq!(params.glyph_advance(&glyph(true)).tx, 4.0);
        assert_eq!(params.glyph_advance(&glyph(false)).tx, 3.0);
        assert_eq!(params.adjustment(-200.0, false).tx, 1.0);
    }

    /// Vertical writing moves along y by `w1`, unscaled by `Tz`, with `Tc` and `Tw` added
    /// as they are; a `TJ` number moves the same way.
    #[test]
    fn a_vertical_glyph_advances_down_by_its_displacement() {
        let mut params = TextParams::default();
        for o in [
            op("Tf", &[0.0, 10.0]),
            op("Tc", &[1.0]),
            op("Tw", &[2.0]),
            op("Tz", &[50.0]),
        ] {
            params.apply(&o);
        }
        let glyph = |word| GlyphAdvance {
            width: 1000.0,
            vertical: Some(VerticalAdvance {
                advance: -1000.0,
                origin: (500.0, 880.0),
            }),
            is_word_space: word,
        };
        // -1000/1000 × 10 + 1 (+ 2 for a word space), and no horizontal scale.
        assert_eq!(
            params.glyph_advance(&glyph(false)),
            Shift { tx: 0.0, ty: -9.0 }
        );
        assert_eq!(
            params.glyph_advance(&glyph(true)),
            Shift { tx: 0.0, ty: -7.0 }
        );
        assert_eq!(params.adjustment(200.0, true), Shift { tx: 0.0, ty: -2.0 });
        // The origin is (vx, vy) before the text position; Th scales x only.
        assert_eq!(
            params.origin_offset(&glyph(false)),
            Shift { tx: -2.5, ty: -8.8 }
        );
    }
}
