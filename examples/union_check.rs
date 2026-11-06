//! Compares the union drawn at once (`ensemble::run_union`) with the union of the members'
//! own realizations: `union_check <3mf> [rotation_deg]`. Per kind and band: both areas, the
//! old union's area outside the new one and how far it reaches (should be ~0: the new union
//! only grows), and the timings.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::object_print::{Loops, Margins};
use goat_packer::support_model::ensemble::{distinct, run_members_with, run_union};
use goat_packer::support_model::envelope::envelope;
use goat_packer::support_model::exceedance::{area_mm2, exceedance};
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[0]);
    let rotation: f64 = args.get(1).map_or(0.0, |text| text.parse().unwrap());
    let mesh = load_object_mesh(path).unwrap().rotated(rotation);
    let config = ProjectConfig::load(path).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(None, config.application.as_deref(), None)
        .unwrap()
        .flavour;
    let orders = flavour.default_orders();
    let overlaps = flavour.overlap_readings();
    let stage = || {
        stage_a(&StageAInput {
            mesh: &mesh,
            settings: &settings,
            flavour,
            bed: None,
        })
        .unwrap()
    };
    let started = Instant::now();
    let old_stage = stage();
    let runs = run_members_with(&old_stage, orders, overlaps);
    let old = envelope(&distinct(&runs), Margins::default());
    let old_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let new_stage = stage();
    let (_, realizations) = run_union(&new_stage, orders, overlaps);
    let new = envelope(&realizations.iter().collect::<Vec<_>>(), Margins::default());
    println!(
        "{flavour:?} rotation {rotation}: members {:.1} s, union {:.1} s ({} realizations)",
        old_seconds,
        started.elapsed().as_secs_f64(),
        realizations.len()
    );
    let report = |name: String, old: &Loops, new: &Loops| {
        let missing = exceedance(old, new);
        let extra = exceedance(new, old);
        println!(
            "{name:<14} old {:>8.1} new {:>8.1} mm²  old outside new {:>6.2} mm² / {:.3} mm  new outside old {:>6.2} mm² / {:.3} mm",
            area_mm2(old),
            area_mm2(new),
            missing.outside_mm2,
            missing.max_mm,
            extra.outside_mm2,
            extra.max_mm
        );
    };
    report("base".into(), &old.base, &new.base);
    let mut worst: Vec<(f64, usize)> = distinct(&runs)
        .iter()
        .enumerate()
        .map(|(index, member)| (exceedance(&member.base, &new.base).max_mm, index))
        .collect();
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    let firsts: Vec<&goat_packer::support_model::ensemble::MemberRun> = runs.iter().filter(|r| r.realization.is_some()).collect();
    for (miss, index) in worst.iter().take(3) {
        let run = firsts.iter().find(|r| std::ptr::eq(r.realization.as_ref().unwrap(), distinct(&runs)[*index])).unwrap();
        println!("base miss {miss:.3} mm: {:?} {:?} {:?} T {}", run.family, run.order, run.overlap, run.threads);
    }
    for band in 0..old.above_base.len().max(new.above_base.len()) {
        let empty = Loops::new();
        report(
            format!("above {band}"),
            old.above_base.get(band).unwrap_or(&empty),
            new.above_base.get(band).unwrap_or(&empty),
        );
        report(
            format!("interface {band}"),
            old.interface.get(band).unwrap_or(&empty),
            new.interface.get(band).unwrap_or(&empty),
        );
    }
}
