//! Merge rounds over the ordered candidates (spec 02 §4.3-§4.4): consecutive buckets merged
//! inside, then neighbouring buckets merged pairwise until one remains. The pair test is a
//! parameter, so the order rules are tested on abstract labels (spec 02 §8.2).
//!
//! Order properties kept: the destination is always the element on the left and keeps its
//! identity; an absorbed element leaves by moving the range's last element into its slot
//! (inside a bucket and on the destination side) or its first element (on the source side),
//! so survivors do not keep their relative order.

/// The pair test: the merged element, or `None` when the pair is refused.
pub type Merger<'a, T> = &'a dyn Fn(&T, &T) -> Option<T>;

/// How many candidates share a bucket (spec 02 §4.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Buckets {
    pub size: usize,
}

/// Merges `items` (already in tree order) with buckets of `buckets.size`.
pub fn merge_rounds<T: Clone>(items: Vec<T>, buckets: Buckets, merge: Merger<T>) -> Vec<T> {
    let count = items.len();
    if count < 2 {
        return items;
    }
    let size = buckets.size.max(2);
    let full = if size == 4 {
        (count + 2) / 4
    } else {
        count / 2
    };
    let mut groups: Vec<Vec<T>> = Vec::new();
    let mut rest = items;
    for _ in 0..full {
        let take = size.min(rest.len());
        let tail = rest.split_off(take);
        groups.push(std::mem::replace(&mut rest, tail));
    }
    if !rest.is_empty() {
        groups.push(rest);
    }
    for group in &mut groups {
        merge_inside(group, merge);
    }
    while groups.len() > 1 {
        let mut next = Vec::with_capacity(groups.len().div_ceil(2));
        let mut pairs = groups.into_iter();
        while let Some(mut destination) = pairs.next() {
            if let Some(source) = pairs.next() {
                merge_sets(&mut destination, source, merge);
            }
            next.push(destination);
        }
        groups = next;
    }
    groups.pop().unwrap_or_default()
}

/// Round 1: the leftmost live element offers itself to every later one; an absorbed element
/// is replaced by the range's last, and the offers start again after the destination.
fn merge_inside<T: Clone>(group: &mut Vec<T>, merge: Merger<T>) {
    let mut destination = 0;
    while destination < group.len() {
        let mut source = destination + 1;
        while source < group.len() {
            if let Some(merged) = merge(&group[destination], &group[source]) {
                group[destination] = merged;
                group.swap_remove(source);
                source = destination + 1;
            } else {
                source += 1;
            }
        }
        destination += 1;
    }
}

/// Later rounds: every element of `source` goes to the first element of `destination` that
/// accepts it; the absorbing element then tries the later elements of `destination`. The
/// survivors of `source` follow `destination` afterwards.
fn merge_sets<T: Clone>(destination: &mut Vec<T>, mut source: Vec<T>, merge: Merger<T>) {
    let mut at = 0;
    while at < source.len() {
        let taker = (0..destination.len()).find_map(|index| {
            merge(&destination[index], &source[at]).map(|merged| (index, merged))
        });
        let Some((index, merged)) = taker else {
            at += 1;
            continue;
        };
        destination[index] = merged;
        // Remove by first: the first element takes the slot; the slot is offered again.
        source.swap(0, at);
        source.remove(0);
        at = at.saturating_sub(1);
        let mut later = index + 1;
        while later < destination.len() {
            if let Some(merged) = merge(&destination[index], &destination[later]) {
                destination[index] = merged;
                destination.swap_remove(later);
            } else {
                later += 1;
            }
        }
    }
    destination.extend(source);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Elements are label strings; the pair test merges two when a listed pair has one label
    /// in each, joining the labels with '+'.
    fn run(labels: &str, pairs: &[&str], size: usize) -> String {
        let items: Vec<String> = labels.split(' ').map(str::to_owned).collect();
        let merge = |a: &String, b: &String| {
            let listed = a.split('+').any(|x| {
                b.split('+').any(|y| {
                    pairs.iter().any(|pair| {
                        let (p, q) = (&pair[0..1], &pair[1..2]);
                        (x == p && y == q) || (x == q && y == p)
                    })
                })
            });
            listed.then(|| format!("{a}+{b}"))
        };
        merge_rounds(items, Buckets { size }, &merge)
            .iter()
            .map(|label| {
                let mut parts: Vec<&str> = label.split('+').collect();
                parts.sort_unstable();
                parts.join("+")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The recorded vectors of spec 02 §8.2 (the pair order inside a label is irrelevant).
    #[test]
    fn merge_round_test_vectors() {
        assert_eq!(run("a b c d e", &["ab", "ce"], 4), "a+b, d, c+e");
        assert_eq!(run("a b c d e", &["ab", "ce"], 2), "a+b, c+e, d");
        let eight = "a b c d e f g h";
        assert_eq!(
            run(eight, &["ab", "ah", "bh", "cf"], 4),
            "a+b+h, d, c+f, g, e"
        );
        assert_eq!(
            run(eight, &["ab", "ah", "bh", "cf"], 2),
            "a+b+h, c+f, d, g, e"
        );
        let nine = "a b c d e f g h i";
        assert_eq!(run(nine, &["ab", "cg", "di"], 4), "a+b, d+i, c+g, f, e, h");
        assert_eq!(run(nine, &["ab", "cg", "di"], 2), "a+b, c+g, d+i, f, e, h");
        let seven = "a b c d e f g";
        assert_eq!(run(seven, &["ab", "cg", "ef", "be"], 4), "a+b+e+f, d, c+g");
        assert_eq!(run(seven, &["ab", "cg", "ef", "be"], 2), "a+b+e+f, c+g, d");
    }
}
