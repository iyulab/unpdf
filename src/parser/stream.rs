//! Streaming / parallel page parsing pipeline.

use std::cmp::{Ord, Ordering, Reverse};
use std::collections::BinaryHeap;

use crate::error::Error;
use crate::model::{ExtractionQuality, FormField, Metadata, Outline, Page};
use crate::render::PageSelection;

use super::options::{ErrorMode, ParseOptions};

/// 페이지 단위 스트리밍 파싱 이벤트.
///
/// `DocumentStart` 는 document-wide metadata + outline + form_fields 를
/// 번들로 싣고 있어 다른 variant 보다 구조체가 크다. 이는 이벤트 스트림
/// 시작 시 **한 번만** 발생하므로 평균 메모리 영향은 무시 가능.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ParseEvent {
    DocumentStart {
        metadata: Metadata,
        page_count: u32,
        outline: Option<Outline>,
        form_fields: Vec<FormField>,
    },
    /// page_num ASC 순서로만 방출된다.
    PageParsed(Page),
    /// Lenient 모드에서 실패한 페이지 통지.
    PageFailed {
        page: u32,
        error: Error,
    },
    /// 주기적 진척도 (기본 16페이지마다).
    Progress {
        done: u32,
        total: u32,
    },
    DocumentEnd {
        quality: ExtractionQuality,
    },
}

/// 스트리밍 파싱 옵션.
#[derive(Debug, Clone)]
pub struct PageStreamOptions {
    pub error_mode: ErrorMode,
    pub extract_text: bool,
    pub extract_resources: bool,
    /// 장식용 작은 이미지를 걸러내는 픽셀 임계값. `ParseOptions` 참고.
    pub min_image_dimension: u32,
    pub pages: PageSelection,
    pub password: Option<String>,
    pub parallel: bool,
    /// 읽을 수 없는 OCR 텍스트 레이어를 버릴지 여부. `ParseOptions` 참고.
    pub suppress_low_confidence_ocr: bool,
    /// Whether to omit a page's running heads and feet. See `ParseOptions::omit_running_heads`.
    pub omit_running_heads: bool,
    /// How many pages may be parsing, or parsed and waiting for an earlier page, at once in a
    /// parallel run -- what bounds memory on a long document whose early page is slow. At least
    /// 1; default twice the worker threads.
    pub window_size: usize,
    pub emit_progress_every: u32,
}

impl Default for PageStreamOptions {
    fn default() -> Self {
        Self {
            error_mode: ErrorMode::Lenient,
            extract_text: true,
            extract_resources: false,
            min_image_dimension: 64,
            pages: PageSelection::All,
            password: None,
            parallel: true,
            suppress_low_confidence_ocr: true,
            omit_running_heads: false,
            #[cfg(not(target_arch = "wasm32"))]
            window_size: rayon::current_num_threads().saturating_mul(2).max(2),
            #[cfg(target_arch = "wasm32")]
            window_size: 2,
            emit_progress_every: 16,
        }
    }
}

impl From<&ParseOptions> for PageStreamOptions {
    fn from(o: &ParseOptions) -> Self {
        Self {
            error_mode: o.error_mode,
            extract_text: o.extract_text,
            extract_resources: o.effective_extract_resources(),
            min_image_dimension: o.min_image_dimension,
            pages: o.pages.clone(),
            password: o.password.clone(),
            parallel: o.parallel,
            suppress_low_confidence_ocr: o.suppress_low_confidence_ocr,
            omit_running_heads: o.omit_running_heads,
            ..Self::default()
        }
    }
}

/// 진척도 카운터 — consumer 스레드가 직접 inc 하도록 노출.
pub(crate) struct ProgressCounter {
    pub done: u32,
    pub total: u32,
    pub every: u32,
}

impl ProgressCounter {
    pub fn new(total: u32, every: u32) -> Self {
        Self {
            done: 0,
            total,
            every: every.max(1),
        }
    }

    pub fn tick(&mut self) -> Option<(u32, u32)> {
        self.done += 1;
        if self.done.is_multiple_of(self.every) || self.done == self.total {
            Some((self.done, self.total))
        } else {
            None
        }
    }
}

/// 페이지 번호만으로 비교되는 heap entry. T 는 임의 타입 허용.
struct ByPage<T>(u32, T);

impl<T> PartialEq for ByPage<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> Eq for ByPage<T> {}

impl<T> PartialOrd for ByPage<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for ByPage<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

/// page_num 을 키로 임의 순서 입력을 받아 ASC 로 flush 하는 버퍼.
pub(crate) struct ReorderBuffer<T> {
    heap: BinaryHeap<Reverse<ByPage<T>>>,
    next_expected: u32,
}

impl<T> ReorderBuffer<T> {
    pub fn new(start_from: u32) -> Self {
        Self {
            heap: BinaryHeap::new(),
            next_expected: start_from,
        }
    }

    pub fn push(&mut self, page_num: u32, value: T) {
        self.heap.push(Reverse(ByPage(page_num, value)));
    }

    /// 다음 기대 page_num 이 top 에 있으면 pop 하고 반환. 그렇지 않으면 None.
    pub fn try_pop_next(&mut self) -> Option<(u32, T)> {
        match self.heap.peek() {
            Some(Reverse(ByPage(n, _))) if *n == self.next_expected => {
                let Reverse(ByPage(n, v)) = self.heap.pop().unwrap();
                self.next_expected += 1;
                Some((n, v))
            }
            _ => None,
        }
    }

    /// missing page 가 있을 때, 현재 가장 작은 page_num 으로 forward 한다.
    pub fn skip_to_next_present(&mut self) {
        if let Some(Reverse(ByPage(n, _))) = self.heap.peek() {
            self.next_expected = *n;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// How many pages are waiting — what the windowed parallel parse measures its window by.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

// ---------------------------------------------------------------------------
// run_stream — rayon+crossbeam streaming pipeline
// ---------------------------------------------------------------------------

use std::ops::ControlFlow;

use super::backend::PdfBackend;
use super::pdf_parser::{convert_outline_item_pub, parse_pdf_date_pub, parse_single_page};

/// Run `parse` over `pages` on the rayon pool, at most `window` pages at a time, and hand each
/// result to `receive` on this thread as it arrives.
///
/// `receive` answers with how many pages have left the window since its last call -- passed on,
/// in page order, to whoever consumes them -- or `Break` to stop. A page enters the window when it
/// is handed to the pool and leaves when it is passed on, so the pages being parsed plus those
/// parsed and waiting for an earlier one never exceed `window` (at least 1).
///
/// Pages are handed out in order, each once a place in the window is free. So the next page the
/// consumer needs is always one already handed out, and a full window always drains: there is no
/// order of completion that deadlocks. The dispatcher waits for places on this thread's side, not
/// on a pool worker, so a pool of one thread runs too.
///
/// A panic in `parse` is carried to this thread and resumed here, after the dispatcher is told to
/// stop -- the page it would have produced is not waited for.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parse_windowed<R: Send>(
    pages: &[u32],
    window: usize,
    parse: impl Fn(u32) -> R + Sync,
    mut receive: impl FnMut(u32, R) -> ControlFlow<(), usize>,
) {
    let window = window.max(1);
    let (place_tx, place_rx) = crossbeam_channel::bounded::<()>(window);
    for _ in 0..window {
        let _ = place_tx.send(());
    }
    let (tx, rx) = crossbeam_channel::unbounded::<(u32, std::thread::Result<R>)>();
    let parse = &parse;

    std::thread::scope(|s| {
        s.spawn(move || {
            rayon::in_place_scope(|pool| {
                for &page in pages {
                    if place_rx.recv().is_err() {
                        break; // the consumer stopped
                    }
                    let tx = tx.clone();
                    pool.spawn(move |_| {
                        let r =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(page)));
                        let _ = tx.send((page, r));
                    });
                }
            });
        });

        // Owned here, so that leaving this closure -- by finishing, stopping or unwinding --
        // tells the dispatcher there will be no more places.
        let place_tx = place_tx;
        while let Ok((page, r)) = rx.recv() {
            let r = match r {
                Ok(r) => r,
                Err(payload) => std::panic::resume_unwind(payload),
            };
            match receive(page, r) {
                ControlFlow::Continue(left) => {
                    for _ in 0..left {
                        let _ = place_tx.send(());
                    }
                }
                ControlFlow::Break(()) => break,
            }
        }
        drop(place_tx);
        drop(rx);
    });
}

/// Sample the first `max_pages` pages of content stream operators to detect
/// scan-only PDFs. Returns `true` when image (`Do`, `BI`) operators are present but
/// no text-show operators (`Tj`, `TJ`, `'`, `"`) are found, which is the
/// signature of a PDF composed entirely of raster images without a text layer.
fn detect_scan_pdf(
    backend: &(dyn PdfBackend + Sync),
    page_map: &std::collections::BTreeMap<u32, super::backend::PageId>,
    targets: &[u32],
    max_pages: usize,
) -> bool {
    let sample = &targets[..targets.len().min(max_pages)];
    let mut image_ops: u32 = 0;

    for &page_num in sample {
        let Some(&page_id) = page_map.get(&page_num) else {
            continue;
        };
        let Ok(painted) = super::form_xobject::page_operations(backend, page_id) else {
            continue;
        };
        for op in &painted.ops {
            match op.operator.as_str() {
                "Tj" | "TJ" | "'" | "\"" => return false, // Text found — not a scan PDF
                // An XObject or an inline image (`BI`) — a scan stored either way.
                "Do" | "BI" => image_ops += 1,
                _ => {}
            }
        }
    }

    // Scan PDF: image operators present and no text operators encountered (early return on text)
    image_ops > 0
}

/// 페이지를 page_num ASC 순서로 스트리밍. 콜백이 `Break`를 반환하면 조기 종료.
/// 반환값은 누적된 `ExtractionQuality`.
pub(crate) fn run_stream<F>(
    backend: &(dyn PdfBackend + Sync),
    opts: &PageStreamOptions,
    mut on_event: F,
) -> crate::error::Result<ExtractionQuality>
where
    F: FnMut(ParseEvent) -> ControlFlow<()>,
{
    use crate::model::QualityAccumulator;

    // 1. Metadata / outline / form_fields 수집 후 DocumentStart emit
    let page_map = backend.pages();
    let total: u32 = page_map.len() as u32;
    let meta_raw = backend.metadata();
    let mut metadata = Metadata::with_version(meta_raw.version);
    metadata.title = meta_raw.title;
    metadata.author = meta_raw.author;
    metadata.subject = meta_raw.subject;
    metadata.keywords = meta_raw.keywords;
    metadata.creator = meta_raw.creator;
    metadata.producer = meta_raw.producer;
    metadata.encrypted = meta_raw.encrypted;
    metadata.page_count = total;
    if let Some(date_str) = meta_raw.creation_date {
        metadata.created = parse_pdf_date_pub(&date_str);
    }
    if let Some(date_str) = meta_raw.mod_date {
        metadata.modified = parse_pdf_date_pub(&date_str);
    }

    let outline = backend
        .outline()
        .ok()
        .filter(|o| !o.is_empty())
        .map(|raw_items| {
            let mut o = Outline::new();
            o.items = raw_items
                .into_iter()
                .map(convert_outline_item_pub)
                .collect();
            o
        });
    let form_fields = backend.acroform_fields();

    if let ControlFlow::Break(_) = on_event(ParseEvent::DocumentStart {
        metadata: metadata.clone(),
        page_count: total,
        outline: outline.clone(),
        form_fields: form_fields.clone(),
    }) {
        return Ok(ExtractionQuality::default());
    }

    // 2. 대상 page_num 수집 (PageSelection 필터)
    let numbers: Vec<u32> = page_map.keys().copied().collect();
    opts.pages.check(&numbers)?;
    let mut targets: Vec<u32> = page_map
        .keys()
        .copied()
        .filter(|n| opts.pages.includes(*n))
        .collect();
    targets.sort_unstable();
    let first_expected = targets.first().copied().unwrap_or(0);

    // 2a. Scan detection: sample first 5 pages before full parsing
    let is_scan_pdf = detect_scan_pdf(backend, &page_map, &targets, 5);

    // ParseOptions 재구성
    let parse_opts = ParseOptions {
        error_mode: opts.error_mode,
        extract_text: opts.extract_text,
        extract_resources: opts.extract_resources,
        min_image_dimension: opts.min_image_dimension,
        pages: opts.pages.clone(),
        password: opts.password.clone(),
        parallel: opts.parallel,
        suppress_low_confidence_ocr: opts.suppress_low_confidence_ocr,
        omit_running_heads: opts.omit_running_heads,
        // AI post-processing runs once over the assembled `Document` in
        // `PdfParser::parse` (see `ai_wiring`), not per-page here — `extract_resources`
        // above already carries whatever image-byte decoding it forced.
        #[cfg(feature = "ai")]
        ai: None,
    };

    // 3. 실행
    let mut quality = QualityAccumulator::new();
    let mut progress = ProgressCounter::new(targets.len() as u32, opts.emit_progress_every);
    let mut reorder: ReorderBuffer<crate::error::Result<Page>> = ReorderBuffer::new(first_expected);

    // flush_ready: 준비된 페이지를 consumer 에 전달한다.
    fn flush_ready<F2>(
        reorder: &mut ReorderBuffer<crate::error::Result<Page>>,
        quality: &mut QualityAccumulator,
        progress: &mut ProgressCounter,
        on_event: &mut F2,
    ) -> ControlFlow<()>
    where
        F2: FnMut(ParseEvent) -> ControlFlow<()>,
    {
        while let Some((n, item)) = reorder.try_pop_next() {
            match item {
                Ok(page) => {
                    if page.ocr_text_suppressed {
                        quality.note_suppressed_ocr_page();
                    }
                    quality.note_suppressed_text_runs(page.suppressed_text_runs);
                    quality.note_undecodable_content_streams(page.undecodable_content_streams);
                    quality.note_unsupported_images(page.unsupported_image_count);
                    for block in &page.elements {
                        let mut buf = String::new();
                        block.append_plain_text(&mut buf);
                        quality.accumulate(&buf);
                        quality.accumulate("\n");
                    }
                    if let ControlFlow::Break(_) = on_event(ParseEvent::PageParsed(page)) {
                        return ControlFlow::Break(());
                    }
                }
                Err(err) => {
                    if let ControlFlow::Break(_) = on_event(ParseEvent::PageFailed {
                        page: n,
                        error: err,
                    }) {
                        return ControlFlow::Break(());
                    }
                }
            }
            if let Some((done, tot)) = progress.tick() {
                if let ControlFlow::Break(_) = on_event(ParseEvent::Progress { done, total: tot }) {
                    return ControlFlow::Break(());
                }
            }
        }
        ControlFlow::Continue(())
    }

    let mut cancelled = false;
    let mut strict_err: Option<Error> = None;

    #[cfg(not(target_arch = "wasm32"))]
    let effective_parallel = opts.parallel && targets.len() > 1;
    #[cfg(target_arch = "wasm32")]
    let effective_parallel = false;

    #[cfg(not(target_arch = "wasm32"))]
    if effective_parallel {
        let parse_opts_ref = &parse_opts;
        parse_windowed(
            &targets,
            opts.window_size,
            |page_num| parse_single_page(backend, page_num, parse_opts_ref),
            |page_num, r| {
                let item = match r {
                    Ok(p) => Ok(p),
                    Err(e) => {
                        if opts.error_mode == ErrorMode::Strict {
                            strict_err = Some(e);
                            cancelled = true;
                            return ControlFlow::Break(());
                        }
                        Err(e)
                    }
                };
                reorder.push(page_num, item);
                let waiting = reorder.len();
                if let ControlFlow::Break(_) =
                    flush_ready(&mut reorder, &mut quality, &mut progress, &mut on_event)
                {
                    cancelled = true;
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(waiting - reorder.len())
            },
        );
    }

    if !effective_parallel {
        for &page_num in &targets {
            let item = match parse_single_page(backend, page_num, &parse_opts) {
                Ok(p) => Ok(p),
                Err(e) => {
                    if opts.error_mode == ErrorMode::Strict {
                        strict_err = Some(e);
                        cancelled = true;
                        break;
                    }
                    Err(e)
                }
            };
            reorder.push(page_num, item);
            if let ControlFlow::Break(_) =
                flush_ready(&mut reorder, &mut quality, &mut progress, &mut on_event)
            {
                cancelled = true;
                break;
            }
        }
    }

    if let Some(e) = strict_err {
        return Err(e);
    }

    if !cancelled {
        while !reorder.is_empty() {
            reorder.skip_to_next_present();
            if let ControlFlow::Break(_) =
                flush_ready(&mut reorder, &mut quality, &mut progress, &mut on_event)
            {
                break;
            }
        }
    }

    let mut final_q = quality.finalize();
    final_q.encrypted = metadata.encrypted;
    final_q.is_scan_pdf = is_scan_pdf;

    // Structural integrity is a document-level fact, so it is attached here rather than
    // accumulated per page: the pages this reports on are precisely the ones that never
    // arrived to be accumulated.
    let integrity = backend.integrity();
    final_q.pages_incomplete = integrity.pages_incomplete();
    final_q.declared_page_count = integrity.declared_page_count;
    final_q.unresolved_page_nodes = integrity.unresolved_page_nodes;
    final_q.skipped_object_count = integrity.skipped_object_count;
    final_q.xref_reconstructed = integrity.xref_reconstructed;

    let _ = on_event(ParseEvent::DocumentEnd {
        quality: final_q.clone(),
    });

    Ok(final_q)
}

#[cfg(test)]
mod reorder_tests {
    use super::*;

    #[test]
    fn flushes_in_ascending_order() {
        let mut b = ReorderBuffer::<&'static str>::new(1);
        b.push(3, "c");
        b.push(1, "a");
        b.push(2, "b");

        assert_eq!(b.try_pop_next(), Some((1, "a")));
        assert_eq!(b.try_pop_next(), Some((2, "b")));
        assert_eq!(b.try_pop_next(), Some((3, "c")));
        assert_eq!(b.try_pop_next(), None);
    }

    #[test]
    fn waits_for_missing_head() {
        let mut b = ReorderBuffer::<u32>::new(1);
        b.push(2, 22);
        b.push(3, 33);
        assert_eq!(b.try_pop_next(), None);
        b.push(1, 11);
        assert_eq!(b.try_pop_next(), Some((1, 11)));
        assert_eq!(b.try_pop_next(), Some((2, 22)));
        assert_eq!(b.try_pop_next(), Some((3, 33)));
    }

    #[test]
    fn skip_to_next_present_handles_gaps() {
        let mut b = ReorderBuffer::<&'static str>::new(1);
        b.push(3, "c");
        b.push(5, "e");
        assert_eq!(b.try_pop_next(), None);
        b.skip_to_next_present();
        assert_eq!(b.try_pop_next(), Some((3, "c")));
        assert_eq!(b.try_pop_next(), None);
        b.skip_to_next_present();
        assert_eq!(b.try_pop_next(), Some((5, "e")));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod window_tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as Atomic};
    use std::time::Duration;

    /// A consumer that passes pages on in order, as `run_stream`'s reorder buffer does, and
    /// counts what has left the window.
    struct InOrder {
        waiting: BTreeSet<u32>,
        next: u32,
        passed: Vec<u32>,
    }

    impl InOrder {
        fn new(first: u32) -> Self {
            Self {
                waiting: BTreeSet::new(),
                next: first,
                passed: Vec::new(),
            }
        }

        fn receive(&mut self, page: u32) -> usize {
            self.waiting.insert(page);
            let before = self.passed.len();
            while self.waiting.remove(&self.next) {
                self.passed.push(self.next);
                self.next += 1;
            }
            self.passed.len() - before
        }
    }

    #[test]
    fn the_window_holds_while_an_early_page_is_slow() {
        // Page 1 takes far longer than the rest, so every later page finishes first and waits
        // for it: without a bound they would all pile up behind it.
        let window = 4;
        let entered = AtomicUsize::new(0);
        let left = AtomicUsize::new(0);
        let overflow = AtomicBool::new(false);
        let mut consumer = InOrder::new(1);
        let pages: Vec<u32> = (1..=40).collect();
        parse_windowed(
            &pages,
            window,
            |page| {
                let inside = entered.fetch_add(1, Atomic::SeqCst) + 1 - left.load(Atomic::SeqCst);
                if inside > window {
                    overflow.store(true, Atomic::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(if page == 1 { 60 } else { 1 }));
                page
            },
            |_, page| {
                let n = consumer.receive(page);
                left.fetch_add(n, Atomic::SeqCst);
                ControlFlow::Continue(n)
            },
        );
        assert!(
            !overflow.load(Atomic::SeqCst),
            "more than {window} pages in flight"
        );
        assert_eq!(consumer.passed, pages, "every page, in order");
    }

    #[test]
    fn a_window_of_one_parses_a_page_at_a_time() {
        let entered = AtomicUsize::new(0);
        let left = AtomicUsize::new(0);
        let mut consumer = InOrder::new(1);
        let pages: Vec<u32> = (1..=12).collect();
        parse_windowed(
            &pages,
            0, // read as 1
            |page| {
                let inside = entered.fetch_add(1, Atomic::SeqCst) + 1 - left.load(Atomic::SeqCst);
                assert_eq!(inside, 1);
                page
            },
            |_, page| {
                let n = consumer.receive(page);
                left.fetch_add(n, Atomic::SeqCst);
                ControlFlow::Continue(n)
            },
        );
        assert_eq!(consumer.passed, pages);
    }

    #[test]
    fn stopping_parses_no_more_than_a_window_past_it() {
        let window = 3;
        let parsed = AtomicUsize::new(0);
        let mut consumer = InOrder::new(1);
        let pages: Vec<u32> = (1..=200).collect();
        parse_windowed(
            &pages,
            window,
            |page| {
                parsed.fetch_add(1, Atomic::SeqCst);
                page
            },
            |_, page| {
                let n = consumer.receive(page);
                if consumer.passed.len() >= 5 {
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(n)
            },
        );
        let parsed = parsed.load(Atomic::SeqCst);
        assert!(
            parsed <= 5 + 2 * window,
            "parsed {parsed} pages after stopping at 5"
        );
    }

    #[test]
    fn a_panicking_page_reaches_the_caller_instead_of_hanging_it() {
        let pages: Vec<u32> = (1..=30).collect();
        let result = std::panic::catch_unwind(|| {
            let mut consumer = InOrder::new(1);
            parse_windowed(
                &pages,
                4,
                |page| {
                    if page == 5 {
                        panic!("page 5");
                    }
                    page
                },
                |_, page| ControlFlow::Continue(consumer.receive(page)),
            );
        });
        assert!(result.is_err());
    }
}
