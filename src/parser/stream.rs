//! Streaming / parallel page parsing pipeline.

use std::cmp::{Ord, Ordering, Reverse};
use std::collections::BinaryHeap;
use std::path::PathBuf;

use crate::error::Error;
use crate::model::{ExtractionQuality, FormField, Metadata, Outline, Page};
use crate::render::PageSelection;

use super::options::{ErrorMode, ExtractMode, ParseOptions};

/// 페이지 단위 스트리밍 파싱 이벤트.
#[derive(Debug)]
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
