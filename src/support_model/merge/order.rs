//! The order merge candidates are offered in (spec 02 §4.2): a balanced median-split tree
//! over their bounding boxes, flattened. Only the resulting permutation is used, and it
//! depends on the input order because the median search is not stable, which is how the
//! tip order reaches every merge.

use crate::polygon_ops::IntBox;

/// The candidates' indices in tree order.
pub fn aabb_order(boxes: &[IntBox]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    if boxes.len() > 1 {
        split(boxes, &mut order, 0, boxes.len() - 1);
    }
    order
}

/// Orders `order[low..=high]`: split at the middle along the longer side of the range's
/// union box, recurse left first.
fn split(boxes: &[IntBox], order: &mut [usize], low: usize, high: usize) {
    if low >= high {
        return;
    }
    let axis = longer_axis(boxes, &order[low..=high]);
    let middle = (low + high) / 2;
    let key = |index: usize| centroid(&boxes[index], axis);
    select(order, low, high, middle, &key);
    split(boxes, order, low, middle);
    split(boxes, order, middle + 1, high);
}

/// 0 = x, 1 = y: the longer side of the union of the boxes; x on a tie.
fn longer_axis(boxes: &[IntBox], members: &[usize]) -> usize {
    let mut union = boxes[members[0]];
    for &index in &members[1..] {
        let other = boxes[index];
        union.min.x = union.min.x.min(other.min.x);
        union.min.y = union.min.y.min(other.min.y);
        union.max.x = union.max.x.max(other.max.x);
        union.max.y = union.max.y.max(other.max.y);
    }
    let (width, height) = union.size();
    usize::from(height > width)
}

/// (min + max) / 2 on the axis, truncating toward zero (coordinates can be negative).
fn centroid(found: &IntBox, axis: usize) -> i64 {
    let (low, high) = if axis == 0 {
        (found.min.x, found.max.x)
    } else {
        (found.min.y, found.max.y)
    };
    (i64::from(low) + i64::from(high)) / 2
}

/// Quickselect with median-of-three pivots and Hoare partitioning, placing the element of
/// rank `target` at `target` (Sedgewick's variant): the first, middle and last elements are
/// sorted by three compare-and-swaps, the middle one is the pivot and is parked before the
/// last, both scans stop on keys equal to the pivot, and ranges of at most three elements
/// are done after the three-element sort.
fn select(
    order: &mut [usize],
    mut low: usize,
    mut high: usize,
    target: usize,
    key: &impl Fn(usize) -> i64,
) {
    loop {
        if high <= low {
            return;
        }
        if high == low + 1 {
            if key(order[high]) < key(order[low]) {
                order.swap(low, high);
            }
            return;
        }
        let middle = (low + high) / 2;
        if key(order[low]) > key(order[middle]) {
            order.swap(low, middle);
        }
        if key(order[low]) > key(order[high]) {
            order.swap(low, high);
        }
        if key(order[middle]) > key(order[high]) {
            order.swap(middle, high);
        }
        if high - low <= 2 {
            return;
        }
        order.swap(middle, high - 1);
        let pivot = key(order[high - 1]);
        let (mut i, mut j) = (low, high - 1);
        loop {
            i += 1;
            while key(order[i]) < pivot {
                i += 1;
            }
            j -= 1;
            while pivot < key(order[j]) {
                j -= 1;
            }
            if i >= j {
                break;
            }
            order.swap(i, j);
        }
        order.swap(i, high - 1);
        if i >= target {
            high = i - 1;
        }
        if i <= target {
            low = i + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::IntPoint;

    fn boxes(list: &[(i32, i32, i32, i32)]) -> Vec<IntBox> {
        list.iter()
            .map(|&(x0, y0, x1, y1)| IntBox {
                min: IntPoint::new(x0, y0),
                max: IntPoint::new(x1, y1),
            })
            .collect()
    }

    /// The recorded vectors of spec 02 §8.1.
    #[test]
    fn aabb_order_test_vectors() {
        let a1 = boxes(&[
            (0, 0, 0, 0),
            (10, 1, 10, 1),
            (5, 2, 5, 2),
            (-3, 0, -3, 0),
            (7, -1, 7, -1),
            (2, 3, 2, 3),
            (5, 0, 5, 0),
        ]);
        assert_eq!(aabb_order(&a1), vec![3, 0, 5, 2, 6, 4, 1]);
        let a2 = boxes(&[
            (4, 0, 4, 0),
            (4, 5, 4, 5),
            (1, 1, 1, 1),
            (4, 2, 4, 2),
            (9, 9, 9, 9),
            (4, 1, 4, 1),
            (0, 8, 0, 8),
            (4, 3, 4, 3),
        ]);
        assert_eq!(aabb_order(&a2), vec![2, 0, 5, 6, 3, 7, 1, 4]);
        let a3 = boxes(&[
            (-5, -20, -3, -18),
            (0, 10, 2, 12),
            (-1, -1, 1, 1),
            (3, 30, 5, 31),
            (-4, 5, -2, 6),
            (1, -9, 2, -8),
        ]);
        assert_eq!(aabb_order(&a3), vec![0, 5, 2, 4, 1, 3]);
        let a4 = boxes(&[
            (0, 0, 0, 0),
            (4, 4, 4, 4),
            (4, 0, 4, 0),
            (0, 4, 0, 4),
            (2, 2, 2, 2),
        ]);
        assert_eq!(aabb_order(&a4), vec![0, 4, 3, 2, 1]);
    }
}
