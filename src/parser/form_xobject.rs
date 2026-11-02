//! Form XObjects, interpreted where they are painted.
//!
//! A Form XObject is a content stream of its own that a page paints with `Do` (ISO 32000-1
//! §8.10). Painting it is defined as: save the graphics state, concatenate the form's
//! `/Matrix` onto the CTM, run the form's operators with the form's `/Resources`, restore the
//! graphics state. So everything a form paints -- text, images, ruling lines -- is part of
//! the page, and every reader of a page's operators has to see it.
//!
//! [`page_operations`] does that once, for every reader: a form's `Do` is replaced by
//! `q`, `cm <Matrix>`, the form's operators and `Q`, each operator tagged with the form it
//! came from so its names resolve in the form's resources ([`ContentOp::scope`]). What is
//! left as `Do` paints an image (or something that is neither image nor form).
//!
//! The form's `/BBox` clips what it paints, so it is emitted as a clip path (`re W n`) right
//! after the matrix: a reader that follows clipping sees the form exactly as a viewer shows
//! it, and one that does not is unaffected.

use std::collections::HashMap;
use std::rc::Rc;

use super::backend::{
    ContentOp, FormXObject, ObjectId, PageId, PaintedXObject, PdfBackend, PdfValue, ResourceScope,
    MAX_FORM_DEPTH,
};
use crate::error::Result;

/// Pseudo-operators around a transparency group's content: everything between them is one
/// group, composited as a whole where it is painted. Never a real operator -- an operator in
/// a content stream is a run of letters and digits.
pub(crate) const BEGIN_GROUP: &str = "{group";
pub(crate) const END_GROUP: &str = "}group";

/// Pseudo-operator where a form nested past [`MAX_FORM_DEPTH`] was left unpainted, so a reader
/// that paints can count what it lost.
pub(crate) const FORM_TOO_DEEP: &str = "{deep";

/// A page's operators, with the Form XObjects it paints interpreted in place.
pub(crate) struct PageOperations {
    pub ops: Vec<ContentOp>,
    /// Form XObjects painted -- each `Do` of a form that was interpreted.
    pub forms_painted: u32,
    /// Content streams that could not be decoded: the page's own, and those of the forms it
    /// paints. Each is content the page lost.
    pub undecodable_streams: usize,
}

/// Decode `page`'s content and interpret the Form XObjects it paints.
pub(crate) fn page_operations(backend: &dyn PdfBackend, page: PageId) -> Result<PageOperations> {
    let content = backend.page_content_with_losses(page)?;
    let ops = backend.decode_content(&content.data)?;

    let mut expander = Expander {
        backend,
        page,
        out: Vec::with_capacity(ops.len()),
        forms_painted: 0,
        undecodable_streams: content.undecodable_streams,
        decoded: HashMap::new(),
        path: Vec::new(),
    };
    expander.paint(&ops, None);

    Ok(PageOperations {
        ops: expander.out,
        forms_painted: expander.forms_painted,
        undecodable_streams: expander.undecodable_streams,
    })
}

/// The operators of content painted with `form`'s resources -- a tiling pattern's cell, whose
/// operators already carry it as their form -- with the Form XObjects it paints interpreted in
/// place. `undecodable_streams` counts the forms whose streams could not be decoded.
#[cfg(feature = "raster")]
pub(crate) fn content_operations(
    backend: &dyn PdfBackend,
    page: PageId,
    ops: &[ContentOp],
    form: ObjectId,
) -> PageOperations {
    let mut expander = Expander {
        backend,
        page,
        out: Vec::with_capacity(ops.len()),
        forms_painted: 0,
        undecodable_streams: 0,
        decoded: HashMap::new(),
        // Its own content painting it again would never finish.
        path: vec![form],
    };
    expander.paint(ops, Some(form));
    PageOperations {
        ops: expander.out,
        forms_painted: expander.forms_painted,
        undecodable_streams: expander.undecodable_streams,
    }
}

struct Expander<'a> {
    backend: &'a dyn PdfBackend,
    page: PageId,
    out: Vec<ContentOp>,
    forms_painted: u32,
    undecodable_streams: usize,
    /// Each form's operators, decoded once per page however often it is painted; `None` when
    /// its stream could not be decoded.
    decoded: HashMap<ObjectId, Option<Rc<Vec<ContentOp>>>>,
    /// The forms being interpreted, outermost first: a form that paints one of them (itself
    /// included) would never finish.
    path: Vec<ObjectId>,
}

impl Expander<'_> {
    fn paint(&mut self, ops: &[ContentOp], form: Option<ObjectId>) {
        for op in ops {
            if let Some(painted) = self.form_painted_by(op, form) {
                self.paint_form(painted, form);
                continue;
            }
            let mut op = op.clone();
            op.form = form;
            self.out.push(op);
        }
    }

    /// The form `op` paints, when it is a `Do` of one.
    fn form_painted_by(&self, op: &ContentOp, form: Option<ObjectId>) -> Option<FormXObject> {
        if op.operator != "Do" {
            return None;
        }
        let Some(PdfValue::Name(name)) = op.operands.first() else {
            return None;
        };
        let scope = ResourceScope {
            page: self.page,
            form,
        };
        match self.backend.xobject(scope, name)? {
            PaintedXObject::Form(painted) => Some(painted),
            PaintedXObject::Image | PaintedXObject::Other => None,
        }
    }

    fn paint_form(&mut self, painted: FormXObject, outer: Option<ObjectId>) {
        let cycle = self.path.contains(&painted.id);
        if cycle || self.path.len() >= MAX_FORM_DEPTH {
            log::debug!(
                "page {:?}: not painting form {:?} again inside itself",
                self.page,
                painted.id
            );
            if !cycle {
                let mut cut = ContentOp::new(FORM_TOO_DEEP, Vec::new());
                cut.form = outer;
                self.out.push(cut);
            }
            return;
        }
        self.forms_painted += 1;
        let Some(ops) = self.decode(&painted) else {
            return;
        };

        let mut save = ContentOp::new("q", Vec::new());
        save.form = outer;
        self.out.push(save);
        if painted.matrix != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            let operands = painted.matrix.iter().map(|&n| PdfValue::Real(n)).collect();
            let mut cm = ContentOp::new("cm", operands);
            cm.form = outer;
            self.out.push(cm);
        }
        if let Some(b) = painted.bbox {
            let rect = [b.llx, b.lly, b.width(), b.height()]
                .map(PdfValue::Real)
                .to_vec();
            for (operator, operands) in [("re", rect), ("W", Vec::new()), ("n", Vec::new())] {
                let mut clip = ContentOp::new(operator, operands);
                clip.form = outer;
                self.out.push(clip);
            }
        }

        if painted.transparency_group {
            let mut begin = ContentOp::new(BEGIN_GROUP, Vec::new());
            begin.form = outer;
            self.out.push(begin);
        }
        self.path.push(painted.id);
        self.paint(&ops, Some(painted.id));
        self.path.pop();
        if painted.transparency_group {
            let mut end = ContentOp::new(END_GROUP, Vec::new());
            end.form = outer;
            self.out.push(end);
        }

        let mut restore = ContentOp::new("Q", Vec::new());
        restore.form = outer;
        self.out.push(restore);
    }

    fn decode(&mut self, painted: &FormXObject) -> Option<Rc<Vec<ContentOp>>> {
        if let Some(cached) = self.decoded.get(&painted.id) {
            return cached.clone();
        }
        let ops = painted
            .content
            .as_deref()
            .and_then(|data| self.backend.decode_content(data).ok())
            .map(Rc::new);
        if ops.is_none() {
            log::warn!(
                "page {:?}: form {:?} has a content stream that could not be decoded",
                self.page,
                painted.id
            );
            self.undecodable_streams += 1;
        }
        self.decoded.insert(painted.id, ops.clone());
        ops
    }
}
