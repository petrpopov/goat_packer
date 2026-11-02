//! The behaviour switches between the four builds, one function each (design §3.1 version
//! matrix, spec 01 §7, spec 02 §2.10). Every switch cites where the behaviour was read, so a
//! reviewer can check it without reading the generator.

use super::{Family, Flavour};
use crate::paint::StateCoding;

/// What `support_critical_regions_only` keeps (spec 01 §3.7, design §3.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalOnly {
    /// B: cantilevers; on enforcer layers whole cantilever / sharp-tail clusters, the rest cut
    /// to the grown enforcers, big flats united.
    Bambu,
    /// O/R/S: cantilevers, enforced areas and sharp-tail supports.
    Orca,
}

/// How small overhang clusters are recognised (spec 01 §3.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmallOverhang {
    /// B: per-layer narrow width and area averages, then the 2.5w erosion bbox rule.
    Bambu,
    /// O/R/S: the outline eroded by w is narrower than 2w in X or Y.
    Orca,
}

/// Merge bucket size of the branch merging (spec 02 §4.3, nondeterminism S2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeBucket {
    /// 4 if ⌊(n+2)/4⌋ ≥ T (thread arena size), else 2.
    ThreadCount,
    /// R: always the T = 1 class.
    FixedFour,
}

/// Shape of a branch root standing on the bed (spec 03 §3.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootCap {
    Hemisphere,
    FlatFoot,
}

/// How the first-layer support expansion is applied (spec 03 §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirstLayerExpansion {
    /// B: −1 means an automatic 2 mm, grown in steps.
    AutoSteps,
    /// O/R/S: max(0, expansion − fine step).
    MinusFine,
}

/// `bed_exclude_area` encoding in the project (design §3.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExclusionEncoding {
    /// B: groups of four points, each a rectangle.
    Rectangles,
    /// Orca builds: one polygon.
    Polygon,
}

/// The GUI's toolpath-vs-exclusion check (design §3.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuiExclusionCheck {
    /// B: every segment, exact.
    EverySegment,
    /// S: convex hull of all toolpaths against the exclusion boxes.
    ConvexHull,
    /// O/R: none.
    None,
}

/// The prime tower's chamfer rings above the first layer (spec 04 §4.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TowerChamfer {
    /// B: always on, at most 3 mm, one loop narrower per layer.
    Always,
    /// S: with `prime_tower_brim_chamfer`, up to its max width.
    Setting,
    /// O/R: L0 brim only.
    Never,
}

/// Who moves the tower anchor and when (spec 04 §6, design §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnchorClamp {
    /// B: the GUI clamps to [15, bed − estimate − 15] when the project is opened.
    GuiOnOpen,
    /// O: kept on open and on CLI slices.
    Never,
    /// R: CLI and GUI clamp on every slice, snapping within 5 mm.
    EverySlice,
    /// S: the GUI clamps with 1 + `prime_tower_brim_width` on every scene reload.
    EveryReload,
}

impl Flavour {
    fn is_bambu(self) -> bool {
        self.family() == Family::Bambu
    }

    /// Branch distance / angle / diameter keys: B the plain keys, Orca builds `*_organic`
    /// (B `TreeSupportCommon.hpp:82-83`, O `:85-86`).
    pub fn uses_organic_keys(self) -> bool {
        !self.is_bambu()
    }

    /// B fixes the slow angle (25°), tip diameter (0.8 mm) and top rate (30 %) in code
    /// (B `TreeSupportCommon.hpp:80-89`); Orca builds read the settings.
    pub fn fixed_tree_constants(self) -> bool {
        self.is_bambu()
    }

    pub fn critical_only(self) -> CriticalOnly {
        if self.is_bambu() {
            CriticalOnly::Bambu
        } else {
            CriticalOnly::Orca
        }
    }

    /// B also calls an overhang with a ring that has no vertex near the layer below a
    /// cantilever (B `TreeSupport.cpp:1009-1050`).
    pub fn floating_ring_is_cantilever(self) -> bool {
        self.is_bambu()
    }

    pub fn merge_bucket(self) -> MergeBucket {
        if self == Flavour::Orca250 {
            MergeBucket::FixedFour
        } else {
            MergeBucket::ThreadCount
        }
    }

    /// R fixed the off-by-one that keeps blocker volumes out of the collision volumes
    /// (`TreeModelVolumes.cpp` R:525 vs O:472).
    pub fn blockers_are_obstacles(self) -> bool {
        self == Flavour::Orca250
    }

    /// B routes branches around the outside of the bed and its exclusions, in the slicing
    /// object's frame (B `TreeModelVolumes.cpp:67-88,134`; spec 02 §2.8).
    pub fn bed_is_obstacle(self) -> bool {
        self.is_bambu()
    }

    /// O `TreeSupport3D.cpp:2943-2957`: flat feet in O and R.
    pub fn root_cap(self) -> RootCap {
        match self {
            Flavour::Orca242 | Flavour::Orca250 => RootCap::FlatFoot,
            Flavour::Bambu0208 | Flavour::Snapmaker240 => RootCap::Hemisphere,
        }
    }

    /// O/R constrain organic smoothing to linked layers (O `TreeSupport3D.cpp:3156-3202`).
    pub fn smoothing_anchored(self) -> bool {
        matches!(self, Flavour::Orca242 | Flavour::Orca250)
    }

    /// B prunes each tree to its main connected part (B `TreeSupport3D.cpp:4933-4960`).
    pub fn keeps_main_tree_only(self) -> bool {
        self.is_bambu()
    }

    pub fn first_layer_expansion(self) -> FirstLayerExpansion {
        if self.is_bambu() {
            FirstLayerExpansion::AutoSteps
        } else {
            FirstLayerExpansion::MinusFine
        }
    }

    /// R: per-nozzle support exclusion volumes and `bed_exclude_volumes`.
    pub fn has_exclusion_volumes(self) -> bool {
        self == Flavour::Orca250
    }

    /// The auto-brim thermal term: B multiplies it by 0 (B `Brim.cpp:798`).
    pub fn brim_thermal_term(self) -> bool {
        !self.is_bambu()
    }

    /// The extra 5 mm brim for thin islands (O `Brim.cpp:514-520`; B's guard is always false).
    pub fn thin_island_extra_brim(self) -> bool {
        !self.is_bambu()
    }

    /// The no-brim set is closed by one brim spacing in B and S (spec 03 §5.3).
    pub fn closes_no_brim_set(self) -> bool {
        matches!(self, Flavour::Bambu0208 | Flavour::Snapmaker240)
    }

    /// Arc thresholds of the support-base connector are process-static in B/O/S (S9) and
    /// per call in R (R `Fill/FillBase.cpp:2696-2701`; spec 01 §5.5.4).
    pub fn static_arc_thresholds(self) -> bool {
        self != Flavour::Orca250
    }

    pub fn exclusion_encoding(self) -> ExclusionEncoding {
        if self.is_bambu() {
            ExclusionEncoding::Rectangles
        } else {
            ExclusionEncoding::Polygon
        }
    }

    pub fn gui_exclusion_check(self) -> GuiExclusionCheck {
        match self {
            Flavour::Bambu0208 => GuiExclusionCheck::EverySegment,
            Flavour::Snapmaker240 => GuiExclusionCheck::ConvexHull,
            Flavour::Orca242 | Flavour::Orca250 => GuiExclusionCheck::None,
        }
    }

    /// S warns when a part's box comes within 3.5 mm of the bed edge (strict <;
    /// S `slic3r/GUI/3DScene.cpp:1381-1401`).
    pub fn gui_body_edge_band_mm(self) -> Option<f64> {
        (self == Flavour::Snapmaker240).then_some(3.5)
    }

    /// The CLI's "toolpath outside" error code (object box + 2 mm,
    /// `GCode/GCodeProcessor.cpp:1845`); S has no such check.
    pub fn cli_outside_error_code(self) -> Option<i32> {
        match self {
            Flavour::Bambu0208 => Some(-104),
            Flavour::Orca242 | Flavour::Orca250 => Some(-102),
            Flavour::Snapmaker240 => None,
        }
    }

    /// The ConflictChecker's collinear-overlap rule (absent in S).
    pub fn conflict_checks_collinear(self) -> bool {
        self != Flavour::Snapmaker240
    }

    pub fn tower_chamfer(self) -> TowerChamfer {
        match self {
            Flavour::Bambu0208 => TowerChamfer::Always,
            Flavour::Snapmaker240 => TowerChamfer::Setting,
            Flavour::Orca242 | Flavour::Orca250 => TowerChamfer::Never,
        }
    }

    pub fn anchor_clamp(self) -> AnchorClamp {
        match self {
            Flavour::Bambu0208 => AnchorClamp::GuiOnOpen,
            Flavour::Orca242 => AnchorClamp::Never,
            Flavour::Orca250 => AnchorClamp::EverySlice,
            Flavour::Snapmaker240 => AnchorClamp::EveryReload,
        }
    }

    /// B cuts micro back-tracks (≤ 8 edges, ≤ 2 µm) out of section loops before closing
    /// (B `TriangleMeshSlicer.cpp:1739-1798`; spec 01 §1.3 step 4).
    pub fn removes_micro_backtracks(self) -> bool {
        self.is_bambu()
    }

    /// When printable parts sharing a layer have disjoint XY boxes, O/R/S keep only the first
    /// part's slice (O `PrintObjectSlice.cpp:288-330`, spec 01 §1.4, Q5); B keeps all.
    pub fn keeps_disjoint_parts(self) -> bool {
        self.is_bambu()
    }

    /// Elephant-foot compensation spans `elefant_foot_compensation_layers` in Orca builds
    /// and layer 0 only in B (spec 01 §1.5).
    pub fn elephant_foot_layer_setting(self) -> bool {
        !self.is_bambu()
    }

    /// B reads the raw `line_width` number for the extrudable filter; Orca resolves it
    /// against nozzle 0 (spec 01 §1.6).
    pub fn raw_line_width(self) -> bool {
        self.is_bambu()
    }

    /// Paint states of 18 and above: B chains codes of 15 (spec 01 §2.2).
    pub fn paint_state_coding(self) -> StateCoding {
        if self.is_bambu() {
            StateCoding::Bambu
        } else {
            StateCoding::Orca
        }
    }

    /// B removes grown blockers from the plain overhang and adds the enforced overhang
    /// before cantilever and sharp-tail detection (spec 01 §3.2 steps 1-2).
    pub fn paint_in_detection(self) -> bool {
        self.is_bambu()
    }

    /// Layer-0 islands seed sharp tails when their box is ≤ 6 mm in both axes (B) or in at
    /// least one axis (O/R/S) (spec 01 §3.2, §3.3).
    pub fn first_layer_seed_needs_both_axes(self) -> bool {
        self.is_bambu()
    }

    /// B adds the area, box and two-layers-below conditions to sharp-tail seeds (01 §3.2.4).
    pub fn strict_tail_seeds(self) -> bool {
        self.is_bambu()
    }

    /// B keeps every member island of a cluster per layer; O/R/S only the first (01 §3.5).
    pub fn clusters_keep_all_members(self) -> bool {
        self.is_bambu()
    }

    pub fn small_overhang(self) -> SmallOverhang {
        if self.is_bambu() {
            SmallOverhang::Bambu
        } else {
            SmallOverhang::Orca
        }
    }

    /// B grows every hand-off polygon with the sharp-tail bit by 0.2 mm; O/R/S only the
    /// sharp-tail supports (spec 01 §3.8).
    pub fn grows_sharp_tail_clusters(self) -> bool {
        self.is_bambu()
    }

    /// R simplifies the united collision by S(resolution) (spec 02 §2.3); the others leave it.
    pub fn simplifies_collision(self) -> bool {
        self == Flavour::Orca250
    }

    /// B asks for strictly simple output in S(τ) (spec 02 §1.1).
    pub fn strictly_simple(self) -> bool {
        self.is_bambu()
    }

    /// O and R support Z anti-aliasing (a moved slice plane); S and B do not (01 §1.2 rule 4).
    pub fn has_z_anti_aliasing(self) -> bool {
        matches!(self, Flavour::Orca242 | Flavour::Orca250)
    }

    /// Snapmaker's mixed-filament gradient and Z-dithering modes rewrite the layer ranges
    /// (S `PrintObject.cpp:3838-3900`, spec 01 §1.2.1).
    pub fn has_dithering_modes(self) -> bool {
        self == Flavour::Snapmaker240
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_matrix_rows_that_stage_a_reads() {
        use Flavour::*;
        assert_eq!(
            Flavour::ALL.map(Flavour::bed_is_obstacle),
            [true, false, false, false]
        );
        assert_eq!(
            Flavour::ALL.map(Flavour::merge_bucket),
            [
                MergeBucket::ThreadCount,
                MergeBucket::ThreadCount,
                MergeBucket::FixedFour,
                MergeBucket::ThreadCount
            ]
        );
        assert_eq!(Orca242.root_cap(), RootCap::FlatFoot);
        assert_eq!(Snapmaker240.root_cap(), RootCap::Hemisphere);
        assert!(!Orca250.static_arc_thresholds());
        assert!(Snapmaker240.static_arc_thresholds());
        assert!(Orca250.simplifies_collision() && !Orca242.simplifies_collision());
        assert!(Bambu0208.strictly_simple() && !Snapmaker240.strictly_simple());
    }
}
