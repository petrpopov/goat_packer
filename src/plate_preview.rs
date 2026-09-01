//! Plate preview PNG rendering (tiny-skia) for packed layouts. Ports `_draw_preview` from
//! `build_3mf.py`; contracts in
//! `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md` (Task 2).
//!
//! Spec contract 7: previews are cosmetic. Same sizes/format and visual layout, but NOT
//! pixel-exact — PIL rasterizes hard-edged scanlines while tiny-skia antialiases, and Python's
//! `round` is banker's rounding where Rust rounds halves away from zero. The draw order, colors,
//! widths, and the screen transform below mirror the Python one for one.

use crate::Point;
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Shader, Stroke, Transform};

type Rgba = [u8; 4];

// Colors ported verbatim from `_draw_preview` (RGBA).
const BACKGROUND: Rgba = [245, 247, 250, 255];
const BED_FILL: Rgba = [224, 228, 233, 255];
const BED_OUTLINE: Rgba = [45, 50, 58, 255];
const SAFE_AREA_OUTLINE: Rgba = [35, 170, 95, 255];
const PURGE_LINE: Rgba = [230, 72, 72, 255];
const EXCLUSION_FILL: Rgba = [242, 112, 112, 230];
const KEEPOUT_FILL: Rgba = [238, 140, 76, 170];
const KEEPOUT_OUTLINE: Rgba = [185, 82, 26, 255];
const FOOTPRINT_FILL: Rgba = [111, 169, 220, 255];
const FOOTPRINT_OUTLINE: Rgba = [22, 91, 148, 255];

/// Geometry for one preview, all already rotated and translated into world (bed) coordinates by
/// the caller — the Rust equivalent of `_placed_polygons` feeding `_draw_preview`.
pub struct PreviewInput<'a> {
    /// One entry per packed object: `polygons[i][0]` is the exterior ring and the remaining
    /// rings are holes — exactly `Footprint::loops` after per-placement rotation/translation.
    /// Rings may repeat their first point as an explicit closing point (that is what
    /// `extract_footprint` emits). (The task text shorthand wrote `&'a [Vec<Point>]`, but the
    /// exterior-plus-holes semantics it demands need one nesting level more.)
    pub polygons: &'a [Vec<Vec<Point>>],
    /// Printable-area polygon from `Metadata/project_settings.config`; defines the screen
    /// transform — everything is scaled to fit these bounds.
    pub printable: &'a [(f64, f64)],
    /// `bed_exclude_area` polygons; drawn when a polygon has at least 3 points.
    pub exclusions: &'a [Vec<(f64, f64)>],
    /// `(x0, y0, x1, y1, width)` axis-aligned purge-line segments.
    pub purge_lines: &'a [(f64, f64, f64, f64, f64)],
    /// `(min_x, min_y, max_x, max_y)` keepout rectangles.
    pub keepouts: &'a [(f64, f64, f64, f64)],
    /// Safe-area inset drawn as a green outline when greater than 0.
    pub edge_margin: f64,
    /// Pick-mode preview: per-index rainbow footprint fills instead of the flat blue.
    pub pick: bool,
}

/// Renders one preview as PNG bytes of a `size × size` RGBA image.
///
/// Draw order mirrors `_draw_preview` exactly: background, bed polygon, safe-area rectangle,
/// purge lines, exclusions, keepouts, footprints. Degenerate inputs (no printable polygon,
/// zero-size bed) yield the plain background render instead of panicking.
pub fn render(input: &PreviewInput, size: u32) -> Vec<u8> {
    let Some(mut pixmap) = Pixmap::new(size, size) else {
        // A 0×0 image has no valid PNG encoding (IHDR forbids zero dimensions); report "no
        // preview" as empty bytes rather than panicking.
        return Vec::new();
    };
    pixmap.fill(color(BACKGROUND));

    let Some(screen) = Screen::new(input.printable, size) else {
        return encode(pixmap);
    };

    // 1. Bed polygon: fill plus an outline whose width grows with the canvas
    //    (`width=max(1, size // 256)` in the Python).
    if let Some(bed) = ring_path(input.printable.iter().copied(), &screen) {
        fill_even_odd(&mut pixmap, &bed, BED_FILL);
        stroke(&mut pixmap, &bed, BED_OUTLINE, canvas_outline_width(size));
    }

    // 2. Safe-area rectangle inset by edge_margin, outline-only.
    if input.edge_margin > 0.0 {
        let m = input.edge_margin;
        let corners = [
            (screen.min_x + m, screen.min_y + m),
            (screen.max_x - m, screen.min_y + m),
            (screen.max_x - m, screen.max_y - m),
            (screen.min_x + m, screen.max_y - m),
        ];
        if let Some(safe_area) = ring_path(corners.iter().copied(), &screen) {
            stroke(
                &mut pixmap,
                &safe_area,
                SAFE_AREA_OUTLINE,
                canvas_outline_width(size),
            );
        }
    }

    // 3. Purge lines, stroked at `max(2, round(width · scale))` screen pixels.
    for &(x0, y0, x1, y1, width) in input.purge_lines {
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let (sx0, sy0) = screen.point(x0, y0);
        let (sx1, sy1) = screen.point(x1, y1);
        let mut builder = PathBuilder::new();
        builder.move_to(sx0, sy0);
        builder.line_to(sx1, sy1);
        if let Some(line) = builder.finish() {
            let stroke_width = purge_stroke_width(width, screen.scale, size);
            stroke(&mut pixmap, &line, PURGE_LINE, stroke_width);
        }
    }

    // 4. Exclusion polygons (fill only; the Python skips polygons with fewer than 3 points).
    for exclusion in input.exclusions {
        if exclusion.len() >= 3
            && let Some(path) = ring_path(exclusion.iter().copied(), &screen)
        {
            fill_even_odd(&mut pixmap, &path, EXCLUSION_FILL);
        }
    }

    // 5. Keepout rectangles, fill plus a 1 px outline (PIL's `draw.rectangle` default width).
    for &(min_x, min_y, max_x, max_y) in input.keepouts {
        if ![min_x, min_y, max_x, max_y].iter().all(|v| v.is_finite()) {
            continue;
        }
        let (ax, ay) = screen.point(min_x, min_y);
        let (bx, by) = screen.point(max_x, max_y);
        // The Y flip means the screen-space rectangle corners are the component-wise min/max of
        // the two projected corners, exactly the min/max PIL's draw.rectangle applies.
        let corners = [
            (f64::from(ax.min(bx)), f64::from(ay.min(by))),
            (f64::from(ax.max(bx)), f64::from(ay.min(by))),
            (f64::from(ax.max(bx)), f64::from(ay.max(by))),
            (f64::from(ax.min(bx)), f64::from(ay.max(by))),
        ];
        if let Some(rect) = ring_path(corners.iter().copied(), &screen) {
            fill_even_odd(&mut pixmap, &rect, KEEPOUT_FILL);
            stroke(&mut pixmap, &rect, KEEPOUT_OUTLINE, 1.0);
        }
    }

    // 6. Footprints.
    for (index, rings) in input.polygons.iter().enumerate() {
        let fill = if input.pick {
            pick_fill(index)
        } else {
            FOOTPRINT_FILL
        };
        // PIL filled each exterior ring and then repainted every hole with the bed color. With
        // tiny-skia we instead build ONE path containing the exterior ring followed by all hole
        // rings and fill it with FillRule::EvenOdd, which punches the holes out of the footprint
        // directly — no separate hole redraw. (Unlike PIL's trick, whatever sits beneath a hole
        // shows through instead of painted-over bed color; cosmetic, accepted by contract 7.)
        if let Some(footprint) = footprint_path(rings, &screen) {
            fill_even_odd(&mut pixmap, &footprint, fill);
        }
        // PIL outlined only the exterior ring; holes stay outline-free.
        if let Some(exterior) = rings.first()
            && let Some(outline) = ring_path(exterior.iter().map(|p| (p.x, p.y)), &screen)
        {
            stroke(&mut pixmap, &outline, FOOTPRINT_OUTLINE, 1.0);
        }
    }

    encode(pixmap)
}

/// The world→screen transform of `_draw_preview`: a 3% margin and a uniform scale that fits the
/// printable bounds, with the Y axis flipped (bed +Y is up, PNG +Y is down).
struct Screen {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    scale: f64,
    margin: f64,
    size: f64,
}

impl Screen {
    /// `None` when the printable area cannot define a usable transform (empty polygon, zero-size
    /// or inverted bed, canvas too small for the 3% margin) — the caller then renders only the
    /// background instead of dividing by zero.
    fn new(printable: &[(f64, f64)], size: u32) -> Option<Self> {
        if printable.is_empty() {
            return None;
        }
        let min_x = printable.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let max_x = printable
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = printable.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let max_y = printable
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max);
        // Python: margin = max(3, round(size * 0.03)); its banker's rounding can differ from
        // Rust's by half a pixel at most (cosmetic).
        let margin = 3.0f64.max((f64::from(size) * 0.03).round());
        let span_x = max_x - min_x;
        let span_y = max_y - min_y;
        if span_x <= 0.0 || span_y <= 0.0 {
            return None;
        }
        let drawable = f64::from(size) - 2.0 * margin;
        let scale = (drawable / span_x).min(drawable / span_y);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(Self {
            min_x,
            max_x,
            min_y,
            max_y,
            scale,
            margin,
            size: f64::from(size),
        })
    }

    fn point(&self, x: f64, y: f64) -> (f32, f32) {
        (
            (self.margin + (x - self.min_x) * self.scale) as f32,
            (self.size - self.margin - (y - self.min_y) * self.scale) as f32,
        )
    }
}

fn color(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn solid_paint(rgba: Rgba) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color(rgba)),
        anti_alias: true,
        ..Paint::default()
    }
}

fn fill_even_odd(pixmap: &mut Pixmap, path: &Path, rgba: Rgba) {
    pixmap.fill_path(
        path,
        &solid_paint(rgba),
        FillRule::EvenOdd,
        Transform::identity(),
        None,
    );
}

fn stroke(pixmap: &mut Pixmap, path: &Path, rgba: Rgba, width: f32) {
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(
        path,
        &solid_paint(rgba),
        &stroke,
        Transform::identity(),
        None,
    );
}

/// Appends one closed ring subpath, skipping non-finite vertices so malformed geometry cannot
/// reach the tessellator.
fn push_ring(builder: &mut PathBuilder, ring: impl Iterator<Item = (f64, f64)>, screen: &Screen) {
    let mut open = false;
    for (x, y) in ring {
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let (sx, sy) = screen.point(x, y);
        if open {
            builder.line_to(sx, sy);
        } else {
            builder.move_to(sx, sy);
            open = true;
        }
    }
    if open {
        builder.close();
    }
}

fn ring_path(ring: impl Iterator<Item = (f64, f64)>, screen: &Screen) -> Option<Path> {
    let mut builder = PathBuilder::new();
    push_ring(&mut builder, ring, screen);
    builder.finish()
}

/// One path per footprint: the exterior ring followed by every hole ring (see the EvenOdd note
/// in `render`). Each ring gets its own subpath, so rings are never connected to each other.
fn footprint_path(rings: &[Vec<Point>], screen: &Screen) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for ring in rings {
        push_ring(&mut builder, ring.iter().map(|p| (p.x, p.y)), screen);
    }
    builder.finish()
}

/// PIL: `width=max(1, size // 256)` for the bed outline and safe-area rectangle.
fn canvas_outline_width(size: u32) -> f32 {
    (size / 256).max(1) as f32
}

/// PIL: `width=max(2, round(width * scale))` for purge lines. The result is clamped to a few
/// canvas sizes so an absurd line width cannot overflow the stroker (anything past the canvas
/// edge is invisible anyway).
fn purge_stroke_width(line_width: f64, scale: f64, size: u32) -> f32 {
    let pixels = if line_width.is_finite() {
        (line_width * scale).round().max(2.0)
    } else {
        2.0
    };
    pixels.min(f64::from(size) * 4.0) as f32
}

/// Pick-mode palette from the Python:
/// `((37·(i+1)) % 254 + 1, (83·(i+1)) % 254 + 1, (149·(i+1)) % 254 + 1)`.
fn pick_fill(index: usize) -> Rgba {
    let step = index as u64 + 1;
    [
        ((37 * step) % 254 + 1) as u8,
        ((83 * step) % 254 + 1) as u8,
        ((149 * step) % 254 + 1) as u8,
        255,
    ]
}

fn encode(pixmap: Pixmap) -> Vec<u8> {
    // Only zero-sized pixmaps fail to encode, and `render` never builds one.
    pixmap
        .encode_png()
        .expect("PNG encode of an in-memory preview")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    fn point_square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
        vec![
            Point { x: x0, y: y0 },
            Point { x: x1, y: y0 },
            Point { x: x1, y: y1 },
            Point { x: x0, y: y1 },
        ]
    }

    fn tuple_square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    /// Synthetic scene: printable square 0..200, one square footprint (exterior ring carries the
    /// explicit closing point `extract_footprint` emits) with a square hole, an exclusion
    /// polygon, one purge line, one keepout, and a 5 mm edge margin so every draw branch runs.
    struct Scene {
        polygons: Vec<Vec<Vec<Point>>>,
        printable: Vec<(f64, f64)>,
        exclusions: Vec<Vec<(f64, f64)>>,
        purge_lines: Vec<(f64, f64, f64, f64, f64)>,
        keepouts: Vec<(f64, f64, f64, f64)>,
    }

    impl Scene {
        fn sample() -> Self {
            let mut exterior = point_square(40.0, 40.0, 160.0, 160.0);
            exterior.push(exterior[0]);
            let hole = point_square(80.0, 80.0, 120.0, 120.0);
            Self {
                polygons: vec![vec![exterior, hole]],
                printable: tuple_square(0.0, 0.0, 200.0, 200.0),
                exclusions: vec![tuple_square(5.0, 5.0, 35.0, 35.0)],
                purge_lines: vec![(10.0, 10.0, 190.0, 10.0, 6.0)],
                keepouts: vec![(5.0, 170.0, 60.0, 195.0)],
            }
        }

        fn input(&self, pick: bool) -> PreviewInput<'_> {
            PreviewInput {
                polygons: &self.polygons,
                printable: &self.printable,
                exclusions: &self.exclusions,
                purge_lines: &self.purge_lines,
                keepouts: &self.keepouts,
                edge_margin: 5.0,
                pick,
            }
        }
    }

    fn assert_valid_png(png: &[u8], size: u32) {
        assert!(png.len() >= 33, "PNG too short for signature + IHDR");
        assert_eq!(&png[..8], &PNG_SIGNATURE, "missing PNG signature");
        assert_eq!(&png[8..12], &[0, 0, 0, 13], "IHDR data must be 13 bytes");
        assert_eq!(&png[12..16], b"IHDR", "first chunk must be IHDR");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!(width, size, "IHDR width");
        assert_eq!(height, size, "IHDR height");
        assert_eq!(png[24], 8, "IHDR bit depth must be 8 (tiny-skia RGBA8)");
        assert_eq!(png[25], 6, "IHDR color type must be 6 (RGBA)");
    }

    fn distinct_byte_values(png: &[u8]) -> usize {
        let mut seen = [false; 256];
        for &byte in png {
            seen[byte as usize] = true;
        }
        seen.into_iter().filter(|&seen| seen).count()
    }

    #[test]
    fn renders_size_square_png_at_512_and_128() {
        let scene = Scene::sample();
        for size in [512, 128] {
            let png = render(&scene.input(false), size);
            assert_valid_png(&png, size);
            // A constant image would collapse to a handful of distinct byte values.
            assert!(
                distinct_byte_values(&png) > 32,
                "render at {size} px looks constant"
            );
        }
    }

    #[test]
    fn render_depends_on_scene_content_and_is_deterministic() {
        let scene = Scene::sample();
        let full = render(&scene.input(false), 512);
        let bare = Scene {
            polygons: Vec::new(),
            printable: scene.printable.clone(),
            exclusions: Vec::new(),
            purge_lines: Vec::new(),
            keepouts: Vec::new(),
        };
        assert_ne!(full, render(&bare.input(false), 512));
        assert_eq!(
            full,
            render(&scene.input(false), 512),
            "must be deterministic"
        );
    }

    #[test]
    fn pick_mode_changes_the_render() {
        let scene = Scene::sample();
        assert_ne!(
            render(&scene.input(false), 512),
            render(&scene.input(true), 512)
        );
    }

    #[test]
    fn moving_a_polygon_changes_the_render() {
        let scene = Scene::sample();
        let mut moved = Scene::sample();
        for point in &mut moved.polygons[0][0] {
            point.x += 10.0;
        }
        assert_ne!(
            render(&scene.input(false), 512),
            render(&moved.input(false), 512)
        );
    }

    /// The EvenOdd single-path fill must punch holes out of the footprint: the hole interior
    /// stays untouched while the ring is painted (the property PIL faked by repainting holes
    /// with the bed color).
    #[test]
    fn even_odd_fill_punches_holes_out_of_footprints() {
        let printable = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let screen = Screen::new(&printable, 100).expect("valid printable area");
        let rings = vec![
            point_square(10.0, 10.0, 90.0, 90.0),
            point_square(40.0, 40.0, 60.0, 60.0),
        ];
        let path = footprint_path(&rings, &screen).expect("two finite rings");

        let mut pixmap = Pixmap::new(100, 100).expect("100×100 pixmap");
        fill_even_odd(&mut pixmap, &path, FOOTPRINT_FILL);
        let pixels = pixmap.data();
        let at = |x: usize, y: usize| {
            let base = (y * 100 + x) * 4;
            [
                pixels[base],
                pixels[base + 1],
                pixels[base + 2],
                pixels[base + 3],
            ]
        };

        // Screen center projects to world (50, 50): inside the hole, so untouched.
        assert_eq!(at(50, 50), [0, 0, 0, 0]);
        // Screen (25, 50) projects to world (~23.4, 50): inside the ring, painted opaque blue
        // (premultiplied RGBA equals straight RGBA at alpha 255).
        assert_eq!(at(25, 50), [111, 169, 220, 255]);
    }

    #[test]
    fn degenerate_inputs_render_a_plain_background() {
        fn bare<'a>(printable: &'a [(f64, f64)]) -> PreviewInput<'a> {
            PreviewInput {
                polygons: &[],
                printable,
                exclusions: &[],
                purge_lines: &[],
                keepouts: &[],
                edge_margin: 5.0,
                pick: false,
            }
        }

        let background = render(&bare(&[]), 256);
        assert_valid_png(&background, 256);

        // Zero-size bed: no usable scale, so the output is the plain background.
        let zero_bed = vec![(0.0, 0.0); 4];
        assert_eq!(render(&bare(&zero_bed), 256), background);

        // Content with no usable bed degrades to the background instead of panicking.
        let scene = Scene::sample();
        let no_bed = PreviewInput {
            polygons: &scene.polygons,
            printable: &[],
            exclusions: &scene.exclusions,
            purge_lines: &scene.purge_lines,
            keepouts: &scene.keepouts,
            edge_margin: 5.0,
            pick: false,
        };
        assert_eq!(render(&no_bed, 256), background);
    }
}
