//! A page as it is shown: `/Rotate` applied to the page's box and to everything drawn on it.
//!
//! A viewer turns the page a quarter, a half or three quarters (ISO 32000-1 §7.7.3.3, `/Rotate`)
//! before it shows it, and a page set across the sheet is usually written the other way round:
//! a landscape table in a portrait content stream. Read in the unturned space its rules run the
//! wrong way and every column is a line of text. The rasterizer already paints the turned page;
//! this is the same turn for the text and the rules, as the first matrix of the content stream,
//! so every position the readers work with is a position on the page as shown.

use super::backend::PageBox;

/// The turn of one page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PageTurn {
    /// The matrix `[a b c d e f]` that takes a point of the page's user space to the same point
    /// of the page as shown. The identity for a page that is not turned.
    pub ctm: [f32; 6],
    /// The page's box as shown: the box itself for a page that is not turned, otherwise the
    /// turned box at the origin.
    pub shown: PageBox,
}

impl PageTurn {
    /// The turn of a page whose box is `raw` and whose `/Rotate` is `rotation` (a multiple of
    /// 90, clockwise).
    pub fn new(raw: PageBox, rotation: u16) -> Self {
        let (w, h) = (raw.width(), raw.height());
        let (x, y) = (raw.llx, raw.lly);
        let at_origin = |width: f32, height: f32| PageBox {
            llx: 0.0,
            lly: 0.0,
            urx: width,
            ury: height,
        };
        match rotation % 360 {
            // The left edge becomes the top, the top the right.
            90 => Self {
                ctm: [0.0, -1.0, 1.0, 0.0, -y, w + x],
                shown: at_origin(h, w),
            },
            180 => Self {
                ctm: [-1.0, 0.0, 0.0, -1.0, w + x, h + y],
                shown: at_origin(w, h),
            },
            // The left edge becomes the bottom, the top the left.
            270 => Self {
                ctm: [0.0, 1.0, -1.0, 0.0, h + y, -x],
                shown: at_origin(h, w),
            },
            _ => Self {
                ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                shown: raw,
            },
        }
    }

    /// Where a point of the page's user space lies on the page as shown.
    pub fn apply(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.ctm;
        (a * x + c * y + e, b * x + d * y + f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page whose box does not start at the origin, so a turn that forgets the offset shows.
    const BOX: PageBox = PageBox {
        llx: 10.0,
        lly: 20.0,
        urx: 110.0,
        ury: 220.0,
    };

    /// The corners of the box go where a viewer shows them: turned clockwise, the lower left
    /// corner goes to the upper left, and so on round.
    #[test]
    fn the_corners_go_where_a_viewer_shows_them() {
        let corners = |rotation| {
            let turn = PageTurn::new(BOX, rotation);
            [(10.0, 20.0), (10.0, 220.0), (110.0, 220.0), (110.0, 20.0)].map(|p| turn.apply(p))
        };
        // Lower left, upper left, upper right, lower right, as shown.
        assert_eq!(
            corners(90),
            [(0.0, 100.0), (200.0, 100.0), (200.0, 0.0), (0.0, 0.0)]
        );
        assert_eq!(
            corners(180),
            [(100.0, 200.0), (100.0, 0.0), (0.0, 0.0), (0.0, 200.0)]
        );
        assert_eq!(
            corners(270),
            [(200.0, 0.0), (0.0, 0.0), (0.0, 100.0), (200.0, 100.0)]
        );
    }

    #[test]
    fn the_shown_box_is_the_turned_box_at_the_origin() {
        for rotation in [90, 270] {
            let shown = PageTurn::new(BOX, rotation).shown;
            assert_eq!(
                (shown.llx, shown.lly, shown.urx, shown.ury),
                (0.0, 0.0, 200.0, 100.0)
            );
        }
        let shown = PageTurn::new(BOX, 180).shown;
        assert_eq!(
            (shown.llx, shown.lly, shown.urx, shown.ury),
            (0.0, 0.0, 100.0, 200.0)
        );
    }

    /// A page that is not turned keeps its box, offset and all, and the identity: nothing a
    /// reader worked out for it changes.
    #[test]
    fn a_page_that_is_not_turned_is_left_alone() {
        let turn = PageTurn::new(BOX, 0);
        assert_eq!(turn.ctm, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(turn.shown, BOX);
        assert_eq!(turn.apply((33.0, 44.0)), (33.0, 44.0));
    }
}
