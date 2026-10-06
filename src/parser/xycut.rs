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

/// Minimum share of a region's width a line must span to count as a line of running text
/// beside a gutter: a column's lines fill most of it, while the fragments a table of
/// contents (entries | page numbers), a chart's labels or a list's markers leave beside a
/// whitespace channel do not. A side reads as a text column when it holds at least
/// [`GUTTER_MIN_SIDE_BLOCKS`] such lines and they carry most of its text — a figure set in
/// the column adds many short label lines without making it any less a column of text.
pub const COLUMN_LINE_MIN_SHARE: f32 = 0.25;

/// What recursive XY-cut made of a page's blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segmentation {
    /// Reading-order groups, each a list of indices into the segmented blocks. Every block
    /// is in exactly one group.
    pub groups: Vec<Vec<usize>>,
    /// Regions left whole — read line by line across — although a whitespace channel
    /// divides their text into two sides that each look like a column of text (see
    /// [`COLUMN_LINE_MIN_SHARE`]). The split rules declined them; the reading order there
    /// is a guess.
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

    // With no column gutter left in the region, a strip of letters stacked in its margin is
    // read on its own before the region is cut into bands, which would scatter it.
    if v_gap.is_none() {
        if let Some(at) = margin_strip(&blocks, min_x, max_x, config) {
            for part in split_v(at) {
                partition_recursive(all, &part, config, out);
            }
            return;
        }
    }

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
            None => match column_band(&blocks, config) {
                Some(at) => split_h(at),
                None => Vec::new(),
            },
        },
    };

    if parts.is_empty() {
        if reads_across_columns(&blocks, config) {
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

/// Whether a region left whole nonetheless looks like two columns of text — see
/// [`text_column_gutter`].
fn reads_across_columns(blocks: &[Block], config: &XyCutConfig) -> bool {
    text_column_gutter(blocks, config.min_gutter).is_some()
}

/// The whitespace channel that divides `blocks` into two columns of text, if there is one:
/// a vertical channel at least `min_gutter` wide with a column of running text on each
/// side (see [`COLUMN_LINE_MIN_SHARE`]). The channel is looked for among all the
/// blocks, then among the narrow ones (at most half the extent) — a line set across both
/// columns fills the channel on its own line. Returns the channel's centre.
///
/// The fragments a table of contents (entries | page numbers), a chart's labels, a list's
/// markers or a table's cells leave beside a channel are short, so they never qualify:
/// what does is running text on both sides.
pub fn text_column_gutter(blocks: &[Block], min_gutter: f32) -> Option<f32> {
    let min_x = blocks.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let max_x = blocks.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    let range = max_x - min_x;
    if range <= 0.0 {
        return None;
    }
    let narrow = narrow_blocks(blocks, range, min_gutter);
    let divides_into_text_columns = |set: &[Block], channel: f32| -> bool {
        let side = |left: bool| {
            line_widths(
                set.iter()
                    .filter(|b| (b.x + b.width / 2.0 < channel) == left),
                min_gutter,
            )
        };
        reads_as_text_column(&side(true), range) && reads_as_text_column(&side(false), range)
    };
    let two_columns = |set: &[Block]| -> Option<f32> {
        if set.len() < 2 * GUTTER_MIN_SIDE_BLOCKS {
            return None;
        }
        // Every channel wide enough, widest first: the widest is not always the gutter — a
        // margin tab or a column of figures beside the text leaves a wider one.
        vertical_channels(set, min_gutter)
            .into_iter()
            .find(|&channel| divides_into_text_columns(set, channel))
    };
    two_columns(blocks).or_else(|| two_columns(&narrow))
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
    let narrow = narrow_blocks(blocks, range, config.min_gutter);
    if narrow.len() * 2 <= blocks.len() {
        return None;
    }
    let narrow_min = narrow.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let narrow_max = narrow.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    let (gutter, _) = find_best_vertical_gap(&narrow, narrow_min, narrow_max, config)?;

    // Lines of a text column fill most of their column; the fragments a table of
    // contents (entries | page numbers) or a chart's labels leave either side of a
    // channel do not.
    let text_column = |left: bool| {
        let lines = line_widths(
            narrow
                .iter()
                .filter(|b| (b.x + b.width / 2.0 < gutter) == left),
            config.min_gutter,
        );
        reads_as_text_column(&lines, range)
    };
    if !text_column(true) || !text_column(false) {
        return None;
    }

    // A line set across the columns may be drawn in pieces none of which crosses the
    // gutter on its own; the line does.
    let spanning = lines_of(blocks, config.min_gutter)
        .into_iter()
        .map(|line| line.extent)
        .filter(|b| b.x < gutter && b.right() > gutter)
        .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))?;
    Some((spanning.y, spanning.bottom()))
}

/// A line of text: blocks on one baseline that sit a word space apart or closer.
struct Line {
    extent: Block,
    members: Vec<usize>,
}

/// The lines `blocks` make: blocks on one baseline (centres within half a block height)
/// joined while the space between them is under `join_gap`.
///
/// A block is not always a line — a producer that positions justified text word by word
/// leaves one block per word — and the column tests are about lines: how far they run, and
/// whether one crosses a gutter. A gap of `join_gap` (the narrowest column gutter) or more
/// is never a word space, so no line is joined across a gutter.
fn lines_of(blocks: &[Block], join_gap: f32) -> Vec<Line> {
    let centre = |b: &Block| b.y - b.height / 2.0;
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by(|&a, &b| centre(&blocks[b]).total_cmp(&centre(&blocks[a])));

    let mut lines = Vec::new();
    let mut start = 0;
    while start < order.len() {
        // One baseline: every block whose centre is within half a height of the first's.
        let first = &blocks[order[start]];
        let mut end = start + 1;
        while end < order.len() {
            let b = &blocks[order[end]];
            if (centre(first) - centre(b)).abs() > first.height.max(b.height) / 2.0 {
                break;
            }
            end += 1;
        }
        let mut baseline = order[start..end].to_vec();
        baseline.sort_by(|&a, &b| blocks[a].x.total_cmp(&blocks[b].x));
        let mut current: Option<Line> = None;
        for i in baseline {
            let b = blocks[i];
            match &mut current {
                Some(line) if b.x - line.extent.right() < join_gap => {
                    let top = line.extent.y.max(b.y);
                    let bottom = line.extent.bottom().min(b.bottom());
                    let right = line.extent.right().max(b.right());
                    line.extent = Block {
                        x: line.extent.x,
                        y: top,
                        width: right - line.extent.x,
                        height: top - bottom,
                    };
                    line.members.push(i);
                }
                _ => {
                    lines.extend(current.take());
                    current = Some(Line {
                        extent: b,
                        members: vec![i],
                    });
                }
            }
        }
        lines.extend(current);
        start = end;
    }
    lines
}

/// The widths of the lines `blocks` make (see [`lines_of`]), shortest first.
fn line_widths<'a>(blocks: impl Iterator<Item = &'a Block>, join_gap: f32) -> Vec<f32> {
    let blocks: Vec<Block> = blocks.copied().collect();
    let mut widths: Vec<f32> = lines_of(&blocks, join_gap)
        .iter()
        .map(|line| line.extent.width)
        .collect();
    widths.sort_by(f32::total_cmp);
    widths
}

/// The blocks whose line spans at most half of `range` — what can make up a column. A line
/// set across the columns is wide even when drawn word by word, so none of its pieces is
/// kept.
fn narrow_blocks(blocks: &[Block], range: f32, join_gap: f32) -> Vec<Block> {
    lines_of(blocks, join_gap)
        .into_iter()
        .filter(|line| line.extent.width <= range * 0.5)
        .flat_map(|line| line.members.into_iter().map(|i| blocks[i]))
        .collect()
}

/// The height to cut a region at so that one part reads as two columns of text, when the
/// region as a whole does not: the columns end (or begin) at a band set across them — a
/// run of footnotes, a full-width table — that leaves no whitespace channel through the
/// region and too little space above it for a plain horizontal cut. Lines are tried as cut
/// points from the outside in, and the cut that leaves the larger columns part wins.
///
/// [`spanning_band`] covers the single line set across the columns; this covers the band of
/// several, whose lines need not even reach across both columns on their own — a
/// footnote's continuation line can be no wider than a column and still close the gutter.
fn column_band(blocks: &[Block], config: &XyCutConfig) -> Option<f32> {
    let centre = |b: &Block| b.y - b.height / 2.0;
    let mut lines = lines_of(blocks, config.min_gutter);
    if lines.len() < 2 * GUTTER_MIN_SIDE_BLOCKS + 1 {
        return None;
    }
    lines.sort_by(|a, b| centre(&b.extent).total_cmp(&centre(&a.extent)));
    let columns = |part: &[Line]| -> bool {
        let members: Vec<Block> = part
            .iter()
            .flat_map(|line| line.members.iter().map(|&i| blocks[i]))
            .collect();
        text_column_gutter(&members, config.min_gutter).is_some()
    };
    let mut best: Option<(usize, f32)> = None;
    for cut in GUTTER_MIN_SIDE_BLOCKS..=lines.len() - GUTTER_MIN_SIDE_BLOCKS {
        let (above, below) = lines.split_at(cut);
        let (last_above, first_below) = (&above[above.len() - 1].extent, &below[0].extent);
        // Never between two lines that overlap: that would cut through text.
        if last_above.bottom() < first_below.y {
            continue;
        }
        let at = (centre(last_above) + centre(first_below)) / 2.0;
        for part in [above, below] {
            if best.is_some_and(|(size, _)| size >= part.len()) {
                continue;
            }
            if columns(part) {
                best = Some((part.len(), at));
            }
        }
    }
    best.map(|(_, at)| at)
}

/// Where `blocks` divide into separate reading flows side by side, if they do: the gutter
/// between two columns of text ([`text_column_gutter`]) or the channel that sets a margin
/// strip apart — the boundaries XY-Cut reads across only as two flows.
pub fn reading_boundary(blocks: &[Block], config: &XyCutConfig) -> Option<f32> {
    let min_x = blocks.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let max_x = blocks.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    text_column_gutter(blocks, config.min_gutter)
        .or_else(|| margin_strip(blocks, min_x, max_x, config))
}

/// Widest share of a region a margin strip may take — see [`margin_strip`].
pub const MARGIN_STRIP_MAX_SHARE: f32 = 0.12;

/// A strip of short blocks along the region's left or right edge that is a flow of its own,
/// returned as the centre of the channel that sets it apart.
///
/// A thumb-index tab set as letters stacked down the margin, or a column of margin notes,
/// stands beside the text with a channel narrower than a column gutter, so no other rule
/// cuts it off — and read with the text, its letters fall between the text's lines and
/// break its paragraphs apart. What sets it apart from a list's markers or a page's line
/// numbers, which also stand in a narrow strip, is that its lines are not the text's
/// lines: markers and line numbers sit on the baselines of the text beside them, a tab's
/// letters mostly do not. So the strip — at most [`MARGIN_STRIP_MAX_SHARE`] of the region,
/// at least [`GUTTER_MIN_SIDE_BLOCKS`] short blocks, across a channel at least
/// `min_gutter` wide — is cut off when most of its blocks share no baseline with the text.
fn margin_strip(blocks: &[Block], min_x: f32, max_x: f32, config: &XyCutConfig) -> Option<f32> {
    let range = max_x - min_x;
    if range <= 0.0 {
        return None;
    }
    // Text on one line shares its baseline to within a point or two, whatever its size.
    let on_a_baseline_of = |b: &Block, others: &[&Block]| {
        others
            .iter()
            .any(|o| (o.y - b.y).abs() <= b.height.max(o.height) * 0.2)
    };
    let channels = vertical_channels(blocks, config.min_gutter);
    channels.into_iter().find(|&channel| {
        let (left, right): (Vec<&Block>, Vec<&Block>) =
            blocks.iter().partition(|b| b.x + b.width / 2.0 < channel);
        let sides = [(&left, &right), (&right, &left)];
        let is_strip = sides.into_iter().any(|(strip, text)| {
            let lo = strip.iter().map(|b| b.x).fold(f32::MAX, f32::min);
            let hi = strip.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
            let at_edge = lo <= min_x + 0.5 || hi >= max_x - 0.5;
            let aligned = strip.iter().filter(|b| on_a_baseline_of(b, text)).count();
            at_edge
                && strip.len() >= GUTTER_MIN_SIDE_BLOCKS
                && text.len() >= GUTTER_MIN_SIDE_BLOCKS
                && hi - lo <= range * MARGIN_STRIP_MAX_SHARE
                && aligned * 2 < strip.len()
        });
        is_strip
    })
}

/// The centres of the vertical whitespace channels at least `min_width` wide that run
/// through `blocks` — no block reaches into them — widest first.
fn vertical_channels(blocks: &[Block], min_width: f32) -> Vec<f32> {
    let lo = blocks.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let mut edges: Vec<(f32, f32)> = blocks.iter().map(|b| (b.x, b.right())).collect();
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut channels: Vec<(f32, f32)> = Vec::new();
    let mut reach = lo;
    for (left, right) in edges {
        if left - reach >= min_width {
            channels.push(((reach + left) / 2.0, left - reach));
        }
        reach = reach.max(right);
    }
    channels.sort_by(|a, b| b.1.total_cmp(&a.1));
    channels.into_iter().map(|(centre, _)| centre).collect()
}

/// Whether lines of these widths make a column of running text in a region `range` wide:
/// at least [`GUTTER_MIN_SIDE_BLOCKS`] of them span [`COLUMN_LINE_MIN_SHARE`] of it, and
/// those carry most of the text.
fn reads_as_text_column(widths: &[f32], range: f32) -> bool {
    let long: Vec<f32> = widths
        .iter()
        .copied()
        .filter(|&w| w >= range * COLUMN_LINE_MIN_SHARE)
        .collect();
    let all: f32 = widths.iter().sum();
    long.len() >= GUTTER_MIN_SIDE_BLOCKS && long.iter().sum::<f32>() * 2.0 >= all
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
