//! Streaming / parallel page parsing pipeline.

use std::cmp::{Ord, Ordering, Reverse};
use std::collections::BinaryHeap;

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
