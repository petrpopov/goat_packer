//! AMS filament-change calibration test (spec
//! `docs/superpowers/specs/2026-10-26-ams-calibration-and-supports-design.md`, section 5).
//!
//! Two single-cube projects are generated from the user's own project 3MF, so they inherit its
//! printer, process, filaments and AMS type: `calib_A` prints the cube in filament 1 with no
//! changes, `calib_B` prints the same cube with exactly `changes` filament changes. Printing
//! both and comparing wall-clock time against the slicer estimates isolates the real duration
//! of one change:
//!
//! ```text
//! extra = ((fact_B - est_B) - (fact_A - est_A)) / changes
//! real_change_s = extra + slicer_change_s
//! ```
//!
//! Everything here is pure (bytes in, bytes/strings out) except the two small zip helpers;
//! slicing lives in `src/bin/ams_calibration.rs`.

use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use quick_xml::escape::escape;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{Rect, colored_packer::PrinterProfile};

pub type CalibrationResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Cube edge (mm). 20 mm keeps every band a proper closed-perimeter print while the flush and
/// tower dominate the time difference, which is exactly what is being measured.
pub const CUBE_SIDE_MM: f64 = 20.0;

/// Gap kept between the cube and the prime tower / bed keepouts when placing the cube.
const PLACEMENT_CLEARANCE_MM: f64 = 5.0;

/// Shown verbatim by the binary and the webapp: the template must offer a second filament to
/// change to.
pub const TWO_FILAMENTS_ERROR: &str = "для теста нужен проект минимум с двумя филаментами";

/// Used only when the template's `3D/3dmodel.model` has no `Application` metadata. The value
/// matters: BambuStudio/Orca only load `project_settings.config` from a 3MF whose
/// `Application` they recognise as one of their own.
const FALLBACK_APPLICATION: &str = "BambuStudio-02.08.02.61";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// 10 changes x 4 layers.
    Normal,
    /// 30 changes x 2 layers.
    Perfectionist,
}

impl Variant {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "normal" => Ok(Self::Normal),
            "perfectionist" => Ok(Self::Perfectionist),
            other => Err(format!(
                "unknown variant '{other}' (expected normal|perfectionist)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Perfectionist => "perfectionist",
        }
    }

    pub fn changes(self) -> usize {
        match self {
            Self::Normal => 10,
            Self::Perfectionist => 30,
        }
    }

    pub fn layers_per_band(self) -> usize {
        match self {
            Self::Normal => 4,
            Self::Perfectionist => 2,
        }
    }

    /// Expected accuracy of the measured change duration, seconds per change: two clock
    /// readings per print with minute resolution (~±1 min combined error over A and B) spread
    /// over the number of changes.
    pub fn accuracy_s(self) -> u32 {
        match self {
            Self::Normal => 12,
            Self::Perfectionist => 4,
        }
    }

    fn russian_name(self) -> &'static str {
        match self {
            Self::Normal => "обычный",
            Self::Perfectionist => "перфекционист",
        }
    }
}

/// Layer geometry of the test cube.
#[derive(Clone, Debug, PartialEq)]
pub struct BandPlan {
    pub first_layer_mm: f64,
    pub layer_height_mm: f64,
    pub layers_per_band: usize,
    /// Cube height: first layer + changes x band thickness.
    pub height_mm: f64,
    /// `top_z` of each `ToolChange` entry, ascending; one per change.
    pub change_z: Vec<f64>,
}

/// Rounds to 0.1 µm so repeated float additions print as the slicer's own layer heights.
fn round_mm(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// The cube is the first layer (filament 1) followed by `changes` bands of `layers_per_band`
/// layers, printed alternately in filament 2, 1, 2, ...; every band start is one change.
///
/// BambuStudio applies a per-layer `ToolChange` to the layer whose `print_z` equals `top_z`
/// (that layer is already printed in the new filament), so each entry carries the `print_z` of
/// the first layer of its band. Verified by real slicing (BambuStudio 02.08, 2026-10-26): an
/// entry at the first layer's own `print_z` silently recolours the first layer instead of
/// producing a change (`filament_change_times` came out one short).
pub fn band_plan(first_layer_mm: f64, layer_height_mm: f64, variant: Variant) -> Result<BandPlan, String> {
    if !(first_layer_mm > 0.0 && layer_height_mm > 0.0) {
        return Err(format!(
            "template layer heights must be positive (first layer {first_layer_mm}, layer {layer_height_mm})"
        ));
    }
    let per_band = variant.layers_per_band();
    let changes = variant.changes();
    let change_z = (0..changes)
        .map(|band| round_mm(first_layer_mm + (band * per_band + 1) as f64 * layer_height_mm))
        .collect();
    Ok(BandPlan {
        first_layer_mm,
        layer_height_mm,
        layers_per_band: per_band,
        height_mm: round_mm(first_layer_mm + (changes * per_band) as f64 * layer_height_mm),
        change_z,
    })
}

/// Lenient read of a project-settings value: a string, a number, or an array whose element 0
/// is one of those (per-extruder / per-plate values).
fn setting_text(settings: &serde_json::Value, key: &str) -> Option<String> {
    let value = settings.get(key)?;
    let value = match value {
        serde_json::Value::Array(items) => items.first()?,
        other => other,
    };
    match value {
        serde_json::Value::String(text) => Some(text.trim().to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

fn setting_number(settings: &serde_json::Value, key: &str) -> Option<f64> {
    setting_text(settings, key)?.parse().ok()
}

/// What the test needs from the template's `Metadata/project_settings.config`.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateSettings {
    pub layer_height_mm: f64,
    pub first_layer_mm: f64,
    pub filament_colours: Vec<String>,
    /// Prime tower footprint estimate on plate 1, `None` when the tower is disabled or unset.
    pub tower: Option<Rect>,
    /// Per-change seconds the slicer itself assumes for these settings.
    pub slicer_change_s: f64,
}

/// Parses the template settings; errors when it has fewer than two filaments.
pub fn parse_template_settings(bytes: &[u8]) -> CalibrationResult<TemplateSettings> {
    let settings: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("template project_settings.config is not JSON: {error}"))?;
    let filament_colours: Vec<String> = settings
        .get("filament_colour")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if filament_colours.len() < 2 {
        return Err(TWO_FILAMENTS_ERROR.into());
    }
    let layer_height_mm =
        setting_number(&settings, "layer_height").ok_or("template has no layer_height")?;
    let first_layer_mm = setting_number(&settings, "initial_layer_print_height")
        .unwrap_or(layer_height_mm);
    Ok(TemplateSettings {
        layer_height_mm,
        first_layer_mm,
        filament_colours,
        tower: tower_footprint(&settings),
        slicer_change_s: crate::print_time::slicer_change_seconds(&settings),
    })
}

/// `wipe_tower_x/y` is the tower's lower-left corner per plate (element 0 = plate 1, the only
/// plate of the calibration projects; it is copied unchanged). The slicer sizes the tower depth
/// from the purge volume, which is not known before slicing, so the footprint is taken as a
/// `prime_tower_width` square; the binary re-checks the real tower from the sliced G-code.
fn tower_footprint(settings: &serde_json::Value) -> Option<Rect> {
    let enabled = setting_text(settings, "enable_prime_tower")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    if !enabled {
        return None;
    }
    let x = setting_number(settings, "wipe_tower_x")?;
    let y = setting_number(settings, "wipe_tower_y")?;
    let width = setting_number(settings, "prime_tower_width").unwrap_or(35.0);
    Some(Rect::new(x, y, x + width, y + width))
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_y < b.max_y && b.min_y < a.max_y
}

fn grow(rect: Rect, margin: f64) -> Rect {
    Rect::new(
        rect.min_x - margin,
        rect.min_y - margin,
        rect.max_x + margin,
        rect.max_y + margin,
    )
}

/// XY footprint of the cube centred at `centre`.
pub fn cube_footprint(centre: (f64, f64)) -> Rect {
    let half = CUBE_SIDE_MM / 2.0;
    Rect::new(
        centre.0 - half,
        centre.1 - half,
        centre.0 + half,
        centre.1 + half,
    )
}

/// Cube centre: the usable-bed centre, unless it collides with the prime tower (the tower stays
/// where the template put it), then shifted along X by tower width + 10 mm, away from the tower
/// first. Every candidate must stay inside the usable bed and off the profile's exclusion
/// rectangles / purge lines.
pub fn place_cube(profile: &PrinterProfile, tower: Option<Rect>) -> CalibrationResult<(f64, f64)> {
    let bed = profile.usable_bed()?;
    let keepouts = profile.fixed_keepouts(0.0)?;
    let centre = ((bed.min_x + bed.max_x) / 2.0, (bed.min_y + bed.max_y) / 2.0);
    let mut candidates = vec![centre];
    if let Some(tower) = tower {
        let shift = (tower.max_x - tower.min_x) + 10.0;
        let tower_centre_x = (tower.min_x + tower.max_x) / 2.0;
        let away = if tower_centre_x >= centre.0 { -shift } else { shift };
        candidates.push((centre.0 + away, centre.1));
        candidates.push((centre.0 - away, centre.1));
    }
    let fits = |candidate: (f64, f64)| {
        let cube = cube_footprint(candidate);
        let inside = cube.min_x >= bed.min_x
            && cube.min_y >= bed.min_y
            && cube.max_x <= bed.max_x
            && cube.max_y <= bed.max_y;
        let padded = grow(cube, PLACEMENT_CLEARANCE_MM);
        inside
            && !keepouts.iter().any(|keepout| rects_overlap(padded, *keepout))
            && !tower.is_some_and(|tower| rects_overlap(padded, tower))
    };
    candidates
        .into_iter()
        .find(|candidate| fits(*candidate))
        .ok_or_else(|| {
            format!(
                "no free spot for the {CUBE_SIDE_MM} mm calibration cube on {} (prime tower at {tower:?})",
                profile.printer_name
            )
            .into()
        })
}

/// Axis-aligned cube mesh: 8 vertices, 12 outward-facing triangles; XY centred on the origin,
/// Z from 0 to `height`.
pub fn cube_mesh(side: f64, height: f64) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let h = side / 2.0;
    let vertices = vec![
        [-h, -h, 0.0],
        [h, -h, 0.0],
        [h, h, 0.0],
        [-h, h, 0.0],
        [-h, -h, height],
        [h, -h, height],
        [h, h, height],
        [-h, h, height],
    ];
    let triangles = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    (vertices, triangles)
}

/// Shortest decimal for a millimetre value (`0.48`, not `0.48000000000000004`).
fn fmt_mm(value: f64) -> String {
    let text = format!("{:.4}", round_mm(value));
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

fn build_transform(centre: (f64, f64)) -> String {
    format!("1 0 0 0 1 0 0 0 1 {} {} 0", fmt_mm(centre.0), fmt_mm(centre.1))
}

/// `3D/3dmodel.model`: the cube as an inline mesh (object id 1), one build item at `centre`.
pub fn model_xml(application: &str, height: f64, centre: (f64, f64)) -> String {
    let (vertices, triangles) = cube_mesh(CUBE_SIDE_MM, height);
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
         xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\">\n",
    );
    xml.push_str(&format!(
        " <metadata name=\"Application\">{}</metadata>\n",
        escape(application)
    ));
    xml.push_str(" <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n");
    xml.push_str(" <resources>\n  <object id=\"1\" type=\"model\">\n   <mesh>\n    <vertices>\n");
    for [x, y, z] in vertices {
        xml.push_str(&format!(
            "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n",
            fmt_mm(x),
            fmt_mm(y),
            fmt_mm(z)
        ));
    }
    xml.push_str("    </vertices>\n    <triangles>\n");
    for [a, b, c] in triangles {
        xml.push_str(&format!("     <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>\n"));
    }
    xml.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n");
    xml.push_str(&format!(
        " <build>\n  <item objectid=\"1\" transform=\"{}\" printable=\"1\"/>\n </build>\n</model>\n",
        build_transform(centre)
    ));
    xml
}

/// `Metadata/model_settings.config`: object 1 on extruder 1, placed on plate 1. BambuStudio
/// slices without this file too, but then the object is missing from `result.json`'s
/// `objects` list, so it is always written.
pub fn model_settings_xml(centre: (f64, f64)) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<config>\n\
  <object id=\"1\">\n\
    <metadata key=\"name\" value=\"calib_cube\"/>\n\
    <metadata key=\"extruder\" value=\"1\"/>\n\
    <part id=\"1\" subtype=\"normal_part\">\n\
      <metadata key=\"name\" value=\"calib_cube\"/>\n\
      <metadata key=\"matrix\" value=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1\"/>\n\
      <metadata key=\"extruder\" value=\"1\"/>\n\
    </part>\n\
  </object>\n\
  <plate>\n\
    <metadata key=\"plater_id\" value=\"1\"/>\n\
    <metadata key=\"plater_name\" value=\"\"/>\n\
    <metadata key=\"locked\" value=\"false\"/>\n\
    <model_instance>\n\
      <metadata key=\"object_id\" value=\"1\"/>\n\
      <metadata key=\"instance_id\" value=\"0\"/>\n\
      <metadata key=\"identify_id\" value=\"1\"/>\n\
    </model_instance>\n\
  </plate>\n\
  <assemble>\n\
   <assemble_item object_id=\"1\" instance_id=\"0\" transform=\"{}\" offset=\"0 0 0\"/>\n\
  </assemble>\n\
</config>\n",
        build_transform(centre)
    )
}

/// `Metadata/custom_gcode_per_layer.xml` (BambuStudio 3mf.cpp format): one `ToolChange`
/// (`type="2"`) per change, alternating filament 2, 1, 2, ... so the cube starts in filament 1
/// and every band switches. `colours` is the template's `filament_colour`.
pub fn custom_gcode_per_layer_xml(plan: &BandPlan, colours: &[String]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_layer>\n<plate>\n<plate_info id=\"1\"/>\n",
    );
    for (index, z) in plan.change_z.iter().enumerate() {
        let extruder = if index % 2 == 0 { 2 } else { 1 };
        let colour = colours.get(extruder - 1).map_or("", String::as_str);
        xml.push_str(&format!(
            "<layer top_z=\"{}\" type=\"2\" extruder=\"{extruder}\" color=\"{}\" extra=\"\" gcode=\"tool_change\"/>\n",
            fmt_mm(*z),
            escape(colour)
        ));
    }
    xml.push_str("<mode value=\"MultiAsSingle\"/>\n</plate>\n</custom_gcodes_per_layer>\n");
    xml
}

const CONTENT_TYPES_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n\
 <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n\
 <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n\
 <Default Extension=\"png\" ContentType=\"image/png\"/>\n\
 <Default Extension=\"gcode\" ContentType=\"text/x.gcode\"/>\n\
</Types>\n";

const ROOT_RELS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
 <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n\
</Relationships>\n";

/// The parts of the template a calibration project is built from.
#[derive(Clone, Debug)]
pub struct Template {
    /// `Metadata/project_settings.config`, byte for byte.
    pub project_settings: Vec<u8>,
    /// `Application` metadata of the template's `3D/3dmodel.model`.
    pub application: String,
}

/// Reads the template 3MF. Only the settings file and the root model's header are needed; the
/// (possibly large) object meshes are never touched.
pub fn read_template(path: &Path) -> CalibrationResult<Template> {
    let mut archive = ZipArchive::new(
        File::open(path).map_err(|error| format!("cannot open template {}: {error}", path.display()))?,
    )?;
    let mut project_settings = Vec::new();
    archive
        .by_name("Metadata/project_settings.config")
        .map_err(|_| format!("{} has no Metadata/project_settings.config", path.display()))?
        .read_to_end(&mut project_settings)?;
    let mut model = String::new();
    if let Ok(mut entry) = archive.by_name("3D/3dmodel.model") {
        entry.read_to_string(&mut model)?;
    }
    let application =
        application_metadata(&model).unwrap_or_else(|| FALLBACK_APPLICATION.to_owned());
    Ok(Template {
        project_settings,
        application,
    })
}

fn application_metadata(model: &str) -> Option<String> {
    let start = model.find("<metadata name=\"Application\">")? + "<metadata name=\"Application\">".len();
    let end = model[start..].find("</metadata>")? + start;
    let value = quick_xml::escape::unescape(model[start..end].trim()).ok()?;
    (!value.is_empty()).then(|| value.into_owned())
}

/// Writes one calibration project. `custom_gcode` is `Some` only for project B.
pub fn write_calibration_3mf(
    path: &Path,
    project_settings: &[u8],
    model: &str,
    model_settings: &str,
    custom_gcode: Option<&str>,
) -> CalibrationResult<()> {
    let mut archive = ZipWriter::new(
        File::create(path).map_err(|error| format!("cannot create {}: {error}", path.display()))?,
    );
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CONTENT_TYPES_XML.as_bytes()),
        ("_rels/.rels", ROOT_RELS_XML.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
        ("Metadata/project_settings.config", project_settings),
        ("Metadata/model_settings.config", model_settings.as_bytes()),
    ];
    if let Some(custom) = custom_gcode {
        entries.push(("Metadata/custom_gcode_per_layer.xml", custom.as_bytes()));
    }
    for (name, bytes) in entries {
        archive.start_file(name, options)?;
        archive.write_all(bytes)?;
    }
    archive.finish()?;
    Ok(())
}

/// Everything the binary needs to write and check the two projects.
#[derive(Clone, Debug)]
pub struct CalibrationProjects {
    pub settings: TemplateSettings,
    pub plan: BandPlan,
    pub centre: (f64, f64),
    pub model: String,
    pub model_settings: String,
    pub custom_gcode: String,
}

/// Pure planning step: settings + profile + variant -> file contents.
pub fn plan_projects(
    template: &Template,
    profile: &PrinterProfile,
    variant: Variant,
) -> CalibrationResult<CalibrationProjects> {
    let settings = parse_template_settings(&template.project_settings)?;
    let plan = band_plan(settings.first_layer_mm, settings.layer_height_mm, variant)?;
    let centre = place_cube(profile, settings.tower)?;
    Ok(CalibrationProjects {
        model: model_xml(&template.application, plan.height_mm, centre),
        model_settings: model_settings_xml(centre),
        custom_gcode: custom_gcode_per_layer_xml(&plan, &settings.filament_colours),
        settings,
        plan,
        centre,
    })
}

/// `filament_change_times` of plate 1 from BambuStudio's `result.json`.
pub fn bambu_filament_change_times(result_json: &[u8]) -> CalibrationResult<u64> {
    let root: serde_json::Value = serde_json::from_slice(result_json)?;
    root.get("sliced_plates")
        .and_then(serde_json::Value::as_array)
        .and_then(|plates| plates.first())
        .and_then(|plate| plate.get("filament_change_times"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Bambu result.json has no filament_change_times".into())
}

/// `instructions.md`: plain-language Russian instructions for printing and timing the test.
pub fn instructions_markdown(printer_name: &str, variant: Variant, plan: &BandPlan) -> String {
    let changes = variant.changes();
    let accuracy = variant.accuracy_s();
    format!(
        "# Калибровка времени смены филамента — {printer_name}\n\
\n\
Вариант: **{variant_ru}** — {changes} смен, точность примерно ±{accuracy} с на одну смену.\n\
\n\
В архиве два файла:\n\
\n\
- `calib_A.3mf` — кубик 20 × 20 × {height} мм одним цветом (филамент 1), без смен.\n\
- `calib_B.3mf` — такой же кубик, но цвет меняется {changes} раз (филамент 2 / филамент 1 \
каждые {layers} слоя).\n\
\n\
Оба файла сделаны из вашего проекта, поэтому принтер, настройки печати, филаменты и тип AMS \
в них те же, что вы используете обычно.\n\
\n\
## Зачем две печати\n\
\n\
Время печати складывается не только из самой печати: принтер греет стол и сопло, делает \
калибровки, и это время в оценке слайсера учтено плохо. Печать A показывает эту «накладную» \
часть, печать B — её же плюс {changes} смен. Разница между ними и есть время смен, а всё \
остальное взаимно сокращается. Поэтому A и B нужно печатать в одинаковых условиях.\n\
\n\
## Подготовка\n\
\n\
1. Пластик — PLA (любой цвет, но две разные катушки).\n\
2. Вставьте филамент 1 и филамент 2 в те слоты / тот блок AMS, с которого вы обычно печатаете. \
Если у вас несколько AMS первого поколения, дальний блок меняет пластик медленнее — \
проверяйте тот, которым пользуетесь.\n\
3. Перед каждой печатью принтер должен остыть: подождите 15 минут после предыдущей печати.\n\
4. Включите одинаковые калибровки (выравнивание стола, калибровка потока и т. п.) для A и для B. \
Если в одной печати калибровка включена, а в другой выключена — результат будет неверным.\n\
\n\
## Печать\n\
\n\
1. Отправьте на печать `calib_A.3mf`. Запишите время, когда нажали «Печать» (часы:минуты).\n\
2. Когда придёт уведомление «Печать завершена», запишите это время.\n\
3. Снимите кубик, подождите 15 минут.\n\
4. То же самое для `calib_B.3mf`: время нажатия «Печать» и время уведомления «Печать завершена».\n\
\n\
Если печать закончилась после полуночи, просто запишите время как есть — страница сама поймёт, \
что конец раньше начала означает следующий день.\n\
\n\
## Результат\n\
\n\
Введите четыре времени на странице «Калибровка AMS». Она посчитает полную длительность одной \
смены:\n\
\n\
```\n\
лишнее = ((факт_B − оценка_B) − (факт_A − оценка_A)) / {changes}\n\
смена = лишнее + время смены, которое уже заложено в слайсере\n\
```\n\
\n\
Время записывается с точностью до минуты, поэтому результат точен примерно до ±{accuracy} с \
на смену. Если «лишнее» получилось меньше 0 или больше 200 с, скорее всего время записано \
неверно — проверьте записи.\n",
        variant_ru = variant.russian_name(),
        height = fmt_mm(plan.height_mm),
        layers = plan.layers_per_band,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_mm_is_short() {
        assert_eq!(fmt_mm(0.48000000000000004), "0.48");
        assert_eq!(fmt_mm(14.599999999999998), "14.6");
        assert_eq!(fmt_mm(-10.0), "-10");
        assert_eq!(fmt_mm(0.0), "0");
    }

    #[test]
    fn application_metadata_is_extracted() {
        let model = "<model>\n <metadata name=\"Application\">Snapmaker_Orca-2.4.0</metadata>\n";
        assert_eq!(
            application_metadata(model).as_deref(),
            Some("Snapmaker_Orca-2.4.0")
        );
        assert_eq!(application_metadata("<model/>"), None);
    }
}
