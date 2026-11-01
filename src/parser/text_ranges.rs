use std::ops::Range;

/// Ownership of an exact UTF-8 byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRange<T> {
    pub range: Range<usize>,
    pub value: T,
}

pub(crate) fn push_range<T: PartialEq>(
    ranges: &mut Vec<TextRange<T>>,
    range: Range<usize>,
    value: T,
) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = ranges.last_mut() {
        if last.range.end == range.start && last.value == value {
            last.range.end = range.end;
            return;
        }
    }
    ranges.push(TextRange { range, value });
}

pub(crate) fn copy_ranges<T: Clone + PartialEq>(
    destination: &mut Vec<TextRange<T>>,
    source: &[TextRange<T>],
    range: Range<usize>,
    at: usize,
) {
    let first = source.partition_point(|r| r.range.end <= range.start);
    for r in source[first..]
        .iter()
        .take_while(|r| r.range.start < range.end)
    {
        let start = r.range.start.max(range.start) - range.start + at;
        let end = r.range.end.min(range.end) - range.start + at;
        push_range(destination, start..end, r.value.clone());
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TaggedText<T> {
    pub text: String,
    pub ranges: Vec<TextRange<T>>,
}

impl<T: Clone + PartialEq> TaggedText<T> {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            ranges: Vec::new(),
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.text.push_str(text);
    }

    pub fn copy(&mut self, text: &str, owners: &[TextRange<T>], range: Range<usize>) {
        let at = self.text.len();
        self.text.push_str(&text[range.clone()]);
        copy_ranges(&mut self.ranges, owners, range, at);
    }

    pub fn emit_from(&mut self, owners: &[TextRange<T>], source: usize, text: &str) {
        let start = self.text.len();
        self.text.push_str(text);
        let i = owners.partition_point(|r| r.range.end <= source);
        if let Some(owner) = owners.get(i).filter(|r| r.range.contains(&source)) {
            push_range(
                &mut self.ranges,
                start..self.text.len(),
                owner.value.clone(),
            );
        }
    }

    pub fn slice(&self, range: Range<usize>) -> Self {
        let mut result = Self::new();
        result.copy(&self.text, &self.ranges, range);
        result
    }
}
