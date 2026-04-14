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

/// Segment blocks into reading-order groups using recursive XY-cut.
pub fn xycut_segment(blocks: &[Block], min_x_gap: f32, min_y_gap: f32) -> Vec<Vec<Block>> {
    if blocks.is_empty() {
        return vec![];
    }
    if blocks.len() == 1 {
        return vec![blocks.to_vec()];
    }

    let mut result = Vec::new();
    xycut_recursive(blocks, min_x_gap, min_y_gap, &mut result);

    if result.is_empty() && !blocks.is_empty() {
        result.push(blocks.to_vec());
    }

    result
}

fn xycut_recursive(blocks: &[Block], min_x_gap: f32, min_y_gap: f32, result: &mut Vec<Vec<Block>>) {
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

    let v_gap = find_best_vertical_gap(blocks, min_x, max_x, min_x_gap);
    let h_gap = find_best_horizontal_gap(blocks, min_y, max_y, min_y_gap);

    match (v_gap, h_gap) {
        (Some((v_pos, v_width)), Some((_h_pos, h_height))) if v_width >= h_height => {
            let (left, right) = split_vertical(blocks, v_pos);
            xycut_recursive(&left, min_x_gap, min_y_gap, result);
            xycut_recursive(&right, min_x_gap, min_y_gap, result);
        }
        (_, Some((h_pos, _))) => {
            let (top, bottom) = split_horizontal(blocks, h_pos);
            xycut_recursive(&top, min_x_gap, min_y_gap, result);
            xycut_recursive(&bottom, min_x_gap, min_y_gap, result);
        }
        (Some((v_pos, _)), None) => {
            let (left, right) = split_vertical(blocks, v_pos);
            xycut_recursive(&left, min_x_gap, min_y_gap, result);
            xycut_recursive(&right, min_x_gap, min_y_gap, result);
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
    min_gap: f32,
) -> Option<(f32, f32)> {
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
        for i in start..end.min(num_bins) {
            profile[i] += 1;
        }
    }

    find_widest_gap(&profile, resolution, min_x, min_gap)
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
        for i in start..end.min(num_bins) {
            profile[i] += 1;
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
