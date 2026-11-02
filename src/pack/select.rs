//! Which candidate layout wins (design 2026-11-02, §9.4, §9.6): every member proposes
//! layouts with a ranking key, the best few form a shortlist, every shortlisted layout gets
//! the raster check, and the vector check runs on the best raster-valid one only. A layout
//! that fails a check is logged and the next one is taken: a raster failure means a no-fit
//! bug, a vector failure after a raster pass a raster bug, so both are reported.

use crate::layout::PixelPlacement;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Candidates that get the raster check, best first.
pub const SHORTLIST_LEN: usize = 8;

/// Portfolio members (design §9.2), in the order of the canonical tie key.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Member {
    /// Greedy bottom-left sweeps over every class of the run (member D).
    Sweep,
    /// Sweeps over a palette of the best legacy angle families and their quarter turns
    /// (legacy footprints that may turn freely only).
    Mixed,
    /// Brick rows: copies of one class on a sheared row lattice, the shift scanned.
    Rows,
    /// Single lattice (member A, work package P4).
    Lattice,
    /// 0/180 double lattice (member B, P5).
    DoubleLattice,
    /// One guillotine cut, two lattices (member C, P5).
    TwoBlock,
    /// GLS insertion and ruin-and-recreate (member E, P6).
    Improve,
}

impl Member {
    /// The members this package implements, in run order.
    pub const AVAILABLE: [Member; 3] = [Member::Sweep, Member::Mixed, Member::Rows];
}

/// What decides between two equal layouts that no geometry tells apart: the member, the
/// tower anchor, then the member's own canonical tuple (variant, shift, HNF entries, ...).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TieKey {
    pub member: Member,
    /// Index of the tower anchor in the run's candidate list (0 = the source anchor, and 0
    /// without a tower).
    pub anchor: u32,
    pub detail: Vec<i64>,
}

/// The ranking key of a layout, compared lexicographically (design §9.4). `Ord` sorts the
/// winner first.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RankKey {
    pub count: usize,
    /// Robustness δ* in quarter millimetres (higher wins). Placeholder: 0 for every layout
    /// until work package P6 computes it, so it never decides yet.
    pub robustness_quarter_mm: u8,
    /// The canonical copy clears the flavour threshold (§9.7; true wins). Placeholder: true
    /// for every layout until the canonical-order step lands.
    pub canonical_clear: bool,
    /// Box around every copy's printed extent, pixels (smaller wins).
    pub bbox_area_px: u64,
    pub tie: TieKey,
}

impl Ord for RankKey {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .count
            .cmp(&self.count)
            .then(other.robustness_quarter_mm.cmp(&self.robustness_quarter_mm))
            .then(other.canonical_clear.cmp(&self.canonical_clear))
            .then(self.bbox_area_px.cmp(&other.bbox_area_px))
            .then_with(|| self.tie.cmp(&other.tie))
    }
}

impl PartialOrd for RankKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One proposed layout: copies on the run's origin grid, classes indexed by the run.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) key: RankKey,
    /// Index into the run's tower anchors; `None` without a tower.
    pub(crate) anchor: Option<usize>,
    pub(crate) placements: Vec<PixelPlacement>,
}

/// A shortlisted candidate that failed a check.
#[derive(Clone, Debug, Serialize)]
pub struct CheckFailure {
    /// Position in the shortlist (0 = best key).
    pub rank: usize,
    pub member: Member,
    pub count: usize,
    pub message: String,
}

/// How the winner was validated.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ValidationReport {
    pub shortlisted: usize,
    pub raster_passed: usize,
    pub raster_failures: Vec<CheckFailure>,
    pub vector_failures: Vec<CheckFailure>,
    /// Shortlist position of the published layout.
    pub winner_rank: usize,
}

/// The best candidate that passes the raster check and then `vector`, with the report.
/// `raster` gets the whole shortlist (best first) and returns one verdict per candidate, so a
/// caller may check independent groups in parallel. `Err` carries the shortlist and the
/// report when no shortlisted candidate passes both.
pub(crate) fn choose(
    mut candidates: Vec<Candidate>,
    raster: impl FnOnce(&[Candidate]) -> Vec<Result<(), String>>,
    mut vector: impl FnMut(&Candidate) -> Result<(), String>,
) -> Result<(Candidate, ValidationReport), (Vec<Candidate>, ValidationReport)> {
    candidates.sort_by(|a, b| a.key.cmp(&b.key));
    candidates.truncate(SHORTLIST_LEN);
    let mut report = ValidationReport {
        shortlisted: candidates.len(),
        ..ValidationReport::default()
    };
    let verdicts = raster(&candidates);
    assert_eq!(
        verdicts.len(),
        candidates.len(),
        "one raster verdict per candidate"
    );
    let mut passed = Vec::new();
    for (rank, (candidate, verdict)) in candidates.iter().zip(verdicts).enumerate() {
        match verdict {
            Ok(()) => passed.push(rank),
            Err(message) => report
                .raster_failures
                .push(failure(rank, candidate, message)),
        }
    }
    report.raster_passed = passed.len();
    for rank in passed {
        match vector(&candidates[rank]) {
            Ok(()) => {
                report.winner_rank = rank;
                log_failures(&report);
                return Ok((candidates.swap_remove(rank), report));
            }
            Err(message) => report
                .vector_failures
                .push(failure(rank, &candidates[rank], message)),
        }
    }
    log_failures(&report);
    Err((candidates, report))
}

fn failure(rank: usize, candidate: &Candidate, message: String) -> CheckFailure {
    CheckFailure {
        rank,
        member: candidate.key.tie.member,
        count: candidate.key.count,
        message,
    }
}

/// Every failed check is a bug signal (design §8.5): say so, whatever the outcome.
fn log_failures(report: &ValidationReport) {
    let raster = report.raster_failures.iter().map(|f| ("raster", f));
    let vector = report.vector_failures.iter().map(|f| ("vector", f));
    for (check, f) in raster.chain(vector) {
        eprintln!(
            "pack: shortlisted layout {} ({:?}, {} objects) failed the {check} check, next \
             candidate taken: {}",
            f.rank, f.member, f.count, f.message
        );
    }
}

/// Every message of a report, for an error when nothing passed.
pub(crate) fn failure_summary(report: &ValidationReport) -> String {
    let failures = report
        .raster_failures
        .iter()
        .chain(&report.vector_failures)
        .map(|f| format!("#{} {:?}: {}", f.rank, f.member, f.message))
        .collect::<Vec<_>>();
    format!(
        "no shortlisted layout passed validation ({} shortlisted): {}",
        report.shortlisted,
        failures.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(count: usize, area: u64, member: Member, detail: i64) -> RankKey {
        RankKey {
            count,
            robustness_quarter_mm: 0,
            canonical_clear: true,
            bbox_area_px: area,
            tie: TieKey {
                member,
                anchor: 0,
                detail: vec![detail],
            },
        }
    }

    fn candidate(count: usize, area: u64, detail: i64) -> Candidate {
        Candidate {
            key: key(count, area, Member::Sweep, detail),
            anchor: None,
            placements: Vec::new(),
        }
    }

    #[test]
    fn more_objects_win_then_robustness_then_the_smaller_box_then_the_tie_key() {
        let mut keys = vec![
            key(10, 50, Member::Rows, 0),
            key(11, 90, Member::Rows, 0),
            key(10, 40, Member::Sweep, 1),
            key(10, 40, Member::Sweep, 0),
        ];
        let mut robust = key(10, 90, Member::Rows, 0);
        robust.robustness_quarter_mm = 1;
        keys.push(robust.clone());
        keys.sort();
        assert_eq!(keys[0].count, 11);
        assert_eq!(keys[1], robust);
        assert_eq!(keys[2], key(10, 40, Member::Sweep, 0));
        assert_eq!(keys[3], key(10, 40, Member::Sweep, 1));
        assert_eq!(keys[4], key(10, 50, Member::Rows, 0));
    }

    #[test]
    fn a_failed_check_falls_back_to_the_next_candidate() {
        let candidates = vec![
            candidate(5, 10, 0),
            candidate(7, 10, 1),
            candidate(7, 10, 2),
        ];
        let raster = |shortlist: &[Candidate]| {
            let verdict = |c: &Candidate| match c.key.tie.detail[0] {
                1 => Err("raster".to_owned()),
                _ => Ok(()),
            };
            shortlist.iter().map(verdict).collect()
        };
        let (winner, report) = choose(candidates, raster, |c| match c.key.tie.detail[0] {
            2 => Err("vector".to_owned()),
            _ => Ok(()),
        })
        .unwrap();
        assert_eq!(winner.key.count, 5);
        assert_eq!(report.shortlisted, 3);
        assert_eq!(report.raster_passed, 2);
        assert_eq!(report.raster_failures[0].rank, 0);
        assert_eq!(report.vector_failures[0].rank, 1);
        assert_eq!(report.winner_rank, 2);
    }
}
