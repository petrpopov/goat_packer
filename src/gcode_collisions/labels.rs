//! How sliced plates name their objects. Labels come in four spellings, and a plate is read by
//! exactly one of them, the most explicit it contains (`LabelScheme`):
//! - BambuStudio, several objects: `; start printing object, unique label id: N` ..
//!   `; stop printing object, unique label id: N`;
//! - OrcaSlicer: `; printing object NAME id:N copy M` .. `; stop printing object ...`;
//! - Klipper: `EXCLUDE_OBJECT_START NAME=...` .. `EXCLUDE_OBJECT_END` (Orca writes it next to
//!   its comment under another name, so reading both made one object two labels);
//! - BambuStudio, a single object: only `; OBJECT_ID: N`. It writes no start/stop labels for a
//!   one-object plate (every P1S/A2L probe), which made the probe's support learning fail with
//!   "incomplete or ambiguous sample object coverage". On multi-object plates the same marker
//!   announces the NEXT object a few lines before the current one stops, so it is only trusted
//!   when no explicit label exists.

use std::collections::BTreeSet;

/// How a plate labels its objects, most explicit first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum LabelScheme {
    BambuStart,
    OrcaComment,
    Klipper,
    BambuObjectId,
    Unlabelled,
}

const BAMBU_START: &str = "; start printing object, unique label id:";
const ORCA_START: &str = "; printing object ";
const LABEL_STOP: &str = "; stop printing object";
const KLIPPER_START: &str = "EXCLUDE_OBJECT_START";
const KLIPPER_STOP: &str = "EXCLUDE_OBJECT_END";
const BAMBU_OBJECT_ID: &str = "; OBJECT_ID:";

/// What a label line of the plate's scheme says.
pub(super) enum LabelLine<'a> {
    Start(&'a str),
    Stop,
}

/// The plate's label scheme, plus the distinct `; OBJECT_ID:` values (for the one-object rule).
pub(super) fn label_scheme(gcode: &[u8]) -> (LabelScheme, BTreeSet<String>) {
    let mut scheme = LabelScheme::Unlabelled;
    let mut object_ids = BTreeSet::new();
    for raw in gcode.split(|&byte| byte == b'\n') {
        // Every label line starts with `;` or `E` (EXCLUDE_OBJECT_...); skip the moves cheaply.
        let first = raw.iter().position(|byte| !byte.is_ascii_whitespace());
        if !matches!(first.map(|at| raw[at]), Some(b';' | b'E')) {
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        let line = line.trim();
        let found = if line.starts_with(BAMBU_START) {
            LabelScheme::BambuStart
        } else if line.starts_with(ORCA_START) {
            LabelScheme::OrcaComment
        } else if line.starts_with(KLIPPER_START) {
            LabelScheme::Klipper
        } else if let Some(id) = line.strip_prefix(BAMBU_OBJECT_ID) {
            object_ids.insert(id.trim().to_owned());
            LabelScheme::BambuObjectId
        } else {
            continue;
        };
        scheme = scheme.min(found);
    }
    (scheme, object_ids)
}

/// `line` read as a label of `scheme`; labels of the other schemes are ordinary comments.
pub(super) fn label_line(scheme: LabelScheme, line: &str) -> Option<LabelLine<'_>> {
    let start = match scheme {
        LabelScheme::BambuStart => line.strip_prefix(BAMBU_START).map(str::trim),
        LabelScheme::OrcaComment => line.strip_prefix(ORCA_START).map(str::trim),
        LabelScheme::Klipper => line.strip_prefix(KLIPPER_START).map(klipper_name),
        LabelScheme::BambuObjectId => line.strip_prefix(BAMBU_OBJECT_ID).map(str::trim),
        LabelScheme::Unlabelled => None,
    };
    if let Some(label) = start {
        return Some(LabelLine::Start(label));
    }
    let stop = match scheme {
        LabelScheme::BambuStart | LabelScheme::OrcaComment => line.starts_with(LABEL_STOP),
        LabelScheme::Klipper => line.starts_with(KLIPPER_STOP),
        LabelScheme::BambuObjectId | LabelScheme::Unlabelled => false,
    };
    stop.then_some(LabelLine::Stop)
}

/// The object name of an `EXCLUDE_OBJECT_START NAME=...` line (the whole rest without `NAME=`).
fn klipper_name(rest: &str) -> &str {
    rest.split_whitespace()
        .find_map(|token| token.strip_prefix("NAME="))
        .unwrap_or(rest.trim())
}
