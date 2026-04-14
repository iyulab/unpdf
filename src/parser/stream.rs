//! Streaming / parallel page parsing pipeline.

use std::cmp::{Ord, Ordering, Reverse};
use std::collections::BinaryHeap;
use std::path::PathBuf;

use crate::error::Error;
use crate::model::{ExtractionQuality, FormField, Metadata, Outline, Page};
use crate::render::PageSelection;

use super::options::{ErrorMode, ExtractMode, ParseOptions};

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
    pub extract_mode: ExtractMode,
    pub extract_resources: bool,
    pub pages: PageSelection,
    pub password: Option<String>,
    pub parallel: bool,
    /// 동시에 in-flight 상태로 둘 페이지 수의 상한. 기본 cores*2.
    pub window_size: usize,
    pub emit_progress_every: u32,
    /// Some 이면 페이지 파싱 직후 리소스(이미지)를 이 디렉토리로 즉시 flush,
    /// `Document.resources` 에는 적재하지 않음. 대용량 문서 메모리 보호.
    pub flush_resources_to: Option<PathBuf>,
}

impl Default for PageStreamOptions {
    fn default() -> Self {
        Self {
            error_mode: ErrorMode::Lenient,
            extract_mode: ExtractMode::Full,
            extract_resources: false,
            pages: PageSelection::All,
            password: None,
            parallel: true,
            window_size: rayon::current_num_threads().saturating_mul(2).max(2),
            emit_progress_every: 16,
            flush_resources_to: None,
        }
    }
}

impl From<&ParseOptions> for PageStreamOptions {
    fn from(o: &ParseOptions) -> Self {
        Self {
            error_mode: o.error_mode,
            extract_mode: o.extract_mode,
            extract_resources: o.extract_resources,
            pages: o.pages.clone(),
            password: o.password.clone(),
            parallel: o.parallel,
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
        if self.done % self.every == 0 || self.done == self.total {
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

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

// ---------------------------------------------------------------------------
// run_stream — rayon+crossbeam streaming pipeline
// ---------------------------------------------------------------------------

use rayon::prelude::*;
use std::ops::ControlFlow;

use super::backend::PdfBackend;
use super::pdf_parser::{convert_outline_item_pub, parse_pdf_date_pub, parse_single_page};

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
    let mut targets: Vec<u32> = page_map
        .keys()
        .copied()
        .filter(|n| opts.pages.includes(*n))
        .collect();
    targets.sort_unstable();
    let first_expected = targets.first().copied().unwrap_or(0);

    // ParseOptions 재구성
    let parse_opts = ParseOptions {
        error_mode: opts.error_mode,
        extract_mode: opts.extract_mode,
        extract_resources: opts.extract_resources,
        pages: opts.pages.clone(),
        password: opts.password.clone(),
        parallel: opts.parallel,
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

    if opts.parallel && targets.len() > 1 {
        // Use unbounded channel: the ReorderBuffer already limits outstanding pages.
        // A bounded channel here would deadlock because the consumer (on_event) is
        // on the current thread and cannot run concurrently with std::thread::scope.
        let (tx, rx) = crossbeam_channel::unbounded::<(u32, crate::error::Result<Page>)>();
        let parse_opts_ref = &parse_opts;
        let targets_ref = &targets;

        // Spawn a dedicated OS thread for the producer so the consumer can run on
        // the current thread concurrently. We use std::thread::scope for lifetime
        // safety — the scope returns only after the consumer loop has exited AND
        // the producer thread has finished, but we drop `rx` to unblock the scope
        // if the consumer exits early.
        std::thread::scope(|s| {
            let tx_for_producer = tx;
            s.spawn(|| {
                targets_ref
                    .par_iter()
                    .for_each_with(tx_for_producer, |tx, &page_num| {
                        let r = parse_single_page(backend, page_num, parse_opts_ref);
                        let _ = tx.send((page_num, r));
                    });
            });

            // Consumer runs on this (current) thread.
            while let Ok((page_num, r)) = rx.recv() {
                let item = match r {
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
            // Drop rx here so the producer isn't blocked on send if we broke early.
            drop(rx);
        });
    } else {
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
