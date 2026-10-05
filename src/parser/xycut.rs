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

/// Segment blocks into reading-order groups using recursive XY-cut.
pub fn xycut_segment(blocks: &[Block], config: &XyCutConfig) -> Vec<Vec<Block>> {
    if blocks.is_empty() {
        return vec![];
    }
    if blocks.len() == 1 {
        return vec![blocks.to_vec()];
    }

    let mut result = Vec::new();
    xycut_recursive(blocks, config, &mut result);

    if result.is_empty() && !blocks.is_empty() {
        result.push(blocks.to_vec());
    }

    result
}

fn xycut_recursive(blocks: &[Block], config: &XyCutConfig, result: &mut Vec<Vec<Block>>) {
    if blocks.is_empty() {
        return;
    }
    if blocks.len() == 1 {
        result.push(blocks.to_vec());
        return;
    }

    let min_x = blocks.iter().map(|b| b.x).fold(f32::MAX, f32::min);
    let max_x = blocks.iter().map(|b| b.right()).fold(f32::MIN, f32::max);
    let min_y = blocks.iter().map(|b| b.bottom()).fold(f32::MAX, f32::min);
    let max_y = blocks.iter().map(|b| b.y).fold(f32::MIN, f32::max);

    let v_gap = find_best_vertical_gap(blocks, min_x, max_x, config);
    let h_gap = find_best_horizontal_gap(blocks, min_y, max_y, config.min_y_gap);

    match (v_gap, h_gap) {
        (Some((v_pos, v_width)), Some((_h_pos, h_height))) if v_width >= h_height => {
            let (left, right) = split_vertical(blocks, v_pos);
            xycut_recursive(&left, config, result);
            xycut_recursive(&right, config, result);
        }
        (_, Some((h_pos, _))) => {
            let (top, bottom) = split_horizontal(blocks, h_pos);
            xycut_recursive(&top, config, result);
            xycut_recursive(&bottom, config, result);
        }
        (Some((v_pos, _)), None) => {
            let (left, right) = split_vertical(blocks, v_pos);
            xycut_recursive(&left, config, result);
            xycut_recursive(&right, config, result);
        }
        (None, None) => {
            result.push(blocks.to_vec());
        }
    }
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

fn split_vertical(blocks: &[Block], split_x: f32) -> (Vec<Block>, Vec<Block>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for block in blocks {
        let center_x = block.x + block.width / 2.0;
        if center_x < split_x {
            left.push(*block);
        } else {
            right.push(*block);
        }
    }
    (left, right)
}

fn split_horizontal(blocks: &[Block], split_y: f32) -> (Vec<Block>, Vec<Block>) {
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for block in blocks {
        let center_y = block.y - block.height / 2.0;
        if center_y > split_y {
            top.push(*block);
        } else {
            bottom.push(*block);
        }
    }
    (top, bottom)
}
