.pragma library
// SPDX-License-Identifier: GPL-3.0-or-later
// Catalogue metadata does not contain a usable distortion calibration.

function clean(value) { return typeof value === "string" ? value.trim().replace(/\s+/g, " ") : ""; }
function norm(value) { return clean(value).toLowerCase(); }
function cameraKey(brand, model) { return JSON.stringify([norm(brand), norm(model)]); }
function ordered(values) { return values.sort(function(a, b) { return a.localeCompare(b); }); }
function unique(values) { return ordered(Array.from(new Set(values.filter(function(x) { return !!x; })))); }
function positive(value) { return typeof value === "number" && isFinite(value) && value > 0; }
function list(value) { return Array.isArray(value) ? value : []; }
function record(value) { return !!value && typeof value === "object" && !Array.isArray(value); }
function overlaps(a, b) { return list(a).some(function(value) { return list(b).indexOf(value) !== -1; }); }
function profileKey(profile) { return profile ? clean(profile.id || profile.checksum) : ""; }
function normalizeHidden(value) {
    var result = Object.create(null);
    if (!record(value)) return result;
    Object.keys(value).forEach(function(key) {
        if (value[key] === true) result[key] = true;
    });
    return result;
}
function isProfileHidden(hidden, profile) {
    var key = profileKey(profile);
    return !!key && record(hidden) && Object.prototype.hasOwnProperty.call(hidden, key)
        && hidden[key] === true;
}
function addAlias(map, key, value) {
    if (!Object.prototype.hasOwnProperty.call(map, key)) map[key] = value;
    else if (map[key] !== value) map[key] = null; // Ambiguous names must not auto-select.
}

function build(document) {
    if (!document || document.schema_version !== 1 || !Array.isArray(document.cameras)
        || !Array.isArray(document.lenses) || !Array.isArray(document.profiles)) {
        throw new Error("Unsupported camera catalogue");
    }
    var index = { cameras: Object.create(null), aliases: Object.create(null),
                  modelAliases: Object.create(null),
                  brands: [], models: Object.create(null), profiles: Object.create(null),
                  lenses: document.lenses, lensesByMount: Object.create(null), lensChoices: Object.create(null),
                  compatible: Object.create(null), count: 0 };
    // Reserve every canonical model ID before considering aliases. Otherwise
    // one camera's alias can invalidate a real model added earlier/later.
    document.cameras.forEach(function(camera) {
        if (!record(camera) || !clean(camera.brand) || !clean(camera.model)) return;
        var id = cameraKey(camera.brand, camera.model);
        index.cameras[id] = camera;
        index.aliases[id] = id;
        addAlias(index.modelAliases, norm(camera.model), id);
    });
    document.cameras.forEach(function(camera) {
        if (!record(camera) || !clean(camera.brand) || !clean(camera.model)) return;
        var id = cameraKey(camera.brand, camera.model);
        list(camera.aliases).forEach(function(alias) {
            if (!clean(alias)) return;
            var key = cameraKey(camera.brand, alias);
            if (!Object.prototype.hasOwnProperty.call(index.cameras, key)) {
                addAlias(index.aliases, key, id);
            }
            addAlias(index.modelAliases, norm(alias), id);
        });
    });
    document.profiles.forEach(function(profile) {
        if (!record(profile) || !clean(profile.id) || !clean(profile.brand) || !clean(profile.model)) return;
        var id = index.aliases[cameraKey(profile.brand, profile.model)] || cameraKey(profile.brand, profile.model);
        if (!index.cameras[id]) {
            index.cameras[id] = { brand: clean(profile.brand), model: clean(profile.model), mounts: [], aliases: [] };
            addAlias(index.aliases, id, id);
            addAlias(index.modelAliases, norm(profile.model), id);
        }
        if (!index.profiles[id]) index.profiles[id] = Object.create(null);
        var lens = clean(profile.lens);
        if (!index.profiles[id][lens]) index.profiles[id][lens] = [];
        index.profiles[id][lens].push(profile);
        index.count += 1;
    });
    Object.keys(index.cameras).forEach(function(id) {
        var camera = index.cameras[id], brand = clean(camera.brand);
        if (!index.models[brand]) index.models[brand] = [];
        index.models[brand].push(camera.model);
    });
    index.brands = ordered(Object.keys(index.models));
    Object.keys(index.models).forEach(function(brand) { index.models[brand] = unique(index.models[brand]); });
    document.lenses.forEach(function(lens) {
        if (!record(lens) || !clean(lens.model)) return;
        list(lens.mounts).forEach(function(mount) {
            if (!clean(mount)) return;
            if (!index.lensesByMount[mount]) index.lensesByMount[mount] = [];
            index.lensesByMount[mount].push(lens);
        });
    });
    Object.keys(index.profiles).forEach(function(id) {
        Object.keys(index.profiles[id]).forEach(function(lens) {
            index.profiles[id][lens].sort(function(a, b) {
                return (b.official ? 1 : 0) - (a.official ? 1 : 0) ||
                       (b.rating || 0) - (a.rating || 0) ||
                       clean(a.name).localeCompare(clean(b.name)) || a.id.localeCompare(b.id);
            });
        });
    });
    return index;
}

function resolveCamera(index, brand, model) {
    if (!index) return "";
    var branded = index.aliases[cameraKey(brand, model)] || "";
    if (branded || clean(brand)) return branded;
    return index.modelAliases[norm(model)] || "";
}

function camera(index, brand, model) {
    return index && index.cameras[resolveCamera(index, brand, model)] || null;
}

function lensLabels(index, brand, model) {
    var id = resolveCamera(index, brand, model);
    if (!id) return [];
    if (index.lensChoices[id]) return index.lensChoices[id].slice();
    var result = Object.keys(index.profiles[id] || {});
    var body = index.cameras[id];
    list(body.mounts).forEach(function(mount) {
        (index.lensesByMount[mount] || []).forEach(function(lens) {
            // A smaller image circle must not be suggested for a larger sensor.
            if (positive(body.crop_factor) && positive(lens.crop_factor) && lens.crop_factor > body.crop_factor * 1.01) return;
            result.push(clean(lens.model));
        });
    });
    index.lensChoices[id] = unique(result);
    return index.lensChoices[id].slice();
}

function profilesFor(index, brand, model, lens) {
    var id = resolveCamera(index, brand, model);
    var bucket = index && index.profiles[id];
    return bucket && bucket[clean(lens)] ? bucket[clean(lens)].slice() : [];
}

function compatibleCameras(index, brand, model) {
    var id = resolveCamera(index, brand, model);
    if (!id) return [];
    if (index.compatible[id]) return index.compatible[id].slice();
    var selected = index.cameras[id];
    if (!positive(selected.crop_factor) || !list(selected.mounts).length) return [];
    var result = Object.keys(index.cameras).filter(function(otherId) {
        if (otherId === id) return false;
        var other = index.cameras[otherId];
        return positive(other.crop_factor) && Math.abs(other.crop_factor / selected.crop_factor - 1) <= 0.01
            && overlaps(selected.mounts, other.mounts);
    }).map(function(otherId) { return index.cameras[otherId]; });
    result.sort(function(a, b) { return (a.brand + " " + a.model).localeCompare(b.brand + " " + b.model); });
    index.compatible[id] = result;
    return result.slice();
}

function prefill(index, metadata) {
    var value = metadata || {};
    var id = resolveCamera(index, value.brand || value.camera_brand, value.model || value.camera_model);
    if (!id) return { brand: clean(value.brand || value.camera_brand), model: clean(value.model || value.camera_model),
                      lens: clean(value.lens_model || value.lens_info), known: false };
    var body = index.cameras[id];
    var labels = lensLabels(index, body.brand, body.model);
    var lensModel = clean(value.lens_model);
    var lensInfo = clean(value.lens_info);
    var candidates = [];
    if (lensModel) candidates.push(lensModel);
    if (lensInfo && norm(lensInfo) !== norm(lensModel)) candidates.push(lensInfo);
    if (lensModel && lensInfo && norm(lensModel) !== norm(lensInfo)) {
        candidates.push(clean(lensModel + " " + lensInfo));
    }
    var requested = candidates.length ? candidates[0] : "";
    for (var i = 0; i < candidates.length; ++i) {
        var matches = labels.filter(function(label) { return norm(label) === norm(candidates[i]); });
        if (matches.length === 1) {
            requested = matches[0];
            break;
        }
    }
    return { brand: body.brand, model: body.model, lens: requested, known: true };
}

function visibleProfiles(profiles, hidden, showHidden) {
    return (profiles || []).filter(function(profile) { return showHidden || !isProfileHidden(hidden, profile); });
}

function profileLabel(profile) {
    return clean(profile.name) + " — " + (profile.width || "?") + "×" + (profile.height || "?")
        + (positive(profile.focal_length) ? " / " + profile.focal_length + " mm" : "")
        + (clean(profile.author) ? " / " + profile.author : "");
}
