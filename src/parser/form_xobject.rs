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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::rc::Rc;

use super::backend::{
    ContentOp, FormXObject, GroupAttributes, ObjectId, PageBox, PageId, PaintedXObject, PdfBackend,
    PdfValue, ResourceScope, MAX_FORM_DEPTH,
};
use crate::error::Result;

/// Pseudo-operators around a transparency group's content: everything between them is one
/// group, composited as a whole where it is painted. Never a real operator -- an operator in
/// a content stream is a run of letters and digits. The group's `/I` and `/K`, when true, are
/// [`BEGIN_GROUP`]'s operands, as names.
pub(crate) const BEGIN_GROUP: &str = "{group";
pub(crate) const END_GROUP: &str = "}group";

/// Pseudo-operator where a form nested past [`MAX_FORM_DEPTH`] was left unpainted, so a reader
/// that paints can count what it lost.
pub(crate) const FORM_TOO_DEEP: &str = "{deep";

/// Pseudo-operator for a `Do` of a form, before the form is read or found to be painting itself
/// again: a step for a reader that charges each one. Never among the operators collected for a
/// page.
pub(crate) const FORM_DO: &str = "{do";

/// Takes each operator as it is interpreted, until it breaks.
pub(crate) type Emit<'e> = dyn FnMut(ContentOp) -> ControlFlow<()> + 'e;

/// A page's operators, with the Form XObjects it paints interpreted in place.
pub(crate) struct PageOperations {
    pub ops: Vec<ContentOp>,
    /// Form XObjects painted -- each `Do` of a form that was interpreted.
    pub forms_painted: u32,
    /// Content streams that could not be decoded: the page's own, and those of the forms it
    /// paints. Each is content the page lost.
    pub undecodable_streams: usize,
}

/// The Form XObjects read for a page: each is read and decoded once, however often and from
/// wherever it is painted, and one whose stream does not decode is counted once. Clones share
/// them.
#[derive(Clone, Default)]
pub(crate) struct Forms(Rc<RefCell<PageForms>>);

#[derive(Default)]
struct PageForms {
    decoded: HashMap<ObjectId, Rc<Form>>,
    undecodable: HashSet<ObjectId>,
}

impl Forms {
    fn get(&self, id: ObjectId) -> Option<Rc<Form>> {
        self.0.borrow().decoded.get(&id).cloned()
    }

    /// `read` with its operators decoded, kept to be painted again.
    fn decode(&self, backend: &dyn PdfBackend, page: PageId, read: &FormXObject) -> Rc<Form> {
        let ops = read
            .content
            .as_deref()
            .and_then(|data| backend.decode_content(data).ok());
        if ops.is_none() {
            log::warn!(
                "page {:?}: form {:?} has a content stream that could not be decoded",
                page,
                read.id
            );
        }
        let form = Rc::new(Form {
            id: read.id,
            matrix: read.matrix,
            bbox: read.bbox,
            group: read
                .transparency_group
                .then(|| backend.group_attributes(read.id)),
            ops,
        });
        self.0
            .borrow_mut()
            .decoded
            .insert(form.id, Rc::clone(&form));
        form
    }

    /// Form XObject `id`, read and decoded once for the page; `None` when it is not a stream.
    pub(crate) fn form(
        &self,
        backend: &dyn PdfBackend,
        page: PageId,
        id: ObjectId,
    ) -> Option<Rc<Form>> {
        self.get(id).or_else(|| {
            backend
                .form_xobject(id)
                .map(|read| self.decode(backend, page, &read))
        })
    }

    /// `group`, a soft mask's group, decoded once for the page.
    #[cfg(feature = "raster")]
    pub(crate) fn mask_group(
        &self,
        backend: &dyn PdfBackend,
        page: PageId,
        group: &FormXObject,
    ) -> Rc<Form> {
        self.get(group.id)
            .unwrap_or_else(|| self.decode(backend, page, group))
    }

    /// Whether `id` is counted as a stream that does not decode for the first time.
    fn newly_undecodable(&self, id: ObjectId) -> bool {
        self.0.borrow_mut().undecodable.insert(id)
    }
}

/// Decode `page`'s content and interpret the Form XObjects it paints.
pub(crate) fn page_operations(backend: &dyn PdfBackend, page: PageId) -> Result<PageOperations> {
    page_operations_with(backend, page, &Forms::default())
}

/// [`page_operations`], reading forms into `forms`.
pub(crate) fn page_operations_with(
    backend: &dyn PdfBackend,
    page: PageId,
    forms: &Forms,
) -> Result<PageOperations> {
    let content = backend.page_content_with_losses(page)?;
    let ops = backend.decode_content(&content.data)?;

    let mut expander = Expander::new(backend, page, forms, Vec::new());
    expander.undecodable_streams = content.undecodable_streams;
    let mut out = Vec::with_capacity(ops.len());
    let _ = expander.paint(&ops, None, &mut |op| {
        if op.operator != FORM_DO {
            out.push(op);
        }
        ControlFlow::Continue(())
    });

    Ok(PageOperations {
        ops: out,
        forms_painted: expander.forms_painted,
        undecodable_streams: expander.undecodable_streams,
    })
}

/// Hand `emit` the operators of content painted with `form`'s resources -- a tiling pattern's
/// cell or a soft mask's group, whose operators already carry it as their form -- with the Form
/// XObjects it paints interpreted in place, reading them into `forms`, until it breaks. Returns
/// how many of those forms' streams could not be decoded.
#[cfg(feature = "raster")]
pub(crate) fn paint_content(
    backend: &dyn PdfBackend,
    page: PageId,
    forms: &Forms,
    ops: &[ContentOp],
    form: ObjectId,
    emit: &mut Emit<'_>,
) -> usize {
    let painting = vec![form];
    let mut expander = Expander::new(backend, page, forms, painting);
    let _ = expander.paint(ops, Some(form), emit);
    expander.undecodable_streams
}

/// The appearances of a page's annotations, interpreted one at a time as Form XObjects.
#[cfg(feature = "raster")]
pub(crate) struct Appearances<'a> {
    expander: Expander<'a>,
    /// Appearances whose stream could not be read or decoded.
    pub unpainted: u32,
}

#[cfg(feature = "raster")]
impl<'a> Appearances<'a> {
    pub(crate) fn new(backend: &'a dyn PdfBackend, page: PageId, forms: &Forms) -> Self {
        Appearances {
            expander: Expander::new(backend, page, forms, Vec::new()),
            unpainted: 0,
        }
    }

    /// Hand `emit` the operators of `appearance` -- the placement onto the annotation's
    /// rectangle, then the appearance as a Form XObject -- until it breaks. The stream is read
    /// once the placement is taken.
    pub(crate) fn paint(
        &mut self,
        appearance: &super::backend::AnnotationAppearance,
        emit: &mut Emit<'_>,
    ) -> ControlFlow<()> {
        emit(ContentOp::new(
            "cm",
            appearance.placement.map(PdfValue::Real).to_vec(),
        ))?;
        let expander = &mut self.expander;
        let form = expander
            .forms
            .form(expander.backend, expander.page, appearance.form);
        match form.filter(|form| form.ops.is_some()) {
            Some(form) => expander.paint_form(&form, None, emit),
            None => {
                self.unpainted += 1;
                ControlFlow::Continue(())
            }
        }
    }

    /// Content streams nested in the appearances that could not be decoded.
    pub(crate) fn undecodable_streams(&self) -> usize {
        self.expander.undecodable_streams
    }
}

/// A Form XObject as painted: where it goes, the transparency group it is if any, and its
/// operators -- `None` when its stream could not be decoded.
pub(crate) struct Form {
    id: ObjectId,
    pub(crate) matrix: [f32; 6],
    pub(crate) bbox: Option<PageBox>,
    pub(crate) group: Option<GroupAttributes>,
    pub(crate) ops: Option<Vec<ContentOp>>,
}

/// The form a `Do` paints: found by its object id without reading it, or read to find out.
enum Painted {
    Found(ObjectId),
    Read(FormXObject),
}

struct Expander<'a> {
    backend: &'a dyn PdfBackend,
    page: PageId,
    forms: Forms,
    forms_painted: u32,
    undecodable_streams: usize,
    /// The forms being interpreted, outermost first: a form that paints one of them (itself
    /// included) would never finish.
    path: Vec<ObjectId>,
}

impl<'a> Expander<'a> {
    fn new(backend: &'a dyn PdfBackend, page: PageId, forms: &Forms, path: Vec<ObjectId>) -> Self {
        Expander {
            backend,
            page,
            forms: forms.clone(),
            forms_painted: 0,
            undecodable_streams: 0,
            path,
        }
    }

    fn paint(
        &mut self,
        ops: &[ContentOp],
        form: Option<ObjectId>,
        emit: &mut Emit<'_>,
    ) -> ControlFlow<()> {
        for op in ops {
            if let Some(painted) = self.form_painted_by(op, form) {
                self.paint_do(painted, form, emit)?;
                continue;
            }
            let mut op = op.clone();
            op.form = form;
            emit(op)?;
        }
        ControlFlow::Continue(())
    }

    /// The form `op` paints, when it is a `Do` of one.
    fn form_painted_by(&self, op: &ContentOp, form: Option<ObjectId>) -> Option<Painted> {
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
        if let Some(id) = self.backend.form_xobject_id(scope, name) {
            return Some(Painted::Found(id));
        }
        match self.backend.xobject(scope, name)? {
            PaintedXObject::Form(read) => Some(Painted::Read(read)),
            PaintedXObject::Image | PaintedXObject::Other => None,
        }
    }

    /// Paint the form a `Do` paints, unless it is being painted already or nests too deep; each
    /// `Do` is one step, taken before the form is read.
    fn paint_do(
        &mut self,
        painted: Painted,
        outer: Option<ObjectId>,
        emit: &mut Emit<'_>,
    ) -> ControlFlow<()> {
        let id = match &painted {
            Painted::Found(id) => *id,
            Painted::Read(form) => form.id,
        };
        let cycle = self.path.contains(&id);
        if cycle || self.path.len() >= MAX_FORM_DEPTH {
            log::debug!(
                "page {:?}: not painting form {:?} again inside itself",
                self.page,
                id
            );
            let step = if cycle { FORM_DO } else { FORM_TOO_DEEP };
            return emit(in_form(step, Vec::new(), outer));
        }
        emit(in_form(FORM_DO, Vec::new(), outer))?;
        let form = match self.forms.get(id) {
            Some(form) => form,
            None => {
                let read = match painted {
                    Painted::Found(id) => self.backend.form_xobject(id),
                    Painted::Read(read) => Some(read),
                };
                let Some(read) = read else {
                    return ControlFlow::Continue(());
                };
                self.forms.decode(self.backend, self.page, &read)
            }
        };
        if form.ops.is_none() && self.forms.newly_undecodable(id) {
            self.undecodable_streams += 1;
        }
        self.paint_form(&form, outer, emit)
    }

    fn paint_form(
        &mut self,
        form: &Form,
        outer: Option<ObjectId>,
        emit: &mut Emit<'_>,
    ) -> ControlFlow<()> {
        self.forms_painted += 1;
        let Some(ops) = &form.ops else {
            return ControlFlow::Continue(());
        };

        emit(in_form("q", Vec::new(), outer))?;
        if form.matrix != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            emit(in_form(
                "cm",
                form.matrix.map(PdfValue::Real).to_vec(),
                outer,
            ))?;
        }
        if let Some(b) = form.bbox {
            let rect = [b.llx, b.lly, b.width(), b.height()]
                .map(PdfValue::Real)
                .to_vec();
            for (operator, operands) in [("re", rect), ("W", Vec::new()), ("n", Vec::new())] {
                emit(in_form(operator, operands, outer))?;
            }
        }

        if let Some(group) = form.group {
            let flags = [(group.isolated, "I"), (group.knockout, "K")]
                .into_iter()
                .filter(|(set, _)| *set)
                .map(|(_, key)| PdfValue::Name(key.as_bytes().to_vec()))
                .collect();
            emit(in_form(BEGIN_GROUP, flags, outer))?;
        }
        self.path.push(form.id);
        let painted = self.paint(ops, Some(form.id), emit);
        self.path.pop();
        painted?;
        if form.group.is_some() {
            emit(in_form(END_GROUP, Vec::new(), outer))?;
        }

        emit(in_form("Q", Vec::new(), outer))
    }
}

/// An operator placing or bracketing a form, in the scope of the content that paints it.
fn in_form(operator: &str, operands: Vec<PdfValue>, outer: Option<ObjectId>) -> ContentOp {
    let mut op = ContentOp::new(operator, operands);
    op.form = outer;
    op
}
