//! Recursive XY-Cut algorithm for document layout segmentation.

/// A rectangular region with position and size.
#[derive(Debug, Clone, Copy)]
pub struct Block {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Block {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f32 {
        self.y - self.height
    }
}

/// Gap thresholds for [`xycut_segment`].
#[derive(Debug, Clone, Copy)]
pub struct XyCutConfig {
    /// A vertical whitespace channel at least this wide always splits a region.
    pub min_x_gap: f32,
    /// A horizontal whitespace band at least this tall always splits a region.
    pub min_y_gap: f32,
    /// A narrower vertical channel, down to this width, splits a region only as a
    /// column gutter: when the text on each side spans at least
    /// [`GUTTER_MIN_SIDE_SHARE`] of the region's width and holds at least
    /// [`GUTTER_MIN_SIDE_BLOCKS`] blocks. Gutters between text columns are a line
    /// height or two wide — far narrower than a safe unconditional cut — while the
    /// channel between a list's markers and its items is just as narrow but leaves
    /// a sliver on one side. Set it equal to `min_x_gap` to disable the rule.
    pub min_gutter: f32,
}

/// Minimum share of a region's width each side of a column gutter must span.
pub const GUTTER_MIN_SIDE_SHARE: f32 = 0.3;
/// Minimum number of blocks on each side of a column gutter.
pub const GUTTER_MIN_SIDE_BLOCKS: usize = 3;

/// Minimum share of a region's width the median line on each side of a gutter must span
/// for that side to read as a text column: a column's lines fill most of it, while the
/// fragments a table of contents (entries | page numbers), a chart's labels or a list's
/// markers leave beside a whitespace channel do not.
pub const COLUMN_LINE_MIN_SHARE: f32 = 0.25;

/// What recursive XY-cut made of a page's blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segmentation {
    /// Reading-order groups, each a list of indices into the segmented blocks. Every block
    /// is in exactly one group.
    pub groups: Vec<Vec<usize>>,
    /// Regions left whole — read line by line across — although a whitespace channel
    /// divides their text into two sides that each look like a column of text (at least
    /// [`GUTTER_MIN_SIDE_BLOCKS`] lines, median line at least [`COLUMN_LINE_MIN_SHARE`] of
    /// the region's width). The split rules declined them; the reading order there is a
    /// guess.
    pub ambiguous_regions: usize,
}

impl Segmentation {
    /// The most groups set side by side at any height: 1 for a single column, 2 for two
    /// columns, and so on. 0 when there are no groups.
    ///
    /// XY-cut groups do not overlap, so groups whose vertical extents (the span of their
    /// blocks' centre lines) overlap stand next to each other.
    pub fn column_count(&self, blocks: &[Block]) -> usize {
        let mut events: Vec<(f32, i32)> = Vec::with_capacity(self.groups.len() * 2);
        for group in &self.groups {
            let centers = group.iter().map(|&i| blocks[i].y - blocks[i].height / 2.0);
            let (lo, hi) = centers.fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(c), hi.max(c)));
            if lo <= hi {
                events.push((lo, 1));
                events.push((hi, -1));
            }
        }
        // At equal heights, open before close: groups touching at a line overlap there.
        events.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
        let (mut open, mut most) = (0i32, 0i32);
        for (_, delta) in events {
            open += delta;
            most = most.max(open);
        }
        most as usize
    }
}

/// Segment blocks into reading-order groups using recursive XY-cut.
///
/// The groups as copies of the blocks; [`xycut_partition`] gives them as indices, with
/// what the segmentation could not decide.
pub fn xycut_segment(blocks: &[Block], config: &XyCutConfig) -> Vec<Vec<Block>> {
    xycut_partition(blocks, config)
        .groups
        .into_iter()
        .map(|group| group.into_iter().map(|i| blocks[i]).collect())
        .collect()
}

/// Segment blocks into reading-order groups using recursive XY-cut, by index.
pub fn xycut_partition(blocks: &[Block], config: &XyCutConfig) -> Segmentation {
    let mut segmentation = Segmentation::default();
    let all: Vec<usize> = (0..blocks.len()).collect();
    match blocks.len() {
        0 => {}
        1 => segmentation.groups.push(all),
        _ => {
            partition_recursive(blocks, &all, config, &mut segmentation);
            if segmentation.groups.is_empty() {
                segmentation.groups.push(all);
            }
        }
    }
    segmentation
}

fn partition_recursive(
    all: &[Block],
    indices: &[usize],
    config: &XyCutConfig,
    out: &mut Segmentation,
) {
    if indices.is_empty() {
        return;
    }
    if indices.len() == 1 {
        out.groups.push(indices.to_vec());
        return;
    }
    let blocks: Vec<Block> = indices.iter().map(|&i| all[i]).collect();

    let min_x = blocks.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let max_x = blocks.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    let min_y = blocks.iter().map(|b| b.bottom()).fold(f32::MAX, f32::min);
    let max_y = blocks.iter().map(|b| b.y).fold(f32::MIN, f32::max);

    let v_gap = find_best_vertical_gap(&blocks, min_x, max_x, config);
    let h_gap = find_best_horizontal_gap(&blocks, min_y, max_y, config.min_y_gap);

    let split_v = |at: f32| split_indices(all, indices, |b| b.x + b.width / 2.0 < at);
    let split_h = |at: f32| split_indices(all, indices, |b| b.y - b.height / 2.0 > at);

    let parts: Vec<Vec<usize>> = match (v_gap, h_gap) {
        (Some((v_pos, v_width)), Some((_, h_height))) if v_width >= h_height => split_v(v_pos),
        (_, Some((h_pos, _))) => split_h(h_pos),
        (Some((v_pos, _)), None) => split_v(v_pos),
        (None, None) => match spanning_band(&blocks, min_x, max_x, config) {
            Some((band_top, band_bottom)) => {
                let center = |b: &Block| b.y - b.height / 2.0;
                let above = indices
                    .iter()
                    .copied()
                    .filter(|&i| center(&all[i]) > band_top);
                let below = indices
                    .iter()
                    .copied()
                    .filter(|&i| center(&all[i]) < band_bottom);
                let band = indices.iter().copied().filter(|&i| {
                    let c = center(&all[i]);
                    c <= band_top && c >= band_bottom
                });
                let parts: Vec<Vec<usize>> = [above.collect(), band.collect(), below.collect()]
                    .into_iter()
                    .filter(|p: &Vec<usize>| !p.is_empty())
                    .collect();
                if parts.len() > 1 {
                    parts
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        },
    };

    if parts.is_empty() {
        if reads_across_columns(&blocks, min_x, max_x, config) {
            out.ambiguous_regions += 1;
        }
        out.groups.push(indices.to_vec());
        return;
    }
    for part in parts {
        partition_recursive(all, &part, config, out);
    }
}

/// `indices` split into those whose block satisfies `first` and the rest, in that order.
fn split_indices(
    all: &[Block],
    indices: &[usize],
    first: impl Fn(&Block) -> bool,
) -> Vec<Vec<usize>> {
    let (a, b): (Vec<usize>, Vec<usize>) = indices.iter().partition(|&&i| first(&all[i]));
    vec![a, b]
}

/// Whether a region left whole nonetheless looks like two columns of text: a vertical
/// whitespace channel at least `min_gutter` wide leaves at least
/// [`GUTTER_MIN_SIDE_BLOCKS`] blocks on each side, and each side's median line spans at
/// least [`COLUMN_LINE_MIN_SHARE`] of the region. The channel is looked for among all the
/// region's blocks, then among its narrow ones (at most half its width) — a line set
/// across both columns fills the channel on its own line.
fn reads_across_columns(blocks: &[Block], min_x: f32, max_x: f32, config: &XyCutConfig) -> bool {
    let range = max_x - min_x;
    if range <= 0.0 {
        return false;
    }
    let narrow: Vec<Block> = blocks
        .iter()
        .copied()
        .filter(|b| b.width <= range * 0.5)
        .collect();
    let any_channel = XyCutConfig {
        min_x_gap: config.min_gutter,
        ..*config
    };
    let two_columns = |set: &[Block]| -> bool {
        if set.len() < 2 * GUTTER_MIN_SIDE_BLOCKS {
            return false;
        }
        let lo = set.iter().map(|b| b.x).fold(f32::MAX, f32::min);
        let hi = set.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
        let Some((channel, _)) = find_best_vertical_gap(set, lo, hi, &any_channel) else {
            return false;
        };
        let side = |left: bool| -> Vec<f32> {
            let mut widths: Vec<f32> = set
                .iter()
                .filter(|b| (b.x + b.width / 2.0 < channel) == left)
                .map(|b| b.width)
                .collect();
            widths.sort_by(f32::total_cmp);
            widths
        };
        let (left, right) = (side(true), side(false));
        let median = |w: &[f32]| w.get(w.len() / 2).copied().unwrap_or(0.0);
        left.len() >= GUTTER_MIN_SIDE_BLOCKS
            && right.len() >= GUTTER_MIN_SIDE_BLOCKS
            && median(&left) >= range * COLUMN_LINE_MIN_SHARE
            && median(&right) >= range * COLUMN_LINE_MIN_SHARE
    };
    two_columns(blocks) || two_columns(&narrow)
}

/// When a block that spans a column gutter is all that keeps a region from splitting,
/// cut the region into bands at that block: above it, its own line, below it.
///
/// A caption, title or running head set across both columns fills the gutter channel
/// on its line, so no vertical cut exists — and when it sits a line away from the
/// text, no horizontal one either; the columns were then read line by line across.
/// The gutter is found among the region's *narrow* blocks (at most half its width),
/// which must be the majority — in single-column text the wide lines are, and
/// nothing is banded. Returns the band's top and bottom, or `None` when there is no
/// such gutter or nothing crosses it; the caller bands the region only when that makes
/// it smaller.
fn spanning_band(
    blocks: &[Block],
    min_x: f32,
    max_x: f32,
    config: &XyCutConfig,
) -> Option<(f32, f32)> {
    let range = max_x - min_x;
    let narrow: Vec<Block> = blocks
        .iter()
        .copied()
        .filter(|b| b.width <= range * 0.5)
        .collect();
    if narrow.len() * 2 <= blocks.len() {
        return None;
    }
    let narrow_min = narrow.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let narrow_max = narrow.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    let (gutter, _) = find_best_vertical_gap(&narrow, narrow_min, narrow_max, config)?;

    // Lines of a text column fill most of their column; the fragments a table of
    // contents (entries | page numbers) or a chart's labels leave either side of a
    // channel do not. Each side's median line must span a quarter of the region.
    let median_width = |left: bool| {
        let mut widths: Vec<f32> = narrow
            .iter()
            .filter(|b| (b.x + b.width / 2.0 < gutter) == left)
            .map(|b| b.width)
            .collect();
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        widths.get(widths.len() / 2).copied().unwrap_or(0.0)
    };
    if median_width(true) < range * COLUMN_LINE_MIN_SHARE
        || median_width(false) < range * COLUMN_LINE_MIN_SHARE
    {
        return None;
    }

    let spanning = blocks
        .iter()
        .filter(|b| b.x < gutter && b.right() > gutter)
        .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))?;
    Some((spanning.y, spanning.bottom()))
}

fn find_best_vertical_gap(
    blocks: &[Block],
    min_x: f32,
    max_x: f32,
    config: &XyCutConfig,
) -> Option<(f32, f32)> {
    let min_gap = config.min_gutter.min(config.min_x_gap);
    let range = max_x - min_x;
    if range < min_gap * 2.0 {
        return None;
    }

    let resolution = 2.0;
    let num_bins = ((range / resolution) as usize).max(1);
    let mut profile = vec![0u32; num_bins];

    for block in blocks {
        let start = ((block.x - min_x) / resolution) as usize;
        let end = ((block.right() - min_x) / resolution) as usize;
        for slot in profile.iter_mut().take(end.min(num_bins)).skip(start) {
            *slot += 1;
        }
    }

    let qualifies = |start: usize, len: usize| -> bool {
        let width = len as f32 * resolution;
        if width >= config.min_x_gap {
            return true;
        }
        if width < config.min_gutter {
            return false;
        }
        let gap_left = min_x + start as f32 * resolution;
        let gap_right = gap_left + width;
        let center = (gap_left + gap_right) / 2.0;
        let left_blocks = blocks
            .iter()
            .filter(|b| b.x + b.width / 2.0 < center)
            .count();
        let right_blocks = blocks.len() - left_blocks;
        let min_side = range * GUTTER_MIN_SIDE_SHARE;
        gap_left - min_x >= min_side
            && max_x - gap_right >= min_side
            && left_blocks >= GUTTER_MIN_SIDE_BLOCKS
            && right_blocks >= GUTTER_MIN_SIDE_BLOCKS
    };

    widest_qualifying_gap(&profile, resolution, min_x, qualifies)
}

/// The widest run of empty bins that `qualifies(start_bin, len_bins)` accepts, as
/// `(center, width)`.
fn widest_qualifying_gap(
    profile: &[u32],
    resolution: f32,
    offset: f32,
    qualifies: impl Fn(usize, usize) -> bool,
) -> Option<(f32, f32)> {
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < profile.len() {
        if profile[i] != 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < profile.len() && profile[i] == 0 {
            i += 1;
        }
        let len = i - start;
        if best.is_none_or(|(_, best_len)| len > best_len) && qualifies(start, len) {
            best = Some((start, len));
        }
    }
    best.map(|(start, len)| {
        let width = len as f32 * resolution;
        (
            offset + (start as f32 + len as f32 / 2.0) * resolution,
            width,
        )
    })
}

fn find_best_horizontal_gap(
    blocks: &[Block],
    min_y: f32,
    max_y: f32,
    min_gap: f32,
) -> Option<(f32, f32)> {
    let range = max_y - min_y;
    if range < min_gap * 2.0 {
        return None;
    }

    let resolution = 2.0;
    let num_bins = ((range / resolution) as usize).max(1);
    let mut profile = vec![0u32; num_bins];

    for block in blocks {
        let top = block.y;
        let bottom = block.bottom();
        let start = ((bottom - min_y) / resolution).max(0.0) as usize;
        let end = ((top - min_y) / resolution) as usize;
        for slot in profile.iter_mut().take(end.min(num_bins)).skip(start) {
            *slot += 1;
        }
    }

    find_widest_gap(&profile, resolution, min_y, min_gap)
}

fn find_widest_gap(
    profile: &[u32],
    resolution: f32,
    offset: f32,
    min_gap: f32,
) -> Option<(f32, f32)> {
    let mut best_start = 0;
    let mut best_len = 0;
    let mut cur_start = 0;
    let mut cur_len = 0;

    for (i, &count) in profile.iter().enumerate() {
        if count == 0 {
            if cur_len == 0 {
                cur_start = i;
            }
            cur_len += 1;
        } else {
            if cur_len > best_len {
                best_start = cur_start;
                best_len = cur_len;
            }
            cur_len = 0;
        }
    }
    if cur_len > best_len {
        best_start = cur_start;
        best_len = cur_len;
    }

    let gap_width = best_len as f32 * resolution;
    if gap_width >= min_gap {
        let gap_center = offset + (best_start as f32 + best_len as f32 / 2.0) * resolution;
        Some((gap_center, gap_width))
    } else {
        None
    }
}
