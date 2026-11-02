//! The normal appearances of a page's annotations (ISO 32000-2 §12.5.5).

use std::collections::HashMap;

use super::backend::{
    form_matrix, rectangle_from_array, AnnotationAppearance, AnnotationAppearances, BlendMode,
    PageBox, PageId,
};
use super::link::MAX_ANNOTATIONS;
use super::raw::tokenizer::{dict_get, PdfDict};
use super::raw::RawDocument;

const INVISIBLE: i64 = 1;
const HIDDEN: i64 = 1 << 1;
const NO_VIEW: i64 = 1 << 5;

const STANDARD_SUBTYPES: [&[u8]; 28] = [
    b"Text",
    b"Link",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Caret",
    b"Stamp",
    b"Ink",
    b"Popup",
    b"FileAttachment",
    b"Sound",
    b"Movie",
    b"Screen",
    b"Widget",
    b"PrinterMark",
    b"TrapNet",
    b"Watermark",
    b"3D",
    b"Redact",
    b"Projection",
    b"RichMedia",
];

#[derive(Clone, Copy)]
struct Unpainted;

pub(crate) fn read_appearances(doc: &RawDocument, page: PageId) -> AnnotationAppearances {
    let mut read = AnnotationAppearances::default();
    let Some(annotations) = doc
        .get_dict(page)
        .ok()
        .and_then(|page| dict_get(page, b"Annots"))
        .and_then(|annotations| doc.resolve(annotations).as_array())
    else {
        return read;
    };
    let mut examined = HashMap::new();
    for value in annotations.iter().take(MAX_ANNOTATIONS) {
        let examine = || match doc.resolve(value).as_dict() {
            Some(annotation) => appearance(doc, annotation),
            None => Ok(None),
        };
        let outcome = match value.as_reference() {
            Some(id) => examined.entry(id).or_insert_with(examine).clone(),
            None => examine(),
        };
        match outcome {
            Ok(Some(appearance)) => read.appearances.push(appearance),
            Ok(None) => {}
            Err(Unpainted) => read.unpainted += 1,
        }
    }
    read.unpainted += annotations.len().saturating_sub(MAX_ANNOTATIONS) as u32;
    read
}

fn appearance(
    doc: &RawDocument,
    annotation: &PdfDict,
) -> Result<Option<AnnotationAppearance>, Unpainted> {
    let flags = dict_get(annotation, b"F")
        .and_then(|flags| doc.resolve(flags).as_i64())
        .unwrap_or(0);
    let subtype =
        dict_get(annotation, b"Subtype").and_then(|subtype| doc.resolve(subtype).as_name());
    let standard = subtype.is_some_and(|subtype| STANDARD_SUBTYPES.contains(&subtype));
    if flags & (HIDDEN | NO_VIEW) != 0
        || (flags & INVISIBLE != 0 && !standard)
        || subtype == Some(b"Popup".as_slice())
    {
        return Ok(None);
    }
    let Some(normal) = dict_get(annotation, b"AP")
        .and_then(|ap| doc.resolve(ap).as_dict())
        .and_then(|ap| dict_get(ap, b"N"))
    else {
        return Ok(None);
    };
    let entry = match doc.resolve(normal).as_dict() {
        Some(states) => {
            let state = dict_get(annotation, b"AS")
                .and_then(|state| doc.resolve(state).as_name())
                .ok_or(Unpainted)?;
            let Some(entry) = dict_get(states, state) else {
                return Ok(None);
            };
            entry
        }
        None => normal,
    };
    let id = entry.as_reference().ok_or(Unpainted)?;
    let stream = doc.resolve(entry).as_stream().ok_or(Unpainted)?;
    let rect = dict_get(annotation, b"Rect")
        .and_then(|rect| rectangle_from_array(doc, rect))
        .ok_or(Unpainted)?;
    let bbox = dict_get(&stream.dict, b"BBox")
        .and_then(|bbox| rectangle_from_array(doc, bbox))
        .ok_or(Unpainted)?;
    let transformed = transformed_box(form_matrix(doc, stream), bbox);
    if [rect, bbox, transformed]
        .iter()
        .any(|b| b.width() <= 0.0 || b.height() <= 0.0)
    {
        return Ok(None);
    }
    if let Some(blend) = dict_get(annotation, b"BM").map(|blend| doc.resolve(blend)) {
        let name = blend.as_name().ok_or(Unpainted)?;
        if BlendMode::from_name(name).is_some_and(|blend| blend != BlendMode::Normal) {
            return Err(Unpainted);
        }
    }
    let (sx, sy) = (
        rect.width() / transformed.width(),
        rect.height() / transformed.height(),
    );
    let placement = [
        sx,
        0.0,
        0.0,
        sy,
        rect.llx - transformed.llx * sx,
        rect.lly - transformed.lly * sy,
    ];
    if !placement.iter().all(|v| v.is_finite()) {
        return Err(Unpainted);
    }
    Ok(Some(AnnotationAppearance {
        form: id,
        placement,
    }))
}

fn transformed_box(matrix: [f32; 6], bbox: PageBox) -> PageBox {
    let [a, b, c, d, e, f] = matrix;
    let corners = [
        (bbox.llx, bbox.lly),
        (bbox.urx, bbox.lly),
        (bbox.urx, bbox.ury),
        (bbox.llx, bbox.ury),
    ]
    .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let xs = corners.map(|p| p.0);
    let ys = corners.map(|p| p.1);
    let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
    let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
    PageBox {
        llx: min(xs),
        lly: min(ys),
        urx: max(xs),
        ury: max(ys),
    }
}
