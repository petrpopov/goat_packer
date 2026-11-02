//! The `qualify` command line.

use std::path::PathBuf;

use crate::AppResult;

pub const DEFAULT_ORCA: &str = "/opt/slicers/orca/AppRun";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stage {
    /// Flatten presets and export `probe.3mf` (needs the Orca CLI: runner-orca image).
    Export,
    /// Probe slice, purge detection, pack + verify, write the profile (needs the draft's
    /// slicer: runner-bambu for Bambu printers).
    Pack,
    All,
}

#[derive(Debug)]
pub struct QualifyArgs {
    pub draft: PathBuf,
    pub orca: Option<PathBuf>,
    pub orca_slicer: PathBuf,
    pub slicer: Option<PathBuf>,
    pub packer: PathBuf,
    pub verify: PathBuf,
    pub work: PathBuf,
    pub out: PathBuf,
    pub report: Option<PathBuf>,
    pub stage: Stage,
}

pub fn parse_qualify_args(raw: Vec<String>) -> AppResult<QualifyArgs> {
    let mut args = raw.into_iter();
    let (mut draft, mut orca, mut slicer, mut work, mut report) = (None, None, None, None, None);
    let mut orca_slicer = PathBuf::from(DEFAULT_ORCA);
    let mut packer = PathBuf::from("colored_3mf_packer");
    let mut verify = PathBuf::from("verify_outputs");
    let mut out = PathBuf::from("profiles");
    let mut stage = Stage::All;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--draft" => draft = Some(PathBuf::from(value)),
            "--orca" => orca = Some(PathBuf::from(value)),
            "--orca-slicer" => orca_slicer = PathBuf::from(value),
            "--slicer" => slicer = Some(PathBuf::from(value)),
            "--packer" => packer = PathBuf::from(value),
            "--verify" => verify = PathBuf::from(value),
            "--work" => work = Some(PathBuf::from(value)),
            "--out" => out = PathBuf::from(value),
            "--report" => report = Some(PathBuf::from(value)),
            "--stage" => {
                stage = match value.as_str() {
                    "export" => Stage::Export,
                    "pack" => Stage::Pack,
                    "all" => Stage::All,
                    other => return Err(format!("unknown stage {other} (export|pack|all)").into()),
                }
            }
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let args = QualifyArgs {
        draft: draft.ok_or("missing --draft")?,
        orca,
        orca_slicer,
        slicer,
        packer,
        verify,
        work: work.ok_or("missing --work")?,
        out,
        report,
        stage,
    };
    if args.stage != Stage::Pack && args.orca.is_none() {
        return Err("missing --orca (needed to export the probe)".into());
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualify_args_need_orca_except_for_the_pack_stage() {
        let args = |list: &[&str]| parse_qualify_args(list.iter().map(|s| s.to_string()).collect());
        let pack = args(&["--draft", "d.json", "--work", "w", "--stage", "pack"]).unwrap();
        assert_eq!(pack.stage, Stage::Pack);
        assert_eq!(pack.orca_slicer, PathBuf::from(DEFAULT_ORCA));
        assert!(pack.slicer.is_none());
        assert!(args(&["--draft", "d.json", "--work", "w"]).is_err());
        let all = args(&[
            "--draft", "d.json", "--work", "w", "--orca", "o", "--slicer", "/s",
        ])
        .unwrap();
        assert_eq!(
            (all.stage, all.slicer),
            (Stage::All, Some(PathBuf::from("/s")))
        );
        assert!(args(&["--draft", "d.json", "--work", "w", "--stage", "nope"]).is_err());
    }
}
