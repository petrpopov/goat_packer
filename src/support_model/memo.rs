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
//! Since 2026-11-06 the tables are sharded and keyed by folded-multiply fingerprints
//! (P1S members 21 -> 15 s per rotation at two threads).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Mutex;

use super::descend::Moved;
use super::node::Node;
use super::smooth::obstacle::SegmentGrid;
use crate::polygon_ops::{IntPoint, Ring, Rings};

/// Shards of a memo table: one lock for all of it made the members wait on each other
/// (15 % of a P1S model run at two threads was spent in the mutex).
const SHARDS: usize = 64;

/// A fingerprint is already uniformly random: the table uses its bits as the hash.
#[derive(Default)]
pub struct KeyBits(u64);

impl Hasher for KeyBits {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    fn write_u128(&mut self, value: u128) {
        self.0 = (value >> 64) as u64 ^ value as u64;
    }
}

type Table<V> = HashMap<u128, V, BuildHasherDefault<KeyBits>>;

/// A memo table: fingerprint -> result.
pub struct Memo<V>(Vec<Mutex<Table<V>>>);

impl<V> Default for Memo<V> {
    fn default() -> Memo<V> {
        Memo((0..SHARDS).map(|_| Mutex::new(Table::default())).collect())
    }
}

impl<V: Clone> Memo<V> {
    /// The stored result for `key`, or `compute`'s, stored. Two members asking at once may
    /// both compute it; the result is the same.
    pub fn get_or(&self, key: u128, compute: impl FnOnce() -> V) -> V {
        if let Some(done) = self.lock(key).get(&key) {
            return done.clone();
        }
        let done = compute();
        self.lock(key).entry(key).or_insert(done).clone()
    }

    fn lock(&self, key: u128) -> std::sync::MutexGuard<'_, Table<V>> {
        self.0[(key >> 64) as usize % SHARDS]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    /// The smoothing pass's boundary grid by layer.
    pub obstacles: Memo<std::sync::Arc<SegmentGrid>>,
}

/// Builds a fingerprint: two independent 64-bit lanes of folded 128-bit multiplications
/// (the wyhash mixing step), one word at a time. SipHash, which it replaced, was a tenth of
/// the descent; any 128-bit fingerprint keys the same results.
pub struct Fingerprint(u64, u64);

/// The lanes' constants (wyhash's secret).
const LANE: [u64; 4] = [
    0xa076_1d64_78bd_642f,
    0xe703_7ed1_a0b4_28db,
    0x8ebc_6af0_9c88_c6e3,
    0x5899_65cc_7537_4cc3,
];

/// The two halves of the 128-bit product, folded.
fn fold(a: u64, b: u64) -> u64 {
    let product = u128::from(a) * u128::from(b);
    (product >> 64) as u64 ^ product as u64
}

impl Fingerprint {
    /// A fingerprint of `kind` (which table it keys).
    pub fn new(kind: u8) -> Fingerprint {
        let mut fingerprint = Fingerprint(LANE[0], LANE[2]);
        fingerprint.int(i64::from(kind));
        fingerprint
    }

    pub fn int(&mut self, value: i64) -> &mut Fingerprint {
        let word = value as u64;
        self.0 = fold(self.0 ^ LANE[0], word ^ LANE[1]);
        self.1 = fold(self.1 ^ LANE[2], word.rotate_left(29) ^ LANE[3]);
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
        let (first, second) = (fold(self.0, LANE[3]), fold(self.1, LANE[1]));
        (u128::from(first) << 64) | u128::from(second)
    }
}
