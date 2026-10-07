//! The text state and the text matrices of a content stream (ISO 32000-1 §9.3–9.4).
//!
//! Text extraction and page rendering interpret the same text operators. Both read them
//! here, so the text one places and the glyphs the other paints stand on the same spot.

use super::backend::{get_number_from_value, ContentOp, GlyphAdvance};

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

    /// How far one glyph moves the text position, in text space (§9.4.4):
    /// `tx = ((w0 / 1000) × Tfs + Tc + Tw) × Th`, with `Tw` only for a word space.
    pub fn glyph_advance(&self, w0: f32, word_space: bool) -> f32 {
        let word = if word_space { self.word_spacing } else { 0.0 };
        (w0 / 1000.0 * self.size + self.char_spacing + word) * self.horizontal_scale
    }

    /// How far showing `glyphs` moves the text position, in text space.
    pub fn advance_of(&self, glyphs: &[GlyphAdvance]) -> f32 {
        glyphs
            .iter()
            .map(|g| self.glyph_advance(g.width, g.is_word_space))
            .sum()
    }

    /// How far a number in a `TJ` array moves the text position: thousandths of text space,
    /// subtracted.
    pub fn adjustment(&self, n: f32) -> f32 {
        -n / 1000.0 * self.size * self.horizontal_scale
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

    /// Move the text position `tx` along the line, as showing text does.
    pub fn advance(&mut self, tx: f32) {
        let (e, f) = self.point(tx, 0.0);
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
    use crate::parser::backend::PdfValue;

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
        // (500/1000 × 10 + 1 + 2) × 0.5
        assert_eq!(params.glyph_advance(500.0, true), 4.0);
        assert_eq!(params.glyph_advance(500.0, false), 3.0);
        assert_eq!(params.adjustment(-200.0), 1.0);
    }
}
