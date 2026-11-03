//! Command line of `verify_outputs`.

use std::path::PathBuf;

use goat_packer::Rect;

use crate::rules::Foreign;

pub struct Args {
    pub stl: PathBuf,
    pub placements: PathBuf,
    pub project: PathBuf,
    pub minimum_gap: f64,
    pub edge_margin: f64,
    pub purge_lines: Vec<[f64; 5]>,
    pub purge_gap: f64,
    pub custom_keepouts: Vec<Rect>,
    /// `--foreign-base-gap` / `--foreign-band-gap`: the packer's `ForeignGaps` the layout was
    /// packed with (defaults = the packer's defaults).
    pub foreign: Foreign,
    pub report: Option<PathBuf>,
}

fn parse_flag_f64(option: &str, raw: String, name: &str) -> Result<f64, String> {
    raw.parse::<f64>()
        .map_err(|_| format!("{option}: {name} must be a number, got {raw:?}"))
}

/// Exactly `count` comma-separated numbers, or `usage` as the error.
fn comma_separated_numbers(raw: &str, count: usize, usage: &str) -> Result<Vec<f64>, String> {
    let parts = raw
        .split(',')
        .map(|part| part.parse::<f64>())
        .collect::<Result<Vec<f64>, _>>()
        .map_err(|_| usage.to_owned())?;
    if parts.len() != count {
        return Err(usage.to_owned());
    }
    Ok(parts)
}

pub fn parse_args_from<I: Iterator<Item = String>>(mut args: I) -> Result<Args, String> {
    let mut positionals = Vec::new();
    let mut minimum_gap = 0.5;
    let mut edge_margin = 0.0;
    let mut purge_lines = Vec::new();
    let mut purge_gap = 0.5;
    let mut custom_keepouts = Vec::new();
    let mut report = None;
    let mut foreign = Foreign::default();
    while let Some(option) = args.next() {
        let Some(flag) = option.strip_prefix("--") else {
            positionals.push(option);
            continue;
        };
        let (flag, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (flag, None),
        };
        // The Python accepted `--flag value` only; both spellings parse here.
        let mut value = || -> Result<String, String> {
            match inline {
                Some(value) => Ok(value.to_owned()),
                None => args
                    .next()
                    .ok_or_else(|| format!("{option}: expected one argument")),
            }
        };
        match flag {
            "minimum-gap" => minimum_gap = parse_flag_f64(&option, value()?, "minimum gap")?,
            "edge-margin" => edge_margin = parse_flag_f64(&option, value()?, "edge margin")?,
            "foreign-base-gap" => {
                foreign.base_mm = parse_flag_f64(&option, value()?, "foreign base gap")?;
            }
            "foreign-band-gap" => {
                foreign.band_mm = parse_flag_f64(&option, value()?, "foreign band gap")?;
            }
            "purge-gap" => purge_gap = parse_flag_f64(&option, value()?, "purge gap")?,
            "purge-line" => {
                let parts = comma_separated_numbers(
                    &value()?,
                    5,
                    "--purge-line must be x0,y0,x1,y1,width",
                )?;
                purge_lines.push(parts.try_into().expect("five numbers"));
            }
            "keepout" => {
                let parts = comma_separated_numbers(
                    &value()?,
                    4,
                    "--keepout must be min_x,min_y,max_x,max_y",
                )?;
                custom_keepouts.push(Rect::new(parts[0], parts[1], parts[2], parts[3]));
            }
            "report" => report = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown option: {option}")),
        }
    }
    if positionals.len() != 3 {
        return Err(
            "usage: verify_outputs <stl> <placements> <project> [--minimum-gap F] \
             [--edge-margin F] [--purge-line x0,y0,x1,y1,w]... [--purge-gap F] \
             [--keepout min_x,min_y,max_x,max_y]... [--foreign-base-gap F] \
             [--foreign-band-gap F] [--report PATH]"
                .to_owned(),
        );
    }
    let mut positionals = positionals.into_iter();
    let stl = positionals.next().expect("three positionals");
    let placements = positionals.next().expect("three positionals");
    let project = positionals.next().expect("three positionals");
    Ok(Args {
        stl: PathBuf::from(stl),
        placements: PathBuf::from(placements),
        project: PathBuf::from(project),
        minimum_gap,
        edge_margin,
        purge_lines,
        purge_gap,
        custom_keepouts,
        foreign,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_both_flag_spellings() {
        let parsed = parse_args_from(
            [
                "stl.stl",
                "placements.json",
                "project.3mf",
                "--minimum-gap",
                "0.25",
                "--edge-margin=2",
                "--purge-line",
                "0,0,200,0,4",
                "--purge-line=-10,5,300,5,8",
                "--purge-gap=1",
                "--keepout",
                "10,10,20,20",
                "--foreign-base-gap=0.25",
                "--foreign-band-gap",
                "0.75",
                "--report=out.json",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();
        assert_eq!(parsed.stl, PathBuf::from("stl.stl"));
        assert_eq!(parsed.placements, PathBuf::from("placements.json"));
        assert_eq!(parsed.project, PathBuf::from("project.3mf"));
        assert_eq!(parsed.minimum_gap, 0.25);
        assert_eq!(parsed.edge_margin, 2.0);
        assert_eq!(
            parsed.purge_lines,
            vec![[0.0, 0.0, 200.0, 0.0, 4.0], [-10.0, 5.0, 300.0, 5.0, 8.0]]
        );
        assert_eq!(parsed.purge_gap, 1.0);
        assert_eq!(
            parsed.custom_keepouts,
            vec![Rect::new(10.0, 10.0, 20.0, 20.0)]
        );
        assert_eq!(parsed.foreign, Foreign { base_mm: 0.25, band_mm: 0.75 });
        assert_eq!(parsed.report, Some(PathBuf::from("out.json")));
    }
}
