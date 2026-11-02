//! What the colored-3MF pipeline knows about a printer and its slicer: the profile
//! (`profile`), the CLI workarounds applied to a project copy (`cli_settings`) and the prime
//! tower measured from G-code (`prime_tower`).

use std::error::Error;

pub mod bed_rules;
mod cli_settings;
mod prime_tower;
mod profile;

pub type ColoredPackerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub use cli_settings::{apply_cli_settings_workarounds, normalize_flush_volumes_matrix};
pub use prime_tower::{PrimeTowerBounds, parse_prime_tower_bounds, tower_repack_required};
pub use profile::{PrinterProfile, detect_slicer_kind_from_printer_model, load_printer_profile};
