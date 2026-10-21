// SPDX-License-Identifier: GPL-3.0-or-later

//! Import of [Lensfun](https://lensfun.github.io/) lens-profile databases (the `.xml` files at
//! <https://github.com/lensfun/lensfun/tree/master/data/db>) as gyroflow [`LensProfile`]s.
//!
//! Lensfun's own format calibrates the `poly3`, `poly5` and `ptlens` distortion models Hugin/PanoTools
//! use, at one or more focal lengths per lens, in a normalized coordinate space tied to the lens's
//! `cropfactor` and the calibration's `aspect-ratio`. Everything this module does is porting that
//! normalization to the pixel-space, per-resolution `camera_matrix`/`distortion_coeffs` gyroflow's own
//! [`crate::stabilization::distortion_models`] expect - the same rescale
//! (`{Poly3,Poly5,PtLens}::rescale_coeffs`) gyroflow already carries for its own calibrator's use of
//! these models, ported from Lensfun's `rescale_polynomial_coefficients` in `mod-coord.cpp`, and the
//! same real-focal-length fallback (`RealFocal`) Lensfun's own database loader falls back to in
//! `database.cpp` when a calibration doesn't carry a measured `real-focal` attribute of its own.
//!
//! Any drop-in `.xml` file placed next to gyroflow's own `.json`/`.gyroflow` profiles is picked up by
//! [`crate::lens_profile_database::LensProfileDatabase::load_all`] the same way, and turns into one
//! [`LensProfile`] per rectilinear focal-length calibration it contains.

use roxmltree::Document;
use crate::LensProfile;
use crate::lens_profile::{ Dimensions, CameraParams };
use crate::stabilization::distortion_models::{ poly3::Poly3, poly5::Poly5, ptlens::PtLens };

/// `roxmltree::Node`, with both of its lifetimes tied together: every node this module handles borrows
/// straight from the [`Document`] built at the top of [`parse_lensfun_xml`], which in turn borrows the
/// caller's `&str` for exactly that call - there's never a document a node outlives, or vice versa.
type XmlNode<'a> = roxmltree::Node<'a, 'a>;

/// Diagonal, in millimetres, of a "35mm full-frame" (36×24mm) sensor - the reference every `cropfactor`
/// in a Lensfun database is relative to. Matches `hypot(36.0, 24.0)` in Lensfun's own `mod-coord.cpp`.
const FULL_FRAME_DIAGONAL_MM: f64 = 43.266_615_300_557_27;

/// An arbitrary reference height, in pixels, for the synthetic `calib_dimension` a Lensfun calibration is
/// given (Lensfun itself has no notion of a calibration image resolution - only a sensor size and an
/// aspect ratio). Its exact value doesn't affect the result: `FrameTransform::get_lens_data_at_lens_timestamp`
/// always scales `camera_matrix` by `video_size / calib_dimension` before projecting, so any resolution
/// that keeps the calibration's aspect ratio reprojects identically at whatever size the footage actually is.
const REFERENCE_CALIB_HEIGHT_PX: usize = 1000;

/// Parses one Lensfun XML database and returns one [`LensProfile`] per rectilinear focal-length
/// calibration it contains. `source_path` is stored on each profile (`path_to_file`) and used only in
/// log messages - never returns an `Err`, since one malformed or partly-understood database file
/// (a newer Lensfun schema revision, say) shouldn't stop the rest of the profile directory from loading;
/// problems are logged and simply yield fewer profiles.
pub(crate) fn parse_lensfun_xml(xml: &str, source_path: &str) -> Vec<LensProfile> {
    let stripped = strip_doctype(xml);

    let doc = match Document::parse(&stripped) {
        Ok(doc) => doc,
        Err(e) => {
            ::log::warn!("Lensfun: couldn't parse {source_path} as XML: {e}");
            return Vec::new();
        }
    };

    let root = doc.root_element();
    if !root.has_tag_name("lensdatabase") {
        ::log::warn!("Lensfun: {source_path} has no <lensdatabase> root element, skipping");
        return Vec::new();
    }

    elements(root, "lens").flat_map(|lens| profiles_from_lens(lens, source_path)).collect()
}

/// roxmltree has no DTD support, and every real Lensfun database file opens with
/// `<!DOCTYPE lensdatabase SYSTEM "lensfun-database.dtd">`, which its strict XML parser rejects outright
/// (`UnknownEntityReference` / unexpected token) before it gets anywhere near the actual data. Lensfun's
/// database files never declare an internal subset with entities of their own - the DOCTYPE is just the
/// external DTD reference above, purely for validation - so dropping the whole declaration is safe and
/// changes nothing about how the remaining document is parsed.
fn strip_doctype(xml: &str) -> std::borrow::Cow<'_, str> {
    let Some(start) = xml.find("<!DOCTYPE") else { return std::borrow::Cow::Borrowed(xml); };
    let Some(end_rel) = xml[start..].find('>') else { return std::borrow::Cow::Borrowed(xml); };
    let end = start + end_rel + 1;
    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..start]);
    out.push_str(&xml[end..]);
    std::borrow::Cow::Owned(out)
}

/// Direct element children of `node` with the given tag name (`roxmltree::Node::children` also yields
/// text and comment nodes, which `has_tag_name` would silently treat as an element with an empty name).
fn elements<'a>(node: XmlNode<'a>, tag: &'a str) -> impl Iterator<Item = XmlNode<'a>> {
    node.children().filter(move |n| n.is_element() && n.has_tag_name(tag))
}

/// Trimmed text of the first direct `<tag>` child, or `None` if there isn't one or it's blank.
fn child_text<'a>(node: XmlNode<'a>, tag: &'a str) -> Option<&'a str> {
    elements(node, tag).find_map(|n| n.text()).map(str::trim).filter(|s| !s.is_empty())
}

/// The `<maker>`/`<model>` Lensfun records once per language via a repeated element and a `lang`
/// attribute (`<model>Foo</model><model lang="en">Foo</model><model lang="de">Foo (DE)</model>`,
/// mirroring Lensfun's own `_lf_mlstr_get`): this returns the unqualified or `en` entry when there is
/// one, falling back to whichever comes first so a database that only ships one language still imports.
fn localized_text<'a>(node: XmlNode<'a>, tag: &'a str) -> Option<&'a str> {
    let mut fallback = None;
    for n in elements(node, tag) {
        let Some(text) = n.text().map(str::trim).filter(|s| !s.is_empty()) else { continue; };
        match n.attribute("lang") {
            None | Some("en") => return Some(text),
            Some(_) => fallback.get_or_insert(text),
        };
    }
    fallback
}

/// `<aspect-ratio>`/`aspect-ratio="..."` is either a bare ratio (`1.5`) or a `width:height` pair
/// (`4:3`) - Lensfun's own database writer emits the latter, its reader (`atof`, which stops at the
/// first non-numeric character) effectively only the former, and real database files mix both.
fn parse_ratio(s: &str) -> Option<f64> {
    match s.trim().split_once(':') {
        Some((w, h)) => {
            let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
            (h != 0.0).then_some(w / h)
        }
        None => s.trim().parse::<f64>().ok(),
    }
}

fn profiles_from_lens(lens: XmlNode, source_path: &str) -> Vec<LensProfile> {
    let maker = localized_text(lens, "maker").unwrap_or("Unknown").trim().to_string();
    let model = localized_text(lens, "model").unwrap_or("Unknown lens").trim().to_string();
    let mounts: Vec<&str> = elements(lens, "mount").filter_map(|n| n.text()).map(str::trim).filter(|s| !s.is_empty()).collect();

    // `<type>` defaults to "rectilinear" when absent (Lensfun's own `lfLens::Lens()` default, `lens.cpp`).
    // Every other value (fisheye, equisolid, stereographic, panoramic, equirectangular, orthographic, the
    // fisheye_* variants, ...) is a projection none of gyroflow's own `poly3`/`poly5`/`ptlens` models can
    // represent - they're all rectilinear (z=1 plane) models - so importing those coefficients as one of
    // ours would mis-project every frame instead of failing loudly. Skip them.
    let lens_type = child_text(lens, "type").unwrap_or("rectilinear");
    if lens_type != "rectilinear" {
        ::log::debug!("Lensfun: skipping non-rectilinear lens '{maker} {model}' (type={lens_type}) in {source_path}");
        return Vec::new();
    }

    let lens_crop_factor = child_text(lens, "cropfactor").and_then(|s| s.parse::<f64>().ok());
    let lens_aspect_ratio = child_text(lens, "aspect-ratio").and_then(parse_ratio);

    let Some(lens_crop_factor) = lens_crop_factor.filter(|c| *c > 0.0) else {
        ::log::debug!("Lensfun: '{maker} {model}' in {source_path} has no usable <cropfactor>, skipping");
        return Vec::new();
    };

    let mut out = Vec::new();
    for calibration in elements(lens, "calibration") {
        // A `<calibration>` block may itself carry `cropfactor`/`aspect-ratio` attributes overriding the
        // lens's own (a lens tested on more than one sensor size gets one calibration block per size) -
        // exactly the attributes `database.cpp`'s own `<calibration>` handler reads.
        let crop_factor = calibration.attribute("cropfactor").and_then(|s| s.parse::<f64>().ok()).filter(|c| *c > 0.0).unwrap_or(lens_crop_factor);
        let aspect_ratio = calibration.attribute("aspect-ratio").and_then(parse_ratio)
            .or(lens_aspect_ratio)
            .filter(|a| *a > 0.0)
            .unwrap_or(1.5); // Lensfun's own default (`AspectRatio = 1.5` in `lfLens::Lens()`, lens.cpp) when omitted entirely

        for distortion in elements(calibration, "distortion") {
            if let Some(profile) = profile_from_distortion(distortion, &maker, &model, &mounts, crop_factor, aspect_ratio, source_path) {
                out.push(profile);
            }
        }
    }
    out
}

fn profile_from_distortion(node: XmlNode, maker: &str, model: &str, mounts: &[&str], crop_factor: f64, aspect_ratio: f64, source_path: &str) -> Option<LensProfile> {
    let dist_model = node.attribute("model")?;
    let focal: f64 = node.attribute("focal")?.parse().ok().filter(|f| *f > 0.0)?;

    // `database.cpp`'s own attribute parser accepts either name for each term - `ptlens` databases write
    // `a`/`b`/`c`, `poly3`/`poly5` ones write `k1`/`k2` - and both alias the same coefficient slot.
    let term = |names: &[&str]| -> f64 { names.iter().find_map(|n| node.attribute(*n)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) };

    // `k` here (and the rescale below) is the raw Hugin/PanoTools coefficient, in Lensfun's own
    // cropfactor/aspect-ratio-normalized space - not yet a coefficient gyroflow's kernels can use; the
    // `real_focal_default` fallback matches `database.cpp`'s own when a calibration has no `real-focal`
    // attribute of its own (a lens that measures it directly, rather than assuming the nominal focal
    // length also survived the distortion correction, records the true value there instead).
    let (mut k, real_focal_default) = match dist_model {
        "ptlens" => {
            let (a, b, c) = (term(&["a", "k1"]), term(&["b", "k2"]), term(&["c", "k3"]));
            (vec![a, b, c], focal * (1.0 - a - b - c))
        }
        "poly3" => {
            let k1 = term(&["k1", "a"]);
            (vec![k1], focal * (1.0 - k1))
        }
        "poly5" => (vec![term(&["k1"]), term(&["k2"])], focal),
        // "none" (an explicit no-op calibration) and "acm" (not one of gyroflow's models) have no
        // equivalent to import.
        other => {
            ::log::debug!("Lensfun: skipping unsupported distortion model '{other}' for '{maker} {model}' in {source_path}");
            return None;
        }
    };

    let real_focal = node.attribute("real-focal").and_then(|v| v.parse::<f64>().ok()).filter(|f| *f > 0.0).unwrap_or(real_focal_default);
    if !(real_focal > 0.0) {
        return None;
    }

    // `hugin_scale_in_millimeters`/`hugin_scaling` and the per-model rescale below are exactly
    // Lensfun's `rescale_polynomial_coefficients` (`mod-coord.cpp`) - the same normalization gyroflow's
    // own `{Poly3,Poly5,PtLens}::rescale_coeffs` already implements for its own calibrator's use of these
    // models, ported from the same source.
    let hugin_scale_mm = FULL_FRAME_DIAGONAL_MM / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
    if !(hugin_scale_mm > 0.0) {
        return None;
    }
    let hugin_scaling = real_focal / hugin_scale_mm;
    match dist_model {
        "ptlens" => PtLens::rescale_coeffs(&mut k, hugin_scaling),
        "poly3"  => Poly3::rescale_coeffs(&mut k, hugin_scaling),
        "poly5"  => Poly5::rescale_coeffs(&mut k, hugin_scaling),
        _ => unreachable!("checked above"),
    }

    let calib_h = REFERENCE_CALIB_HEIGHT_PX;
    let calib_w = ((calib_h as f64) * aspect_ratio).round().max(1.0) as usize;

    // Sensor height in millimetres at this cropfactor and aspect ratio (half of `hugin_scale_mm * 2`,
    // spelled out rather than reused so the pixel-focal-length math reads on its own); the reference
    // resolution above is picked so `focal_px` comes out identical whichever of width or height it's
    // derived from.
    let sensor_height_mm = FULL_FRAME_DIAGONAL_MM / crop_factor / aspect_ratio.hypot(1.0);
    if !(sensor_height_mm > 0.0) {
        return None;
    }
    let focal_px = real_focal / sensor_height_mm * calib_h as f64;
    if !focal_px.is_finite() || focal_px <= 0.0 {
        return None;
    }

    let mut profile = LensProfile::default();
    profile.camera_brand = maker.to_string();
    profile.camera_model = format!("{maker} {model}");
    profile.lens_model = model.to_string();
    profile.camera_setting = format!("{:.0}mm", focal);
    profile.note = format!(
        "Imported from the Lensfun database ({dist_model} model, cropfactor {crop_factor:.3}). Compatible mounts: {}",
        if mounts.is_empty() { "unknown".to_string() } else { mounts.join(", ") }
    );
    profile.calibrated_by = "Lensfun project".to_string();
    profile.official = true;
    profile.focal_length = Some(focal);
    profile.crop_factor = Some(crop_factor);
    profile.calib_dimension = Dimensions { w: calib_w, h: calib_h };
    profile.orig_dimension = profile.calib_dimension.clone();
    profile.distortion_model = Some(dist_model.to_string());
    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![
            [focal_px, 0.0, calib_w as f64 / 2.0],
            [0.0, focal_px, calib_h as f64 / 2.0],
            [0.0, 0.0, 1.0],
        ],
        distortion_coeffs: k,
        radial_distortion_limit: None,
    };
    // Content-derived, not path-derived: the same lens re-exported to a different filename, or shipped in
    // more than one database that both cover it, resolves to the same identifier and is deduplicated by
    // `LensProfileDatabase::load_all` like any other repeated profile, rather than listed twice.
    profile.identifier = format!("lensfun:{maker}:{model}:{dist_model}:focal={focal:.4}:crop={crop_factor:.4}:ar={aspect_ratio:.4}");
    profile.path_to_file = source_path.to_string();
    profile.init();

    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCTYPE_HEADER: &str = "<!DOCTYPE lensdatabase SYSTEM \"lensfun-database.dtd\">\n";

    fn wrap(inner: &str) -> String {
        format!("{DOCTYPE_HEADER}<lensdatabase version=\"2\">\n{inner}\n</lensdatabase>")
    }

    #[test]
    fn strips_the_doctype_lensfun_databases_always_carry() {
        let xml = wrap("<lens><maker>Acme</maker><model>Wide</model><mount>Acme</mount><cropfactor>1</cropfactor></lens>");
        assert!(Document::parse(&xml).is_err(), "sanity check: roxmltree really does reject the raw DOCTYPE");
        let stripped = strip_doctype(&xml);
        assert!(Document::parse(&stripped).is_ok());
        assert!(!stripped.contains("DOCTYPE"));
    }

    #[test]
    fn leaves_doctype_less_input_untouched() {
        let xml = "<lensdatabase version=\"2\"></lensdatabase>";
        assert!(matches!(strip_doctype(xml), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn parses_ptlens_ratio_and_pixel_aspect_forms() {
        assert_eq!(parse_ratio("1.5"), Some(1.5));
        assert_eq!(parse_ratio("4:3"), Some(4.0 / 3.0));
        assert_eq!(parse_ratio("3:4"), Some(0.75));
        assert_eq!(parse_ratio("3:0"), None);
        assert_eq!(parse_ratio("nonsense"), None);
    }

    #[test]
    fn real_ptlens_calibration_produces_a_plausible_rectilinear_profile() {
        // A real calibration line from Lensfun's slr-canon.xml (Canon EF 17-35mm f/2.8L USM @ 17mm).
        let xml = wrap(r#"
            <lens>
                <maker>Canon</maker>
                <model>Canon EF 17-35mm f/2.8L USM</model>
                <mount>Canon EF</mount>
                <cropfactor>1</cropfactor>
                <calibration>
                    <distortion model="ptlens" focal="17" a="0.020618" b="-0.051946" c="0"/>
                </calibration>
            </lens>
        "#);
        let profiles = parse_lensfun_xml(&xml, "slr-canon.xml");
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.camera_brand, "Canon");
        assert_eq!(p.lens_model, "Canon EF 17-35mm f/2.8L USM");
        assert_eq!(p.focal_length, Some(17.0));
        assert_eq!(p.distortion_model.as_deref(), Some("ptlens"));
        assert_eq!(p.fisheye_params.distortion_coeffs.len(), 3);
        // Rescaled, not the raw Hugin coefficients straight from the XML
        assert!((p.fisheye_params.distortion_coeffs[0] - 0.020618).abs() > 1e-6);
        let fx = p.fisheye_params.camera_matrix[0][0];
        let fy = p.fisheye_params.camera_matrix[1][1];
        assert!((fx - fy).abs() < 1e-6, "fx and fy should match for a profile generated at its own calibration aspect ratio");
        assert!(fx > 0.0 && fx.is_finite());
        assert_eq!(p.calib_dimension.h, REFERENCE_CALIB_HEIGHT_PX);
    }

    #[test]
    fn poly3_and_poly5_use_the_k_attribute_names() {
        let xml = wrap(r#"
            <lens>
                <maker>Sigma</maker>
                <model>18-35mm</model>
                <mount>Canon EF</mount>
                <cropfactor>1.534</cropfactor>
                <calibration>
                    <distortion model="poly3" focal="35" k1="-0.005023"/>
                    <distortion model="poly5" focal="50" k1="-0.01" k2="0.002"/>
                </calibration>
            </lens>
        "#);
        let profiles = parse_lensfun_xml(&xml, "test.xml");
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].fisheye_params.distortion_coeffs.len(), 1);
        assert_eq!(profiles[1].fisheye_params.distortion_coeffs.len(), 2);
    }

    #[test]
    fn honors_an_explicit_real_focal_attribute() {
        // A calibration whose measured real focal length differs noticeably from the nominal one
        // (common on fisheye-adjacent wide rectilinear lenses); dropping the attribute would fall back
        // to the formula and produce a visibly different pixel focal length.
        let with_attr = wrap(r#"<lens><maker>M</maker><model>L</model><mount>X</mount><cropfactor>1.5</cropfactor>
            <calibration><distortion model="ptlens" focal="10.5" a="0.03347" b="-0.0964" c="0.08113" real-focal="10.31"/></calibration></lens>"#);
        let without_attr = wrap(r#"<lens><maker>M</maker><model>L</model><mount>X</mount><cropfactor>1.5</cropfactor>
            <calibration><distortion model="ptlens" focal="10.5" a="0.03347" b="-0.0964" c="0.08113"/></calibration></lens>"#);
        let a = parse_lensfun_xml(&with_attr, "a.xml");
        let b = parse_lensfun_xml(&without_attr, "b.xml");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_ne!(a[0].fisheye_params.camera_matrix[0][0], b[0].fisheye_params.camera_matrix[0][0]);
    }

    #[test]
    fn skips_non_rectilinear_and_unsupported_models() {
        let fisheye = wrap("<lens><maker>M</maker><model>Fisheye</model><mount>X</mount><type>fisheye</type><cropfactor>1</cropfactor>
            <calibration><distortion model=\"ptlens\" focal=\"8\" a=\"0.1\" b=\"0\" c=\"0\"/></calibration></lens>");
        assert!(parse_lensfun_xml(&fisheye, "f.xml").is_empty());

        let acm = wrap("<lens><maker>M</maker><model>L</model><mount>X</mount><cropfactor>1</cropfactor>
            <calibration><distortion model=\"acm\" focal=\"24\"/></calibration></lens>");
        assert!(parse_lensfun_xml(&acm, "acm.xml").is_empty());

        let rectilinear_default = wrap("<lens><maker>M</maker><model>L</model><mount>X</mount><cropfactor>1</cropfactor>
            <calibration><distortion model=\"poly3\" focal=\"24\" k1=\"0.01\"/></calibration></lens>");
        assert_eq!(parse_lensfun_xml(&rectilinear_default, "r.xml").len(), 1, "an absent <type> must default to rectilinear, not be skipped");
    }

    #[test]
    fn calibration_level_cropfactor_and_aspect_override_the_lens_level_ones() {
        let xml = wrap(r#"
            <lens>
                <maker>M</maker><model>L</model><mount>X</mount>
                <cropfactor>1.0</cropfactor>
                <aspect-ratio>3:2</aspect-ratio>
                <calibration cropfactor="1.5" aspect-ratio="4:3">
                    <distortion model="poly3" focal="24" k1="0.01"/>
                </calibration>
            </lens>
        "#);
        let profiles = parse_lensfun_xml(&xml, "t.xml");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].crop_factor, Some(1.5));
        let w = profiles[0].calib_dimension.w as f64 / profiles[0].calib_dimension.h as f64;
        assert!((w - 4.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn identifier_is_stable_and_distinct_per_focal_length() {
        let xml = wrap(r#"
            <lens><maker>Canon</maker><model>Z</model><mount>Canon EF</mount><cropfactor>1</cropfactor>
                <calibration>
                    <distortion model="poly3" focal="24" k1="0.01"/>
                    <distortion model="poly3" focal="35" k1="0.02"/>
                </calibration>
            </lens>
        "#);
        let a = parse_lensfun_xml(&xml, "one.xml");
        let b = parse_lensfun_xml(&xml, "two.xml");
        assert_eq!(a[0].identifier, b[0].identifier, "identifier should be content-derived, not path-derived");
        assert_ne!(a[0].identifier, a[1].identifier, "the two focal lengths must not collide");
    }

    #[test]
    fn malformed_xml_yields_no_profiles_rather_than_panicking() {
        assert!(parse_lensfun_xml("not xml at all <<<", "bad.xml").is_empty());
        assert!(parse_lensfun_xml("<lensdatabase version=\"2\"></lensdatabase>", "empty.xml").is_empty());
        assert!(parse_lensfun_xml("<somethingelse></somethingelse>", "wrong-root.xml").is_empty());
    }
}
