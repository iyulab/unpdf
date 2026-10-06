//! Where a page lets marks through — its clip, tracked as a bounding rectangle.
//!
//! A mark the page paints is visible only inside the page's visible region (its crop box,
//! ISO 32000-1 §14.11.2) and inside the current clipping path (§8.5.4). Content placed
//! outside either — the slug area of a print-ready page, the margin of a larger page placed
//! onto a smaller one, a form's content beyond its `/BBox` — is never shown, and a reader
//! that keeps it reports text no one can see.
//!
//! [`ClipTracker`] follows `q`/`Q`, path construction and `W`/`W*` and keeps the clip as the
//! axis-aligned bounding rectangle of the clipping paths intersected so far. A bounding
//! rectangle is a superset of the true clip, so [`ClipTracker::admits`] never rejects a mark
//! that is visible; it can only keep one a non-rectangular clip hides. Text clipping modes
//! (`Tr` 4–7) only ever narrow the clip and are not followed, for the same reason.

use super::backend::{get_number_from_value, ContentOp, PageBox};
use super::layout::apply_ctm;

/// An axis-aligned rectangle in page space; empty when `x1 < x0` or `y1 < y0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bounds {
    pub fn from_page_box(b: PageBox) -> Self {
        Bounds {
            x0: b.llx,
            y0: b.lly,
            x1: b.urx,
            y1: b.ury,
        }
    }

    /// The smallest rectangle holding `points`; `None` for no points.
    pub fn around(points: impl IntoIterator<Item = (f32, f32)>) -> Option<Self> {
        let mut points = points.into_iter();
        let (x, y) = points.next()?;
        let mut b = Bounds {
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        };
        for (x, y) in points {
            b.include(x, y);
        }
        Some(b)
    }

    fn include(&mut self, x: f32, y: f32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }

    pub fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }

    /// The area both cover, if it is not empty.
    pub fn overlap(&self, other: &Bounds) -> Option<Bounds> {
        let r = self.intersect(other);
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    /// The rectangle both cover — empty (not `None`) when they are apart, so a clip that
    /// excludes everything stays excluding everything.
    pub fn intersect(&self, other: &Bounds) -> Bounds {
        Bounds {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    /// Whether the two share any point, edges included — a hairline rule lying along the
    /// clip's edge, or a zero-width mark, still touches it. Nothing touches an empty one.
    pub fn touches(&self, other: &Bounds) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x0 <= other.x1
            && other.x0 <= self.x1
            && self.y0 <= other.y1
            && other.y0 <= self.y1
    }

    fn is_empty(&self) -> bool {
        self.x1 < self.x0 || self.y1 < self.y0
    }
}

/// Follows a page's operators and answers whether a mark lands inside the clip.
///
/// Feed every operator to [`observe`](Self::observe), in order, with the CTM in force when
/// it executes.
pub(crate) struct ClipTracker {
    clip: Bounds,
    stack: Vec<Bounds>,
    /// The path under construction, in page space.
    path: Option<Bounds>,
    /// `W`/`W*` seen: the path becomes part of the clip when it is painted or ended.
    pending: bool,
}

impl ClipTracker {
    /// A tracker whose clip starts as the page's visible region.
    pub fn new(visible: PageBox) -> Self {
        ClipTracker {
            clip: Bounds::from_page_box(visible),
            stack: Vec::new(),
            path: None,
            pending: false,
        }
    }

    pub fn observe(&mut self, op: &ContentOp, ctm: &[f32; 6]) {
        let n = |i: usize| op.operands.get(i).and_then(get_number_from_value);
        match op.operator.as_str() {
            "q" => self.stack.push(self.clip),
            "Q" => {
                if let Some(saved) = self.stack.pop() {
                    self.clip = saved;
                }
            }
            // Every point a segment is built from, control points included: the curve lies
            // inside their hull, so their bounds hold it.
            "m" | "l" | "c" | "v" | "y" => {
                let points = (0..op.operands.len() / 2)
                    .filter_map(|i| Some(apply_ctm(ctm, n(2 * i)?, n(2 * i + 1)?)));
                self.extend_path(points);
            }
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)];
                    self.extend_path(corners.map(|(px, py)| apply_ctm(ctm, px, py)));
                }
            }
            "W" | "W*" => self.pending = true,
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                let path = self.path.take();
                if std::mem::take(&mut self.pending) {
                    // No path at all clips everything away.
                    let empty = Bounds {
                        x0: 0.0,
                        y0: 0.0,
                        x1: -1.0,
                        y1: -1.0,
                    };
                    self.clip = self.clip.intersect(&path.unwrap_or(empty));
                }
            }
            _ => {}
        }
    }

    fn extend_path(&mut self, points: impl IntoIterator<Item = (f32, f32)>) {
        for (x, y) in points {
            match &mut self.path {
                Some(b) => b.include(x, y),
                None => self.path = Bounds::around([(x, y)]),
            }
        }
    }

    /// Whether a mark covering `mark` (page space) can show through the clip.
    pub fn admits(&self, mark: &Bounds) -> bool {
        self.clip.touches(mark)
    }

    /// The part of `mark` the clip lets through, if any.
    pub fn visible_part(&self, mark: &Bounds) -> Option<Bounds> {
        self.clip.overlap(mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::backend::PdfValue;

    const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    const PAGE: PageBox = PageBox {
        llx: 0.0,
        lly: 0.0,
        urx: 600.0,
        ury: 800.0,
    };

    fn op(operator: &str, operands: &[f32]) -> ContentOp {
        ContentOp::new(
            operator,
            operands.iter().map(|&v| PdfValue::Real(v)).collect(),
        )
    }

    fn mark(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
        Bounds { x0, y0, x1, y1 }
    }

    fn run(ops: &[ContentOp], ctm: [f32; 6]) -> ClipTracker {
        let mut t = ClipTracker::new(PAGE);
        for o in ops {
            t.observe(o, &ctm);
        }
        t
    }

    #[test]
    fn the_visible_region_is_the_initial_clip() {
        let t = run(&[], IDENTITY);
        assert!(t.admits(&mark(10.0, 10.0, 50.0, 20.0)));
        assert!(!t.admits(&mark(10.0, 805.0, 50.0, 815.0)), "above the page");
    }

    #[test]
    fn a_clip_path_narrows_the_clip_once_the_path_is_ended() {
        let pending = run(
            &[op("re", &[100.0, 100.0, 50.0, 50.0]), op("W", &[])],
            IDENTITY,
        );
        assert!(
            pending.admits(&mark(300.0, 300.0, 310.0, 310.0)),
            "W takes effect only when the path is painted or ended"
        );
        let t = run(
            &[
                op("re", &[100.0, 100.0, 50.0, 50.0]),
                op("W", &[]),
                op("n", &[]),
            ],
            IDENTITY,
        );
        assert!(t.admits(&mark(110.0, 110.0, 120.0, 120.0)));
        assert!(!t.admits(&mark(300.0, 300.0, 310.0, 310.0)));
    }

    #[test]
    fn restoring_the_graphics_state_restores_the_clip() {
        let t = run(
            &[
                op("q", &[]),
                op("re", &[100.0, 100.0, 50.0, 50.0]),
                op("W", &[]),
                op("n", &[]),
                op("Q", &[]),
            ],
            IDENTITY,
        );
        assert!(t.admits(&mark(300.0, 300.0, 310.0, 310.0)));
    }

    #[test]
    fn a_painted_path_without_w_does_not_clip() {
        let t = run(
            &[op("re", &[100.0, 100.0, 50.0, 50.0]), op("f", &[])],
            IDENTITY,
        );
        assert!(t.admits(&mark(300.0, 300.0, 310.0, 310.0)));
    }

    #[test]
    fn clip_paths_are_mapped_through_the_ctm() {
        let shifted = [1.0, 0.0, 0.0, 1.0, 200.0, 0.0];
        let t = run(
            &[
                op("re", &[0.0, 0.0, 50.0, 50.0]),
                op("W", &[]),
                op("n", &[]),
            ],
            shifted,
        );
        assert!(t.admits(&mark(210.0, 10.0, 220.0, 20.0)));
        assert!(!t.admits(&mark(10.0, 10.0, 20.0, 20.0)));
    }

    #[test]
    fn a_curved_clip_keeps_everything_inside_its_control_hull() {
        let t = run(
            &[
                op("m", &[100.0, 100.0]),
                op("c", &[100.0, 200.0, 200.0, 200.0, 200.0, 100.0]),
                op("h", &[]),
                op("W", &[]),
                op("n", &[]),
            ],
            IDENTITY,
        );
        assert!(t.admits(&mark(150.0, 150.0, 160.0, 160.0)));
        assert!(!t.admits(&mark(150.0, 250.0, 160.0, 260.0)));
    }

    #[test]
    fn disjoint_clips_exclude_everything() {
        let t = run(
            &[
                op("re", &[0.0, 0.0, 10.0, 10.0]),
                op("W", &[]),
                op("n", &[]),
                op("re", &[100.0, 100.0, 10.0, 10.0]),
                op("W", &[]),
                op("n", &[]),
            ],
            IDENTITY,
        );
        assert!(!t.admits(&mark(0.0, 0.0, 600.0, 800.0)));
    }

    #[test]
    fn a_hairline_along_the_clip_edge_is_admitted() {
        let t = run(
            &[
                op("re", &[100.0, 100.0, 50.0, 50.0]),
                op("W", &[]),
                op("n", &[]),
            ],
            IDENTITY,
        );
        assert!(t.admits(&mark(100.0, 150.0, 150.0, 150.0)));
    }
}
