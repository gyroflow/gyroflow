// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

function selectorModel(emptyText, values, otherText) {
    let model = [emptyText];
    for (const value of values || []) {
        if (value && model.indexOf(value) === -1) model.push(value);
    }
    if (otherText) model.push(otherText);
    return model;
}

function selectorValue(currentIndex, model, hasOtherOption) {
    if (currentIndex <= 0 || (hasOtherOption && currentIndex >= model.length - 1) || currentIndex >= model.length) return "";
    return model[currentIndex];
}

function selectorIndex(model, value) {
    const index = model.indexOf(value);
    return index > 0 ? index : -1;
}

function detectedCamera(identifier) {
    if (!identifier || !identifier.brand) return {};
    return {
        brand: identifier.brand,
        model: identifier.model || "",
        lens: identifier.lens_model || identifier.lens_info || ""
    };
}

function hasSetupName(value) {
    return !!value && !/^(?:[-?]+|other|unknown|none|n\/?a|unspecified)$/i.test(value);
}

function isZoomLens(lens) {
    if (/\bzoom\b|\d+(?:\.\d+)?\s*(?:mm\s*)?[-–—]\s*\d+(?:\.\d+)?\s*mm/i.test(lens)) return true;
    // Community profiles also use names like "18-105 F4", without units.
    const range = /(?:^|[\s(])([1-9]\d{0,2})\s*[-–—]\s*([1-9]\d{0,3})(?=$|[\s/@(]|f\d)/i.exec(lens);
    return !!range && +range[2] > +range[1];
}

function cameraNeedsLens(info) {
    info = info || {};
    return info.fixed_lens !== true;
}
