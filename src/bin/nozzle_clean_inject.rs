//! Injects the periodic nozzle-clean macro into an existing BambuStudio/Orca project 3MF —
//! for projects that did not come out of `colored_3mf_packer` (e.g. hand-made single-object
//! plates). The wipe body comes from the printer profile; this tool only picks when it runs:
//!
//!   nozzle_clean_inject --source in.3mf --output out.3mf \
//!     --profile profiles/p1s.json --nozzle-clean from-z=6.6
//!
//! `--output` may equal `--source` (the file is fully read before being rewritten).

use std::{error::Error, path::PathBuf};

use goat_packer::{
    colored_packer::load_printer_profile,
    nozzle_clean::{NozzleCleanSpec, inject_nozzle_clean},
};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let mut args = std::env::args().skip(1);
    let mut source = None;
    let mut output = None;
    let mut profile_path = None;
    let mut spec_text = None;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--source" => source = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--nozzle-clean" => spec_text = Some(value),
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let source = source.ok_or("--source is required")?;
    let output = output.ok_or("--output is required")?;
    let profile_path = profile_path.ok_or("--profile is required")?;
    let spec_text = spec_text.ok_or("--nozzle-clean is required")?;
    let spec = NozzleCleanSpec::parse(&spec_text)?;
    let profile = load_printer_profile(&profile_path)?;

    inject_nozzle_clean(&profile, &source, &output, &spec)?;

    let describe = match spec {
        NozzleCleanSpec::Global { every } => {
            format!("every {every} layers, whole print")
        }
        NozzleCleanSpec::FromZ { z_mm, every } => {
            let noun = if every == 1 { "layer" } else { "layers" };
            format!("every {every} {noun} at/above z {z_mm} mm")
        }
    };
    println!(
        "injected nozzle clean ({describe}) into layer_change_gcode of {}",
        output.display()
    );
    Ok(())
}
