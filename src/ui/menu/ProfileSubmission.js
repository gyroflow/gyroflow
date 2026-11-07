// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

function hasSetupName(value) {
    return !!value && !/^(?:[-?]+|other|unknown|none|n\/?a|unspecified)$/i.test(value);
}

function isZoomLens(lens) {
    if (/\bzoom\b|\d+(?:\.\d+)?\s*(?:mm\s*)?[-–—]\s*\d+(?:\.\d+)?\s*mm/i.test(lens)) return true;
    // Community profiles also use names like "18-105 F4", without units.
    const range = /(?:^|[\s(])([1-9]\d{0,2})\s*[-–—]\s*([1-9]\d{0,3})(?=$|[\s/@(]|f\d)/i.exec(lens);
    return !!range && +range[2] > +range[1];
}

function isOpticsSpecOnlyLens(value) {
    const specs = value.replace(/mm|[ft]/gi, "");
    return /\d/.test(specs) && /^[\d\s.,:/@–—-]+$/.test(specs);
}

function validationErrors(metadata, selection, needsLens) {
    const brand = (metadata.camera_brand || "").trim();
    const model = (metadata.camera_model || "").trim();
    const lens = (metadata.lens_model || "").trim();
    const errors = [];
    if (!hasSetupName(brand)) errors.push("camera_brand_required");
    if (!hasSetupName(model)) errors.push("camera_model_required");
    if ((selection.brandIndex > 0 && brand !== selection.brandValue) ||
        (selection.modelIndex > 0 && model !== selection.modelValue)) errors.push("camera_other_required");
    if (brand && model && !hasSetupName(lens) && needsLens) errors.push("lens_required");
    if (needsLens && selection.lensIndex === 0 && hasSetupName(lens) && isOpticsSpecOnlyLens(lens)) errors.push("lens_name_required");
    if (selection.lensIndex > 0 && lens !== selection.lensValue) errors.push("lens_other_required");
    const focal = +metadata.focal_length;
    if (isZoomLens(lens) && (!selection.hasFocalLength || !Number.isFinite(focal) || focal <= 0)) errors.push("zoom_focal_length_required");
    return errors;
}

function metadataSnapshot(metadata) {
    return JSON.parse(JSON.stringify(metadata));
}
