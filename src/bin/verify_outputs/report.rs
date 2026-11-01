//! The JSON report of a passed verification, and number formatting shared with its messages.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct Report {
    pub project: String,
    pub printer_model: Option<String>,
    pub count: usize,
    pub minimum_exact_gap_mm: Value,
    pub required_edge_margin_mm: f64,
    pub minimum_edge_clearance_mm: f64,
    pub required_purge_gap_mm: Value,
    pub minimum_purge_gap_mm: Value,
    pub all_inside_printable_area: bool,
    pub exclusion_clear: bool,
    pub zip_crc_ok: bool,
    pub metadata_object_counts_match: bool,
    pub embedded_mesh_transform_ok: bool,
}

/// Python's json.dumps happily emits `Infinity`; strict JSON (and serde_json) cannot, so an
/// empty layout's infinite gap serializes as null instead.
pub fn json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Floats in assertion messages formatted like Python's str(float) (integral values keep ".0").
pub fn py_f64(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}
