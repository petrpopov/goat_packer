//! A raster as bit rows: the fast form `masks_overlap` compares word by word.

use crate::span_raster::SpanRaster;

#[derive(Clone, Debug)]
pub struct PixelMask {
    pub min_x: i32,
    pub min_y: i32,
    pub width: usize,
    pub height: usize,
    pub pixel_count: usize,
    pub(crate) rows: Vec<Vec<u64>>,
}

impl PixelMask {
    /// Packs span rows into bit rows, trimmed to the pixels' tight bounds. An empty raster
    /// becomes a 1x1 mask with no pixels set (it collides with nothing).
    pub(crate) fn from_spans(raster: &SpanRaster) -> Self {
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut first_row = None;
        let mut last_row = 0usize;
        let mut pixel_count = 0usize;
        for (index, row) in raster.rows.iter().enumerate() {
            if row.is_empty() {
                continue;
            }
            first_row.get_or_insert(index);
            last_row = index;
            min_x = min_x.min(row[0].0);
            max_x = max_x.max(row[row.len() - 1].1 - 1);
            pixel_count += row.iter().map(|&(a, b)| (b - a) as usize).sum::<usize>();
        }
        let Some(first_row) = first_row else {
            return Self {
                min_x: 0,
                min_y: 0,
                width: 1,
                height: 1,
                pixel_count: 0,
                rows: vec![vec![0u64]],
            };
        };
        let width = (max_x - min_x + 1) as usize;
        let height = last_row - first_row + 1;
        let words = width.div_ceil(64);
        let mut rows = vec![vec![0u64; words]; height];
        for (local_y, row) in raster.rows[first_row..=last_row].iter().enumerate() {
            for &(start, end) in row {
                set_bit_range(
                    &mut rows[local_y],
                    (start - min_x) as usize,
                    (end - min_x) as usize,
                );
            }
        }
        Self {
            min_x,
            min_y: raster.min_y + first_row as i32,
            width,
            height,
            pixel_count,
            rows,
        }
    }

    /// Row `local_y` as half-open spans of set bits, in mask-local columns.
    pub(crate) fn row_spans(&self, local_y: usize) -> Vec<(i32, i32)> {
        let row = &self.rows[local_y];
        let mut spans = Vec::new();
        let mut x = 0usize;
        while let Some(start) = next_bit(row, x, true, self.width) {
            let end = next_bit(row, start, false, self.width).unwrap_or(self.width);
            spans.push((start as i32, end as i32));
            x = end;
        }
        spans
    }

    pub fn contains_local(&self, x: i32, y: i32) -> bool {
        if x < self.min_x
            || y < self.min_y
            || x >= self.min_x + self.width as i32
            || y >= self.min_y + self.height as i32
        {
            return false;
        }
        let local_x = (x - self.min_x) as usize;
        let local_y = (y - self.min_y) as usize;
        (self.rows[local_y][local_x / 64] & (1u64 << (local_x % 64))) != 0
    }

    pub fn pixels(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.rows
            .iter()
            .enumerate()
            .flat_map(move |(row_index, row)| {
                row.iter().enumerate().flat_map(move |(word_index, &word)| {
                    (0..64).filter_map(move |bit| {
                        let local_x = word_index * 64 + bit;
                        if local_x < self.width && (word & (1u64 << bit)) != 0 {
                            Some((self.min_x + local_x as i32, self.min_y + row_index as i32))
                        } else {
                            None
                        }
                    })
                })
            })
    }
}

/// First bit at or after `from` equal to `value`, below `limit`. Bits past `limit` in the
/// last word are zero, so a search for a clear bit there reports `limit` as `None`.
fn next_bit(row: &[u64], from: usize, value: bool, limit: usize) -> Option<usize> {
    if from >= limit {
        return None;
    }
    let load = |index: usize| if value { row[index] } else { !row[index] };
    let mut index = from / 64;
    let mut word = load(index) & (!0u64 << (from % 64));
    loop {
        if word != 0 {
            let position = index * 64 + word.trailing_zeros() as usize;
            return (position < limit).then_some(position);
        }
        index += 1;
        if index >= row.len() {
            return None;
        }
        word = load(index);
    }
}

/// Sets bits `[start, end)` of a bit row.
pub(crate) fn set_bit_range(row: &mut [u64], start: usize, end: usize) {
    if start >= end {
        return;
    }
    let (first, last) = (start / 64, (end - 1) / 64);
    let low = !0u64 << (start % 64);
    let high = !0u64 >> (63 - (end - 1) % 64);
    if first == last {
        row[first] |= low & high;
        return;
    }
    row[first] |= low;
    for word in &mut row[first + 1..last] {
        *word = !0;
    }
    row[last] |= high;
}

/// The 64 bits `[start, start + 64)` of a bit row; positions outside the row read as clear.
fn bits_at(row: &[u64], start: i64) -> u64 {
    let word_at = |index: i64| -> u64 {
        if index < 0 || index as usize >= row.len() {
            0
        } else {
            row[index as usize]
        }
    };
    let index = start.div_euclid(64);
    let shift = start.rem_euclid(64) as u32;
    if shift == 0 {
        word_at(index)
    } else {
        (word_at(index) >> shift) | (word_at(index + 1) << (64 - shift))
    }
}

pub fn masks_overlap(a: &PixelMask, a_x: i32, a_y: i32, b: &PixelMask, b_x: i32, b_y: i32) -> bool {
    let a_min_x = a.min_x + a_x;
    let a_min_y = a.min_y + a_y;
    let a_max_x = a_min_x + a.width as i32;
    let a_max_y = a_min_y + a.height as i32;
    let b_min_x = b.min_x + b_x;
    let b_min_y = b.min_y + b_y;
    let b_max_x = b_min_x + b.width as i32;
    let b_max_y = b_min_y + b.height as i32;
    if a_max_x <= b_min_x || b_max_x <= a_min_x || a_max_y <= b_min_y || b_max_y <= a_min_y {
        return false;
    }
    // Word-parallel: for each shared row, compare every word of `a` against the 64 bits of
    // `b` that sit under it.
    let first_row = a_min_y.max(b_min_y);
    let last_row = a_max_y.min(b_max_y);
    let offset = i64::from(a_min_x - b_min_x);
    for y in first_row..last_row {
        let a_row = &a.rows[(y - a_min_y) as usize];
        let b_row = &b.rows[(y - b_min_y) as usize];
        for (index, &word) in a_row.iter().enumerate() {
            if word != 0 && word & bits_at(b_row, offset + index as i64 * 64) != 0 {
                return true;
            }
        }
    }
    false
}
