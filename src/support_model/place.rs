//! Stage B4: node placement (spec 02 §5). Bottom nodes take the point of their influence area
//! nearest to their hint; going up, every parent stays above its child when that point lies
//! in its own area and otherwise moves to the nearest point of it (the lazy path). Branches
//! with a dead end below are deleted together with everything above them, tips nothing
//! reaches are deleted, and the layers are compacted in the order the later stages depend on.

use super::node::Node;
use crate::polygon_ops::nearest_inside;

/// The graph with centres, deleted nodes removed and parent indices rewritten.
pub fn place(mut layers: Vec<Vec<Node>>) -> Vec<Vec<Node>> {
    if layers.is_empty() {
        return layers;
    }
    for node in &mut layers[0] {
        node.centre = Some(nearest_inside(&node.area, node.hint));
    }
    for layer in 0..layers.len() {
        if layer > 0 {
            settle_layer(&mut layers, layer);
        }
        pull_parents(&mut layers, layer);
    }
    compact_all(&mut layers);
    layers
}

/// Deletes the nodes of `layer` that nothing below supports or that cannot stand.
fn settle_layer(layers: &mut [Vec<Node>], layer: usize) {
    for index in 0..layers[layer].len() {
        let node = &mut layers[layer][index];
        let dead_end = node.centre.is_none() && node.to_bed;
        let unsupported = !dead_end && !node.marked && node.origin_layer == layer;
        if dead_end || unsupported {
            node.deleted = true;
        }
        if layers[layer][index].deleted {
            reset_parents(layers, layer, index);
        }
    }
}

/// A deleted node takes the centres of its parents with it, so they are deleted in turn.
fn reset_parents(layers: &mut [Vec<Node>], layer: usize, index: usize) {
    let parents = layers[layer][index].parents.clone();
    if let Some(above) = layers.get_mut(layer + 1) {
        for parent in parents {
            above[parent].centre = None;
        }
    }
}

/// Parents without a centre take the point of their area nearest to the child's centre.
fn pull_parents(layers: &mut [Vec<Node>], layer: usize) {
    if layer + 1 >= layers.len() {
        return;
    }
    let (below, above) = layers.split_at_mut(layer + 1);
    for child in &below[layer] {
        let Some(centre) = child.centre.filter(|_| !child.deleted) else {
            continue;
        };
        for &parent in &child.parents {
            let parent = &mut above[0][parent];
            if parent.centre.is_none() {
                parent.centre = Some(nearest_inside(&parent.area, centre));
            }
            parent.marked = true;
        }
    }
}

fn compact_all(layers: &mut [Vec<Node>]) {
    let mut map_above: Option<Vec<Option<usize>>> = None;
    for layer in layers.iter_mut().rev() {
        if let Some(map) = &map_above {
            for node in layer.iter_mut() {
                node.parents = node.parents.iter().filter_map(|&old| map[old]).collect();
            }
        }
        map_above = Some(compact_layer(layer));
    }
}

/// Removes deleted nodes: trailing ones first, then each remaining deleted slot, lowest
/// index first, takes the current last survivor (spec 02 §5.4). Returns old → new index.
pub fn compact_layer(layer: &mut Vec<Node>) -> Vec<Option<usize>> {
    let flags: Vec<bool> = layer.iter().map(|node| node.deleted).collect();
    compact_flags(layer, &flags)
}

fn compact_flags<T>(items: &mut Vec<T>, deleted: &[bool]) -> Vec<Option<usize>> {
    let mut origin: Vec<usize> = (0..items.len()).collect();
    let mut map: Vec<Option<usize>> = (0..items.len()).map(Some).collect();
    let trim = |items: &mut Vec<T>, origin: &mut Vec<usize>, map: &mut Vec<Option<usize>>| {
        while origin.last().is_some_and(|&last| deleted[last]) {
            items.pop();
            map[origin.pop().expect("checked")] = None;
        }
    };
    trim(items, &mut origin, &mut map);
    let mut slot = 0;
    while slot < items.len() {
        if deleted[origin[slot]] {
            items.swap_remove(slot);
            map[origin[slot]] = None;
            origin.swap_remove(slot);
            if slot < origin.len() {
                map[origin[slot]] = Some(slot);
            }
            trim(items, &mut origin, &mut map);
        } else {
            slot += 1;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn survivors(flags: &[u8]) -> Vec<usize> {
        let deleted: Vec<bool> = flags.iter().map(|&flag| flag == 1).collect();
        let mut items: Vec<usize> = (0..flags.len()).collect();
        compact_flags(&mut items, &deleted);
        items
    }

    /// The recorded vectors of spec 02 §8.3.
    #[test]
    fn compaction_test_vectors() {
        assert_eq!(survivors(&[0, 1, 0, 0, 1, 0]), vec![0, 5, 2, 3]);
        assert_eq!(survivors(&[1, 0, 0, 1, 1]), vec![2, 1]);
        assert_eq!(survivors(&[0, 0, 1, 1, 1]), vec![0, 1]);
        assert_eq!(survivors(&[1, 1, 0, 1, 0, 0, 1]), vec![5, 4, 2]);
    }

    #[test]
    fn the_map_follows_the_moves() {
        let deleted = [false, true, false, false];
        let mut items = vec!['a', 'b', 'c', 'd'];
        let map = compact_flags(&mut items, &deleted);
        assert_eq!(items, vec!['a', 'd', 'c']);
        assert_eq!(map, vec![Some(0), None, Some(2), Some(1)]);
    }
}
