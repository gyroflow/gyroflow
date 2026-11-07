// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

function isOpticsSpecOnlyLens(value) {
    const focal = "\\d+(?:\\.\\d+)?\\s*(?:mm)?";
    const range = focal + "(?:\\s*[-–—]\\s*" + focal + ")?";
    const stop = "(?:f|t)\\s*\\/?\\s*\\d+(?:\\.\\d+)?";
    const ratio = "1\\s*:\\s*\\d+(?:\\.\\d+)?";
    return new RegExp("^(?:" + range + "(?:\\s+(?:" + stop + "|" + ratio + "))?|" + stop + "|" + ratio + ")$", "i").test((value || "").trim());
}

function validationErrors(metadata, selection, needsLens, hasSetupName, isZoomLens, isOpticsSpecOnlyLens) {
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
