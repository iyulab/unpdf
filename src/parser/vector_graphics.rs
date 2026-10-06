//! Vector graphics (line/rectangle) extraction from PDF content streams.
//!
//! Foundation for lattice-mode table detection (explicit ruling lines and cell
//! borders), complementing `table_detector`'s text-alignment-based stream mode.
//! Pure and backend-agnostic: takes the same [`ContentOp`] sequence the layout
//! analyzer already decodes, and returns straight line segments in device space
//! with no PDF-library-specific types crossing the boundary.
//!
//! Scope: this module only extracts geometry. Clustering lines into a table grid
//! (row/column boundaries, cell intersections) is a separate, later stage.

use super::backend::{get_number_from_value, ContentOp, PdfValue};
use super::layout::{apply_cm, apply_ctm};

/// A straight line segment in device space, as painted by a content stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphicsLine {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Drawn as a line: stroked, or the outline of a filled shape thin enough to be a
    /// rule. `false` for the outline of a filled area — a chart bar, a shaded band —
    /// which can complete a ruled grid but is no ruling of its own.
    pub ruled: bool,
}

/// How thick a filled shape may be and still be a rule. Generators that draw table rules
/// as filled rectangles (word processors, LaTeX) make them a point or two thick.
const MAX_FILLED_RULE_THICKNESS: f32 = 3.0;

/// Extract every straight line segment painted by a content stream, in device-space
/// coordinates, each marked [`GraphicsLine::ruled`] when it is drawn as a line — a
/// stroked path, or the outline of a filled shape thin enough to be a rule.
///
/// The outline of a filled area is still returned: row shading and a frame painted as
/// one filled ring complete a table's grid. But it is no rule of its own — the bars of a
/// bar chart are filled areas, and a grid made only of their outlines turned the chart's
/// value labels into a table. A path filled *and* stroked (`B b`) has its outline drawn.
///
/// Only path-painting operators (`S s f F f* B B* b b*`) produce visible marks —
/// a path built but never painted (or terminated with `n`, the paint-nothing
/// operator used for clip-only paths) emits nothing, matching PDF semantics.
/// Curve segments (`c v y`) are not straight lines: the current point advances
/// to the curve's end so subsequent segments stay correctly anchored, but the
/// curve itself is not approximated as a line.
pub fn extract_lines(ops: &[ContentOp]) -> Vec<GraphicsLine> {
    let mut lines = Vec::new();

    let mut ctm: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut ctm_stack: Vec<[f32; 6]> = Vec::new();

    // Path construction state. Not part of the graphics state stack (q/Q do
    // not save/restore it) — points are transformed through `ctm` as they are
    // appended, per PDF semantics (path construction uses user space at the
    // time each operator executes).
    let mut current: Option<(f32, f32)> = None;
    let mut subpath_start: Option<(f32, f32)> = None;
    // Segments of the path under construction, one entry per subpath: a fill keeps or
    // drops each subpath by its own extent.
    let mut pending: Vec<Vec<GraphicsLine>> = Vec::new();

    for op in ops {
        match op.operator.as_str() {
            "q" => ctm_stack.push(ctm),
            "Q" => {
                if let Some(saved) = ctm_stack.pop() {
                    ctm = saved;
                }
            }
            "cm" if op.operands.len() >= 6 => {
                let cm = [
                    num(&op.operands[0]),
                    num(&op.operands[1]),
                    num(&op.operands[2]),
                    num(&op.operands[3]),
                    num(&op.operands[4]),
                    num(&op.operands[5]),
                ];
                ctm = apply_cm(&ctm, &cm);
            }
            "m" if op.operands.len() >= 2 => {
                let p = apply_ctm(&ctm, num(&op.operands[0]), num(&op.operands[1]));
                current = Some(p);
                subpath_start = Some(p);
                pending.push(Vec::new());
            }
            "l" if op.operands.len() >= 2 => {
                let p = apply_ctm(&ctm, num(&op.operands[0]), num(&op.operands[1]));
                if let Some((x0, y0)) = current {
                    if pending.is_empty() {
                        pending.push(Vec::new());
                    }
                    if let Some(subpath) = pending.last_mut() {
                        subpath.push(GraphicsLine {
                            x0,
                            y0,
                            x1: p.0,
                            y1: p.1,
                            ruled: true,
                        });
                    }
                }
                current = Some(p);
            }
            "c" if op.operands.len() >= 6 => {
                let end = apply_ctm(&ctm, num(&op.operands[4]), num(&op.operands[5]));
                current = Some(end);
            }
            "v" if op.operands.len() >= 4 => {
                let end = apply_ctm(&ctm, num(&op.operands[2]), num(&op.operands[3]));
                current = Some(end);
            }
            "y" if op.operands.len() >= 4 => {
                let end = apply_ctm(&ctm, num(&op.operands[2]), num(&op.operands[3]));
                current = Some(end);
            }
            "h" => {
                if let (Some((x0, y0)), Some((x1, y1))) = (current, subpath_start) {
                    if (x0, y0) != (x1, y1) {
                        if let Some(subpath) = pending.last_mut() {
                            subpath.push(GraphicsLine {
                                x0,
                                y0,
                                x1,
                                y1,
                                ruled: true,
                            });
                        }
                    }
                    current = subpath_start;
                }
            }
            "re" if op.operands.len() >= 4 => {
                let x = num(&op.operands[0]);
                let y = num(&op.operands[1]);
                let w = num(&op.operands[2]);
                let h = num(&op.operands[3]);
                let p0 = apply_ctm(&ctm, x, y);
                let p1 = apply_ctm(&ctm, x + w, y);
                let p2 = apply_ctm(&ctm, x + w, y + h);
                let p3 = apply_ctm(&ctm, x, y + h);
                pending.push(vec![seg(p0, p1), seg(p1, p2), seg(p2, p3), seg(p3, p0)]);
                current = Some(p0);
                subpath_start = Some(p0);
            }
            "S" | "s" | "B" | "B*" | "b" | "b*" => {
                lines.extend(pending.drain(..).flatten());
                current = None;
                subpath_start = None;
            }
            "f" | "F" | "f*" => {
                for subpath in pending.drain(..) {
                    let ruled = is_rule_thin(&subpath);
                    lines.extend(
                        subpath
                            .into_iter()
                            .map(|line| GraphicsLine { ruled, ..line }),
                    );
                }
                current = None;
                subpath_start = None;
            }
            "n" => {
                // Clip-only (or abandoned) path — no marks painted.
                pending.clear();
                current = None;
                subpath_start = None;
            }
            _ => {}
        }
    }

    lines
}

/// Whether a filled subpath is thin enough to be a rule: its extent along one axis is at
/// most [`MAX_FILLED_RULE_THICKNESS`].
fn is_rule_thin(subpath: &[GraphicsLine]) -> bool {
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for line in subpath {
        min_x = min_x.min(line.x0).min(line.x1);
        max_x = max_x.max(line.x0).max(line.x1);
        min_y = min_y.min(line.y0).min(line.y1);
        max_y = max_y.max(line.y0).max(line.y1);
    }
    !subpath.is_empty() && (max_x - min_x).min(max_y - min_y) <= MAX_FILLED_RULE_THICKNESS
}

fn num(v: &PdfValue) -> f32 {
    get_number_from_value(v).unwrap_or(0.0)
}

fn seg(a: (f32, f32), b: (f32, f32)) -> GraphicsLine {
    GraphicsLine {
        x0: a.0,
        y0: a.1,
        x1: b.0,
        y1: b.1,
        ruled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(operator: &str, operands: &[f32]) -> ContentOp {
        ContentOp::new(
            operator.to_string(),
            operands.iter().map(|n| PdfValue::Real(*n)).collect(),
        )
    }

    #[test]
    fn straight_line_stroked() {
        let ops = vec![
            op("m", &[100.0, 700.0]),
            op("l", &[200.0, 700.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(
            lines,
            vec![GraphicsLine {
                x0: 100.0,
                y0: 700.0,
                x1: 200.0,
                y1: 700.0,
                ruled: true,
            }]
        );
    }

    /// A rule drawn as a thin filled rectangle, as word processors and LaTeX draw them.
    #[test]
    fn thin_filled_rectangle_emits_four_edges() {
        let ops = vec![op("re", &[0.0, 0.0, 100.0, 0.8]), op("f", &[])];
        let lines = extract_lines(&ops);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            GraphicsLine {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 0.0,
                ruled: true,
            }
        );
    }

    /// A filled area — a chart bar, a shaded box — is outlined but not ruled.
    #[test]
    fn filled_area_outline_is_not_ruled() {
        let ops = vec![op("re", &[0.0, 0.0, 100.0, 20.0]), op("f", &[])];
        let lines = extract_lines(&ops);
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| !l.ruled));
    }

    /// Thickness is judged per subpath: one fill painting a rule and a block.
    #[test]
    fn a_fill_rules_its_thin_subpaths_only() {
        let ops = vec![
            op("re", &[0.0, 0.0, 100.0, 1.0]),
            op("re", &[0.0, 10.0, 100.0, 40.0]),
            op("f", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(lines.iter().filter(|l| l.ruled).count(), 4);
        assert_eq!(lines.iter().filter(|l| !l.ruled).count(), 4);
    }

    /// Filled and stroked, the outline is drawn.
    #[test]
    fn filled_and_stroked_area_is_ruled() {
        let ops = vec![op("re", &[0.0, 0.0, 100.0, 20.0]), op("B", &[])];
        let lines = extract_lines(&ops);
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.ruled));
    }

    #[test]
    fn rectangle_stroked_also_emits_four_edges() {
        let ops = vec![op("re", &[10.0, 10.0, 50.0, 5.0]), op("S", &[])];
        let lines = extract_lines(&ops);
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn cm_transform_applied_to_points() {
        // Translate by (100, 200), then draw a line from (0,0) to (10,0).
        let ops = vec![
            op("cm", &[1.0, 0.0, 0.0, 1.0, 100.0, 200.0]),
            op("m", &[0.0, 0.0]),
            op("l", &[10.0, 0.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(
            lines,
            vec![GraphicsLine {
                x0: 100.0,
                y0: 200.0,
                x1: 110.0,
                y1: 200.0,
                ruled: true,
            }]
        );
    }

    #[test]
    fn nested_cm_applies_the_newer_matrix_first() {
        // Scale by 2 first, then translate by (10, 20) in the scaled space:
        // the translation is itself scaled, so (0,0) lands on (20, 40), not (10, 20).
        let ops = vec![
            op("cm", &[2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
            op("cm", &[1.0, 0.0, 0.0, 1.0, 10.0, 20.0]),
            op("m", &[0.0, 0.0]),
            op("l", &[5.0, 0.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(
            lines,
            vec![GraphicsLine {
                x0: 20.0,
                y0: 40.0,
                x1: 30.0,
                y1: 40.0,
                ruled: true,
            }]
        );
    }

    #[test]
    fn q_q_isolates_transform() {
        let ops = vec![
            op("q", &[]),
            op("cm", &[1.0, 0.0, 0.0, 1.0, 1000.0, 1000.0]),
            op("Q", &[]),
            // After Q, ctm is back to identity — this line should NOT be translated.
            op("m", &[0.0, 0.0]),
            op("l", &[5.0, 0.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(
            lines,
            vec![GraphicsLine {
                x0: 0.0,
                y0: 0.0,
                x1: 5.0,
                y1: 0.0,
                ruled: true,
            }]
        );
    }

    #[test]
    fn unpainted_path_emits_nothing() {
        let ops = vec![op("m", &[0.0, 0.0]), op("l", &[10.0, 0.0])];
        assert!(extract_lines(&ops).is_empty());
    }

    #[test]
    fn clip_only_path_emits_nothing() {
        let ops = vec![
            op("re", &[0.0, 0.0, 100.0, 100.0]),
            ContentOp::new("W".to_string(), vec![]),
            op("n", &[]),
        ];
        assert!(extract_lines(&ops).is_empty());
    }

    #[test]
    fn closepath_connects_back_to_subpath_start() {
        let ops = vec![
            op("m", &[0.0, 0.0]),
            op("l", &[10.0, 0.0]),
            op("l", &[10.0, 10.0]),
            ContentOp::new("h".to_string(), vec![]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        // Two explicit `l` segments plus the closing segment back to (0,0).
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[2],
            GraphicsLine {
                x0: 10.0,
                y0: 10.0,
                x1: 0.0,
                y1: 0.0,
                ruled: true,
            }
        );
    }

    #[test]
    fn curve_advances_current_point_without_emitting_a_line() {
        // c: control1, control2, end — end point is (30, 0).
        let ops = vec![
            op("m", &[0.0, 0.0]),
            op("c", &[5.0, 20.0, 25.0, 20.0, 30.0, 0.0]),
            // Straight segment continues from the curve's end point.
            op("l", &[40.0, 0.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(
            lines,
            vec![GraphicsLine {
                x0: 30.0,
                y0: 0.0,
                x1: 40.0,
                y1: 0.0,
                ruled: true,
            }]
        );
    }

    #[test]
    fn multiple_subpaths_before_one_paint_op() {
        // Two disjoint segments, both painted by the single trailing `S`.
        let ops = vec![
            op("m", &[0.0, 0.0]),
            op("l", &[10.0, 0.0]),
            op("m", &[100.0, 100.0]),
            op("l", &[110.0, 100.0]),
            op("S", &[]),
        ];
        let lines = extract_lines(&ops);
        assert_eq!(lines.len(), 2);
    }
}
