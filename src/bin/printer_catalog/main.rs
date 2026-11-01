//! Developer tool: turns OrcaSlicer's JSON machine presets (pinned checkout) into draft
//! `profiles/*.json` for rectangular-bed printers (spec
//! `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md`).
//!
//!   printer_catalog extract --orca <checkout>/resources/profiles --tag v2.4.2
//!     [--vendor Name]... [--profiles profiles/] --out <dir>
//!   printer_catalog qualify --draft <id.json> --orca <checkout>/resources/profiles
//!     --work <dir> [--out profiles/] [--report <file.md>] [--stage export|pack|all]
//!     [--orca-slicer /opt/slicers/orca/AppRun] [--slicer <path, default: draft slicer_path>]
//!     [--packer colored_3mf_packer] [--verify verify_outputs]
//!   printer_catalog relabel [--profiles profiles/]
//!   printer_catalog annotate --orca <checkout>/resources/profiles [--profiles profiles/]
//!
//! `vendor` stays the Orca vendor folder (`BBL`): qualify and STL builds look presets up by it.
//! The picker groups by `vendor_display` (`Bambu Lab`), the leading words every model of the
//! folder shares; `relabel` recomputes it for existing catalog profiles without Orca files.
//!
//! `qualify` runs in the runner image (`deploy/catalog-qualify.sh`): the probe 3MF is always
//! exported by the Orca CLI, slicing and packing use the draft's slicer, so a Bambu printer
//! runs `--stage export` in runner-orca and `--stage pack` in runner-bambu.
//!
//! `annotate` (and `qualify` for new profiles): a purge done by a firmware macro called from the
//! start G-code (`PRINT_START`, `START_PRINT`, …) never reaches the sliced G-code, so
//! `purge_detect` cannot see it; such a profile without detected `purge_lines` gets a
//! [`annotate::MACRO_PURGE_MARGIN_MM`] edge margin and a `source.notes` entry.
//!
//! Only numbers and our own traced polygon leave Orca's files (AGPL, never copied). Draft JSON
//! keys are alphabetical: serde_json is built without `preserve_order`, so `Map` is a BTreeMap.
//!
//! One module per command: `extract`, `qualify`, `relabel`, `annotate`; `util` holds what they
//! share.

mod annotate;
mod extract;
mod qualify;
mod relabel;
mod util;

use std::error::Error;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("extract") => extract::extract(args.collect()),
        Some("qualify") => qualify::qualify(args.collect()),
        Some("relabel") => relabel::relabel(args.collect()),
        Some("annotate") => annotate::annotate(args.collect()),
        _ => Err("usage: printer_catalog extract|qualify|relabel|annotate ...".into()),
    }
}
