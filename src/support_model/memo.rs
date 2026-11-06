//! Results shared by the members of one stage A. The members of an ensemble differ in a few
//! merge decisions, so after they part most trees of the plate still step down, merge and
//! draw from the very same inputs: a step, a pair test or a finished layer is a pure function
//! of its inputs (and the stage), and is computed once per stage. Keys are 128-bit
//! fingerprints of the inputs (two independently keyed SipHash-1-3 values), never the inputs
//! themselves, which would hold every influence area of every member.
//!
//! P1S goat under Bambu 02.09 (424 members per rotation, the class tree of
//! `descend::descend_classes` already shared): moves, merges and drawing took 126 + 210 + 160
//! CPU s per rotation without the memo, 5 + 75 + 178 with it; peak memory 1.3 GB (A2L 1.5 GB).
//! The memo never changes a result: the ensemble cache and the plate are byte-identical.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::sync::Mutex;

use super::descend::Moved;
use super::node::Node;
use crate::polygon_ops::{IntPoint, Ring, Rings};

/// A memo table: fingerprint -> result.
pub struct Memo<V>(Mutex<HashMap<u128, V>>);

impl<V> Default for Memo<V> {
    fn default() -> Memo<V> {
        Memo(Mutex::new(HashMap::new()))
    }
}

impl<V: Clone> Memo<V> {
    /// The stored result for `key`, or `compute`'s, stored. Two members asking at once may
    /// both compute it; the result is the same.
    pub fn get_or(&self, key: u128, compute: impl FnOnce() -> V) -> V {
        if let Some(done) = self.lock().get(&key) {
            return done.clone();
        }
        let done = compute();
        self.lock().entry(key).or_insert(done).clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u128, V>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The memo tables of one stage A.
#[derive(Default)]
pub struct StageMemo {
    /// `evaluate::step_down` by (layer, parent without its parents).
    pub steps: Memo<Option<Moved>>,
    /// The pair test but its thickness check, by (layer, destination, source; without
    /// parents), and the check by (that key, erosion).
    pub pairs: Memo<Option<Moved>>,
    pub thick: Memo<bool>,
    /// `draw::finish_layer` by (layer, pieces).
    pub layers: Memo<Rings>,
}

/// Builds a fingerprint.
pub struct Fingerprint(DefaultHasher, DefaultHasher);

impl Fingerprint {
    /// A fingerprint of `kind` (which table it keys).
    pub fn new(kind: u8) -> Fingerprint {
        let (mut first, mut second) = (DefaultHasher::new(), DefaultHasher::new());
        first.write_u8(kind);
        // The second hash is keyed differently by a salt: the two are independent.
        second.write_u64(0x9e37_79b9_7f4a_7c15);
        second.write_u8(kind);
        Fingerprint(first, second)
    }

    pub fn int(&mut self, value: i64) -> &mut Fingerprint {
        self.0.write_i64(value);
        self.1.write_i64(value);
        self
    }

    /// Another fingerprint.
    pub fn key(&mut self, key: u128) -> &mut Fingerprint {
        self.int(key as i64).int((key >> 64) as i64)
    }

    pub fn point(&mut self, point: IntPoint) -> &mut Fingerprint {
        self.int(i64::from(point.x)).int(i64::from(point.y))
    }

    pub fn ring(&mut self, ring: &Ring) -> &mut Fingerprint {
        self.int(ring.len() as i64);
        for &point in ring {
            self.point(point);
        }
        self
    }

    pub fn rings(&mut self, rings: &[Ring]) -> &mut Fingerprint {
        self.int(rings.len() as i64);
        for ring in rings {
            self.ring(ring);
        }
        self
    }

    /// Every field of the node but `parents` (indices into the layer above: no step or
    /// merge reads them).
    pub fn node(&mut self, node: &Node) -> &mut Fingerprint {
        let Node {
            layer,
            depth,
            rdepth,
            foot,
            origin_layer,
            origin_xy,
            hint,
            centre,
            to_bed,
            graceful,
            tight_xy,
            under_roof,
            past_holes,
            keep_round,
            hold_until,
            model_growth,
            parents: _,
            area,
            deleted,
            marked,
        } = node;
        self.int(*layer as i64)
            .int(*depth)
            .int(*rdepth)
            .int(foot.to_bits() as i64)
            .int(*origin_layer as i64)
            .point(*origin_xy)
            .point(*hint);
        match centre {
            Some(centre) => self.int(1).point(*centre),
            None => self.int(0),
        };
        let flags = [
            to_bed,
            graceful,
            tight_xy,
            under_roof,
            past_holes,
            keep_round,
            deleted,
            marked,
        ];
        let bits = flags
            .iter()
            .enumerate()
            .fold(0, |bits, (index, &&flag)| bits | (i64::from(flag) << index));
        self.int(bits)
            .int(*hold_until)
            .int(*model_growth)
            .rings(area)
    }

    /// A merge candidate, `parents` of its node left out.
    pub fn moved(&mut self, moved: &Moved) -> &mut Fingerprint {
        self.node(&moved.node)
            .int(i64::from(moved.bypass))
            .rings(&moved.plate);
        match moved.bounds {
            Some(bounds) => self.int(1).point(bounds.min).point(bounds.max),
            None => self.int(0),
        }
    }

    pub fn finish(&self) -> u128 {
        (u128::from(self.0.finish()) << 64) | u128::from(self.1.finish())
    }
}
