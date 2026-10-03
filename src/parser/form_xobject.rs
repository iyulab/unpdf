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
//! The form's `/BBox` clip is not applied: content a form draws outside its own box is rare,
//! and keeping it errs on the side of text that exists.

use std::collections::HashMap;
use std::rc::Rc;

use super::backend::{
    ContentOp, FormXObject, ObjectId, PageId, PaintedXObject, PdfBackend, PdfValue, ResourceScope,
    MAX_FORM_DEPTH,
};
use crate::error::Result;

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
        if self.path.contains(&painted.id) || self.path.len() >= MAX_FORM_DEPTH {
            log::debug!(
                "page {:?}: not painting form {:?} again inside itself",
                self.page,
                painted.id
            );
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

        self.path.push(painted.id);
        self.paint(&ops, Some(painted.id));
        self.path.pop();

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
