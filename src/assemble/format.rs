//! Number and identity formatting shared by every generated 3MF part.

use sha1::{Digest, Sha1};

/// The packed clones take object ids 2, 3, … — the source's resources keep id 1 and each
/// clone follows. `rebuild_core`, `rewrite_settings_object` and the layer-heights rewrite all
/// derive their ids from this so the metadata and the model can never disagree.
pub(super) fn clone_object_id(index: usize) -> usize {
    index + 2
}

/// Formats a float exactly like the Python `_number`: near-zero collapses to "0", near-integers
/// print without a decimal point, everything else gets 10 decimals with trailing zeros (and a
/// trailing dot, were one to survive) stripped. The persisted XML/JSON must not drift from the
/// Python artifact's formatting, hence the explicit thresholds.
pub(super) fn number(value: f64) -> String {
    if value.abs() < 1e-10 {
        return "0".to_owned();
    }
    let nearest = value.round();
    if (value - nearest).abs() < 1e-10 {
        let as_i64 = nearest as i64;
        if as_i64 as f64 == nearest {
            return as_i64.to_string();
        }
        // Beyond i64 the Python prints an exact big integer; unreachable for millimeter
        // coordinates, so the closest fixed-point form is an acceptable fallback.
        return format!("{nearest:.0}");
    }
    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Deterministic `uuid5(NAMESPACE_URL, "goat4-contour-packing/{kind}/{index}")` — the same
/// identities the Python emitted via `uuid.uuid5`, so re-runs produce identical projects and
/// downstream tooling can rely on stable resource/component/build-item ids.
pub(super) fn stable_uuid(kind: &str, index: usize) -> String {
    // RFC 4122's URL namespace: 6ba7b811-9dad-11d1-80b4-00c04fd430c8.
    const NAMESPACE_URL: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];
    let mut hasher = Sha1::new();
    hasher.update(NAMESPACE_URL);
    hasher.update(format!("goat4-contour-packing/{kind}/{index}"));
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50; // version 5 (name-based, SHA-1)
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}
