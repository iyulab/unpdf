use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::backend::{PageBox, PageId};
use super::raw::tokenizer::{dict_get, PdfDict, PdfObject};
use super::raw::RawDocument;

const MAX_ANNOTATIONS: usize = 4096;
const MAX_AREAS: usize = 4096;
const MAX_URI_BYTES: usize = 8192;
const MAX_TARGET_BYTES: usize = 1024 * 1024;
const MAX_URI_WORK: usize = 4 * 1024 * 1024;
const GRID: usize = 16;
const MAX_INDEX_REFERENCES: usize = 65_536;
const MAX_GEOMETRY_WORK: usize = 1_000_000;

/// One link's active quadrilateral, in default page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkArea {
    /// Convex vertices in perimeter order.
    pub points: [(f32, f32); 4],
}

impl LinkArea {
    fn from_points(mut points: [(f32, f32); 4]) -> Option<Self> {
        if points
            .iter()
            .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return None;
        }
        let center = points.iter().fold((0.0f64, 0.0f64), |(x, y), p| {
            (x + f64::from(p.0) / 4.0, y + f64::from(p.1) / 4.0)
        });
        points.sort_by(|a, b| {
            (f64::from(a.1) - center.1)
                .atan2(f64::from(a.0) - center.0)
                .total_cmp(&(f64::from(b.1) - center.1).atan2(f64::from(b.0) - center.0))
        });
        let area = Self { points };
        (0..4)
            .all(|i| cross(points[i], points[(i + 1) % 4], points[(i + 2) % 4]) > 0.0)
            .then_some(area)
    }

    pub(crate) fn contains(&self, point: (f32, f32)) -> bool {
        (0..4).all(|i| cross(self.points[i], self.points[(i + 1) % 4], point) >= -1e-4)
    }
}

fn cross(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f64 {
    (f64::from(b.0) - f64::from(a.0)) * (f64::from(p.1) - f64::from(a.1))
        - (f64::from(b.1) - f64::from(a.1)) * (f64::from(p.0) - f64::from(a.0))
}

/// A URI action and the areas activating it. Later annotations win overlaps.
#[derive(Debug, Clone)]
pub struct LinkAnnotation {
    pub target: Arc<str>,
    pub areas: Vec<LinkArea>,
}

/// Supported URI annotations and whether a resource bound prevented reading them all.
#[derive(Debug, Clone, Default)]
pub struct LinkAnnotations {
    pub annotations: Vec<LinkAnnotation>,
    pub incomplete: bool,
}

#[derive(Debug)]
struct LinkBudget {
    output: AtomicUsize,
    incomplete: AtomicBool,
}

/// A shared link target. Output copies share a page's one-megabyte budget.
#[derive(Debug)]
pub struct LinkTarget {
    uri: Arc<str>,
    budget: Arc<LinkBudget>,
}

impl PartialEq for LinkTarget {
    fn eq(&self, other: &Self) -> bool {
        self.uri == other.uri
    }
}

impl Eq for LinkTarget {}

impl LinkTarget {
    pub(crate) fn mark_incomplete(&self) {
        self.budget.incomplete.store(true, Ordering::Relaxed);
    }
    /// The annotation's target, with a catalog base URI resolved when available.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub(crate) fn copy_uri(&self) -> Option<String> {
        let charge = self.uri.len().saturating_add(64);
        let mut left = self.budget.output.load(Ordering::Relaxed);
        loop {
            let Some(next) = left.checked_sub(charge) else {
                self.budget.output.store(0, Ordering::Relaxed);
                self.budget.incomplete.store(true, Ordering::Relaxed);
                return None;
            };
            match self.budget.output.compare_exchange_weak(
                left,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(self.uri.to_string()),
                Err(current) => left = current,
            }
        }
    }
}

/// A link target and the source styling of its label before layout joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpanStyle {
    pub target: Arc<LinkTarget>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
}

impl LinkSpanStyle {
    pub(crate) fn new(target: Arc<LinkTarget>) -> Self {
        Self {
            target,
            bold: false,
            italic: false,
            strikethrough: false,
        }
    }
}

/// Link ownership of an exact UTF-8 byte range of a text span.
pub type TextLink = super::text_ranges::TextRange<LinkSpanStyle>;

pub(crate) struct PageLinks {
    pub page: PageId,
    bounds: PageBox,
    areas: Vec<(Arc<LinkTarget>, LinkArea)>,
    cells: Vec<Vec<usize>>,
    work: Cell<usize>,
    budget: Option<Arc<LinkBudget>>,
    clear_all: bool,
}

impl PageLinks {
    pub fn new(page: PageId, annotations: LinkAnnotations, bounds: PageBox) -> Self {
        let mut result = Self {
            page,
            bounds,
            areas: Vec::new(),
            cells: Vec::new(),
            work: Cell::new(MAX_GEOMETRY_WORK),
            budget: None,
            clear_all: annotations.incomplete,
        };
        if result.clear_all || annotations.annotations.is_empty() {
            return result;
        }
        if ![
            bounds.llx,
            bounds.lly,
            bounds.urx,
            bounds.ury,
            bounds.width(),
            bounds.height(),
        ]
        .into_iter()
        .all(f32::is_finite)
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
        {
            result.clear_all = true;
            return result;
        }
        let budget = Arc::new(LinkBudget {
            output: AtomicUsize::new(MAX_TARGET_BYTES),
            incomplete: AtomicBool::new(false),
        });
        result.budget = Some(budget.clone());
        result.cells = (0..GRID * GRID).map(|_| Vec::new()).collect();
        let mut references = MAX_INDEX_REFERENCES;
        for annotation in annotations.annotations {
            let target = Arc::new(LinkTarget {
                uri: annotation.target,
                budget: budget.clone(),
            });
            for area in annotation.areas {
                let xs = area.points.map(|p| p.0);
                let ys = area.points.map(|p| p.1);
                let left = xs.into_iter().fold(f32::INFINITY, f32::min).max(bounds.llx);
                let right = xs
                    .into_iter()
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(bounds.urx);
                let bottom = ys.into_iter().fold(f32::INFINITY, f32::min).max(bounds.lly);
                let top = ys
                    .into_iter()
                    .fold(f32::NEG_INFINITY, f32::max)
                    .min(bounds.ury);
                if left > right || bottom > top {
                    continue;
                }
                let (x0, y0) = result.cell((left, bottom));
                let (x1, y1) = result.cell((right, top));
                let needed = (x1 - x0 + 1) * (y1 - y0 + 1);
                let Some(left) = references.checked_sub(needed) else {
                    result.clear_all = true;
                    result.areas.clear();
                    result.cells.clear();
                    return result;
                };
                references = left;
                let id = result.areas.len();
                result.areas.push((target.clone(), area));
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        result.cells[y * GRID + x].push(id);
                    }
                }
            }
        }
        result
    }

    fn cell(&self, point: (f32, f32)) -> (usize, usize) {
        let x = ((point.0 - self.bounds.llx) / self.bounds.width() * GRID as f32)
            .clamp(0.0, (GRID - 1) as f32) as usize;
        let y = ((point.1 - self.bounds.lly) / self.bounds.height() * GRID as f32)
            .clamp(0.0, (GRID - 1) as f32) as usize;
        (x, y)
    }

    pub fn incomplete(&self) -> bool {
        self.clear_all
            || self
                .budget
                .as_ref()
                .is_some_and(|b| b.incomplete.load(Ordering::Relaxed))
    }

    pub fn mark_incomplete(&self) {
        if let Some(budget) = &self.budget {
            budget.incomplete.store(true, Ordering::Relaxed);
        }
    }

    pub fn has_links(&self) -> bool {
        !self.areas.is_empty() && !self.clear_all
    }

    pub fn clear_all_links(&self) -> bool {
        self.clear_all
    }

    pub fn owner(&self, point: (f32, f32)) -> Option<Arc<LinkTarget>> {
        if !self.has_links()
            || !point.0.is_finite()
            || !point.1.is_finite()
            || !(self.bounds.llx..=self.bounds.urx).contains(&point.0)
            || !(self.bounds.lly..=self.bounds.ury).contains(&point.1)
        {
            return None;
        }
        let Some(left) = self.work.get().checked_sub(1) else {
            self.mark_incomplete();
            return None;
        };
        self.work.set(left);
        let (x, y) = self.cell(point);
        let candidates = &self.cells[y * GRID + x];
        for &id in candidates.iter().rev() {
            let Some(left) = self.work.get().checked_sub(1) else {
                self.mark_incomplete();
                return None;
            };
            self.work.set(left);
            let (target, area) = &self.areas[id];
            if area.contains(point) {
                return Some(target.clone());
            }
        }
        None
    }
}

pub(crate) fn read_annotations(doc: &RawDocument, page: PageId) -> LinkAnnotations {
    let Ok(page) = doc.get_dict(page) else {
        return LinkAnnotations::default();
    };
    let Some(annotations) = dict_get(page, b"Annots").and_then(|v| doc.resolve(v).as_array())
    else {
        return LinkAnnotations::default();
    };
    if annotations.len() > MAX_ANNOTATIONS {
        return LinkAnnotations {
            annotations: Vec::new(),
            incomplete: true,
        };
    }
    let base = doc
        .catalog()
        .ok()
        .and_then(|c| dict_get(c, b"URI"))
        .and_then(|v| doc.resolve(v).as_dict())
        .and_then(|d| string(doc, d, b"Base"))
        .and_then(|s| url::Url::parse(s).ok());
    let mut result = Vec::new();
    let mut area_budget = MAX_AREAS;
    let mut target_budget = MAX_TARGET_BYTES;
    let mut uri_work = MAX_URI_WORK;
    let mut targets: HashSet<Arc<str>> = HashSet::new();
    for value in annotations.iter().take(MAX_ANNOTATIONS) {
        if area_budget == 0 {
            return LinkAnnotations {
                annotations: Vec::new(),
                incomplete: true,
            };
        }
        let Some(annotation) = doc.resolve(value).as_dict() else {
            continue;
        };
        if dict_get(annotation, b"Subtype").and_then(|v| doc.resolve(v).as_name()) != Some(b"Link")
        {
            continue;
        }
        if dict_get(annotation, b"F")
            .and_then(|v| doc.resolve(v).as_i64())
            .unwrap_or(0)
            & (2 | 32)
            != 0
        {
            continue;
        }
        let Some(action) = dict_get(annotation, b"A").and_then(|v| doc.resolve(v).as_dict()) else {
            continue;
        };
        if dict_get(action, b"S").and_then(|v| doc.resolve(v).as_name()) != Some(b"URI") {
            continue;
        }
        if dict_get(action, b"URI").is_some_and(
            |v| matches!(doc.resolve(v), PdfObject::Str(bytes) if bytes.len() > MAX_URI_BYTES),
        ) {
            return LinkAnnotations {
                annotations: Vec::new(),
                incomplete: true,
            };
        }
        if let Some(PdfObject::Str(bytes)) = dict_get(action, b"URI").map(|v| doc.resolve(v)) {
            let Some(left) = uri_work.checked_sub(bytes.len()) else {
                return LinkAnnotations {
                    annotations: Vec::new(),
                    incomplete: true,
                };
            };
            uri_work = left;
        }
        let Some(original) = string(doc, action, b"URI") else {
            continue;
        };
        let resolved = if url::Url::parse(original).is_err() {
            base.as_ref()
                .and_then(|b| b.join(original).ok())
                .map(String::from)
        } else {
            None
        };
        let target = resolved.as_deref().unwrap_or(original);
        if resolved.is_some() {
            let Some(left) = uri_work.checked_sub(target.len()) else {
                return LinkAnnotations {
                    annotations: Vec::new(),
                    incomplete: true,
                };
            };
            uri_work = left;
        }
        if target.len() > MAX_URI_BYTES
            || (target.len() > target_budget && !targets.contains(target))
        {
            return LinkAnnotations {
                annotations: Vec::new(),
                incomplete: true,
            };
        }
        let Some(rect) = rectangle(doc, annotation) else {
            continue;
        };
        if dict_get(annotation, b"QuadPoints")
            .and_then(|v| doc.resolve(v).as_array())
            .is_some_and(|p| p.len() / 8 > area_budget)
        {
            return LinkAnnotations {
                annotations: Vec::new(),
                incomplete: true,
            };
        }
        let areas = quadrilaterals(doc, annotation, rect, area_budget).unwrap_or_else(|| {
            vec![LinkArea {
                points: [
                    (rect.llx, rect.lly),
                    (rect.urx, rect.lly),
                    (rect.urx, rect.ury),
                    (rect.llx, rect.ury),
                ],
            }]
        });
        area_budget -= areas.len();
        let target = if let Some(target) = targets.get(target) {
            target.clone()
        } else {
            target_budget -= target.len();
            let target: Arc<str> = target.into();
            targets.insert(target.clone());
            target
        };
        result.push(LinkAnnotation { target, areas });
    }
    LinkAnnotations {
        annotations: result,
        incomplete: false,
    }
}

fn string<'a>(doc: &'a RawDocument, dict: &'a PdfDict, key: &[u8]) -> Option<&'a str> {
    let PdfObject::Str(bytes) = doc.resolve(dict_get(dict, key)?) else {
        return None;
    };
    if bytes.is_empty() || bytes.len() > MAX_URI_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    (!text.chars().any(|c| c.is_control() || c.is_whitespace())).then_some(text)
}

fn rectangle(doc: &RawDocument, annotation: &PdfDict) -> Option<PageBox> {
    let values = doc.resolve(dict_get(annotation, b"Rect")?).as_array()?;
    if values.len() != 4 {
        return None;
    }
    let mut n = [0.0; 4];
    for (out, value) in n.iter_mut().zip(values) {
        *out = doc.resolve(value).as_f32()?;
        if !out.is_finite() {
            return None;
        }
    }
    let rect = PageBox {
        llx: n[0].min(n[2]),
        lly: n[1].min(n[3]),
        urx: n[0].max(n[2]),
        ury: n[1].max(n[3]),
    };
    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

fn quadrilaterals(
    doc: &RawDocument,
    annotation: &PdfDict,
    rect: PageBox,
    budget: usize,
) -> Option<Vec<LinkArea>> {
    let points = doc
        .resolve(dict_get(annotation, b"QuadPoints")?)
        .as_array()?;
    if points.is_empty() || points.len() % 8 != 0 || points.len() / 8 > budget {
        return None;
    }
    let mut areas = Vec::with_capacity(points.len() / 8);
    for quad in points.as_chunks::<8>().0 {
        let mut p = [(0.0, 0.0); 4];
        for (point, pair) in p.iter_mut().zip(quad.as_chunks::<2>().0) {
            *point = (
                doc.resolve(&pair[0]).as_f32()?,
                doc.resolve(&pair[1]).as_f32()?,
            );
            if !(rect.llx..=rect.urx).contains(&point.0)
                || !(rect.lly..=rect.ury).contains(&point.1)
            {
                return None;
            }
        }
        areas.push(LinkArea::from_points(p)?);
    }
    Some(areas)
}
