//! Independent exact-geometry and 3MF integrity checks for packed projects.
//!
//! Rust port of `verify_outputs.py` (contract item 9 in
//! `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md`). This binary is the
//! project's independent second opinion: it shares no packing code — only the footprint/geometry
//! types from the library — so a packing bug cannot hide from the checks that gate a print.
//!
//! CLI (mirrors the Python; `--flag value` and `--flag=value` both accepted):
//!
//! ```text
//! verify_outputs <stl> <placements> <project>
//!                [--minimum-gap F] [--edge-margin F]
//!                [--purge-line x0,y0,x1,y1,w]... [--purge-gap F]
//!                [--keepout min_x,min_y,max_x,max_y]... [--report PATH]
//! ```
//!
//! On success the JSON report goes to stdout (and to `--report`). On the first failed check the
//! process exits 1 with an `AssertionError: ...` message carrying the same identifying prefix as
//! the Python verifier.
//!
//! Modules: `args` (command line), `checks` (the checks, in run order), `project` (reading the
//! 3MF), `matrix` (transforms), `rules` (its own copy of the packer's permission matrix and
//! pair table), `stl` (the collision STL), `report` (the output).

mod args;
mod checks;
mod matrix;
mod project;
mod report;
mod rules;
mod stl;

use std::fs;

use args::parse_args_from;
use checks::verify;

/// A verifier failure. `Assertion` is a failed check (the Python raised `AssertionError` with the
/// same message); `Error` is a tooling failure (the Python crashed with a different exception).
/// Both exit non-zero; only the stderr prefix differs.
#[derive(Debug)]
pub enum Failure {
    Assertion(String),
    Error(String),
}

pub fn assertion(message: impl Into<String>) -> Failure {
    Failure::Assertion(message.into())
}

fn main() {
    let args = match parse_args_from(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(usage) => {
            eprintln!("error: {usage}");
            std::process::exit(2);
        }
    };
    match verify(&args) {
        Ok(report) => {
            let text = serde_json::to_string_pretty(&report).expect("report serializes");
            println!("{text}");
            if let Some(path) = &args.report
                && let Err(error) = fs::write(path, text + "\n")
            {
                eprintln!("error: failed to write report {}: {error}", path.display());
                std::process::exit(1);
            }
        }
        Err(Failure::Assertion(message)) => {
            eprintln!("AssertionError: {message}");
            std::process::exit(1);
        }
        Err(Failure::Error(message)) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
}
