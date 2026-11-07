// Exercise metadata prefill from the actual QML signal handlers with lightweight UI doubles.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { qmlMethod as functionFrom } from './helpers/qml-method.mjs';
import assert from 'node:assert/strict';

const calibrateSource = readFileSync(new URL('../src/ui/menu/LensCalibrate.qml', import.meta.url), 'utf8');
const profileSource = readFileSync(new URL('../src/ui/menu/LensProfile.qml', import.meta.url), 'utf8');


const calibrateHandler = functionFrom(calibrateSource, 'onTelemetry_loaded', '        ');
const profileHandler = functionFrom(profileSource, 'onTelemetry_loaded', '        ');
const profilesLoadedHandler = functionFrom(profileSource, 'onAll_profiles_loaded', '        ');
const cameraIdentity = vm.createContext({});
vm.runInContext(readFileSync(new URL('../src/ui/menu/CameraIdentity.js', import.meta.url), 'utf8'), cameraIdentity);

function activationHandler(selectorId) {
    const selector = profileSource.indexOf(`id: ${selectorId};`);
    const start = profileSource.indexOf('onActivated: {', selector);
    assert.ok(selector >= 0 && start > selector, `Missing activation handler for ${selectorId}`);
    const open = profileSource.indexOf('{', start);
    let depth = 0;
    for (let end = open; end < profileSource.length; end++) {
        if (profileSource[end] === '{') depth++;
        if (profileSource[end] === '}' && --depth === 0) return profileSource.slice(open + 1, end);
    }
    assert.fail(`Incomplete activation handler for ${selectorId}`);
}

function calibrationHarness() {
    const context = vm.createContext({
        CameraIdentity: cameraIdentity,
        shutter: { value: 0 }, shutterCb: { checked: false },
        readoutDirection: { value: '', set(value) { this.value = value; } },
        flcb: { checked: false }, fl: { value: 0 }, crop: { value: 1 },
        digitalLens: { currentIndex: 0 }, gs: { checked: false },
        xStretch: { value: 1 }, yStretch: { value: 1 }, sizeTimer: { start() {} },
        calib: {
            calibrationInfo: { camera_brand: 'Manual', camera_model: 'Override', lens_model: 'Custom lens' },
            resetMetadata() { this.resetCount = (this.resetCount || 0) + 1; this.calibrationInfo = {}; },
            updateTable() { this.tableUpdated = true; },
            applySelectorsFromMetadata() { this.selectorsApplied = true; },
        },
    });
    vm.runInContext(calibrateHandler, context);
    return context;
}

const calibrate = calibrationHarness();
const originalMetadata = calibrate.calib.calibrationInfo;
calibrate.onTelemetry_loaded(false, 'gyro.mp4', '', { camera_identifier: { brand: 'Wrong', model: 'Source' } });
assert.equal(calibrate.calib.resetCount || 0, 0, 'separate gyro metadata must not reset video metadata');
assert.equal(calibrate.calib.calibrationInfo, originalMetadata, 'manual metadata overrides survive separate gyro loads');

calibrate.onTelemetry_loaded(true, 'clip.mp4', 'Sony', {
    frame_readout_time: 0.01,
    frame_readout_direction: 'TopToBottom',
    camera_identifier: { brand: 'Sony', model: 'ILCE-7M4', lens_model: 'FE 24-70mm F2.8 GM' },
});
assert.equal(calibrate.calib.calibrationInfo.camera_brand, 'Sony');
assert.equal(calibrate.calib.calibrationInfo.camera_model, 'ILCE-7M4');
assert.equal(calibrate.calib.calibrationInfo.lens_model, 'FE 24-70mm F2.8 GM');
assert.equal(calibrate.calib.selectorsApplied, true, 'video metadata is applied to the catalog selectors');

const profile = vm.createContext({
    CameraIdentity: cameraIdentity,
    root: { isSony: false, selected_manually: false, detectedCamera: { brand: 'Manual', model: 'Override', lens: 'Custom' }, lensProfilesListPrepared: true, applyCount: 0,
        applyDetectedCamera() { this.applyCount++; },
        refreshCameraBrands() {
            profile.cameraBrand.model = ['Any brand', 'Canon', 'Other'];
            profile.cameraBrand.currentValue = 'Any brand';
        },
        refreshCameraModels() {
            this.refreshCameraModelsCalls = (this.refreshCameraModelsCalls || 0) + 1;
            profile.cameraModel.model = ['Any model', 'EOS R5', 'Other'];
            profile.cameraModel.currentValue = 'Any model';
        },
        refreshCameraLenses() {
            this.refreshCameraLensesCalls = (this.refreshCameraLensesCalls || 0) + 1;
            profile.cameraLens.model = ['Any lens', 'RF 24-70', 'Other'];
            profile.cameraLens.currentValue = 'Any lens';
        },
        updateCameraSearch() { this.updateCameraSearchCalls = (this.updateCameraSearchCalls || 0) + 1; profile.searchCount++; },
        searchProfiles() { profile.searchCount++; },
        loadFavorites() { profile.favoriteLoads++; } },
    controller: { lens_breathing_enabled: true, lens_metadata_delay_frames: 0 },
    lensBreathing: { checked: false }, lensDelay: { value: 0 }, outputProjection: { setValue() {} },
    focalLengthOnly: { checked: false },
    cameraBrand: { currentValue: '', model: [] }, cameraModel: { currentValue: '', model: [] }, cameraLens: { currentValue: '', model: [] },
    selectorValue(selector) { return selector.currentValue; },
    setSelectorValue(selector, value) {
        if (!selector.model.includes(value)) return false;
        selector.currentValue = value;
        return true;
    },
    searchCount: 0, favoriteLoads: 0,
});
vm.runInContext(profileHandler + '\n' + profilesLoadedHandler, profile);
profile.onTelemetry_loaded(false, 'gyro.mp4', '', { camera_identifier: { brand: 'Wrong', model: 'Source' } });
assert.deepEqual(JSON.parse(JSON.stringify(profile.root.detectedCamera)), { brand: 'Manual', model: 'Override', lens: 'Custom' },
    'separate gyro metadata must not replace the video camera prefill');
assert.equal(profile.root.applyCount, 0, 'separate gyro metadata must not reapply camera selectors');
profile.onTelemetry_loaded(true, 'clip.mp4', 'Sony', { camera_identifier: { brand: 'Sony', model: 'ILCE-7M4', lens_model: 'FE 24-70mm F2.8 GM' } });
assert.deepEqual(JSON.parse(JSON.stringify(profile.root.detectedCamera)), { brand: 'Sony', model: 'ILCE-7M4', lens: 'FE 24-70mm F2.8 GM' });
assert.equal(profile.root.applyCount, 1, 'main-video camera identity is applied');
profile.root.selected_manually = true; // User changed the selectors while another metadata result was pending.
profile.onTelemetry_loaded(true, 'clip.mp4', 'Sony', { camera_identifier: { brand: 'Sony', model: 'ILCE-7C', lens_model: 'FE 35mm F1.8' } });
assert.equal(profile.root.applyCount, 1, 'late metadata must not overwrite a manually selected camera filter');
profile.root.selected_manually = false; // VideoArea resets the manual override when a new main video opens.
profile.onTelemetry_loaded(true, 'next-clip.mp4', 'Canon', { camera_identifier: { brand: 'Canon', model: 'EOS R5', lens_model: 'RF 24-70mm F2.8 L IS USM' } });
assert.equal(profile.root.applyCount, 2, 'a new main video can apply its detected camera');

profile.lensProfilesListPrepared = false;
profile.root.lensProfilesListPrepared = false;
profile.root.selected_manually = true;
profile.root.fetched_from_github = false;
profile.controller.request_profile_ratings = () => { profile.ratingRequests = (profile.ratingRequests || 0) + 1; };
profile.controller.fetch_profiles_from_github = () => { profile.fetchRequests = (profile.fetchRequests || 0) + 1; };
profile.cameraBrand.currentValue = 'Canon';
profile.cameraModel.currentValue = 'EOS R5';
profile.cameraLens.currentValue = 'RF 24-70';
profile.onAll_profiles_loaded();
assert.equal(profile.root.applyCount, 2, 'first catalog load preserves a manual choice instead of applying detected metadata');
assert.deepEqual(
    [profile.cameraBrand.currentValue, profile.cameraModel.currentValue, profile.cameraLens.currentValue],
    ['Canon', 'EOS R5', 'RF 24-70'],
    'manual camera and lens selections survive the first catalog load'
);
assert.equal(profile.searchCount, 1, 'first catalog load searches using the restored manual choices');
assert.equal(profile.ratingRequests, 1, 'first catalog load requests profile ratings once');
assert.equal(profile.fetchRequests, 1, 'first catalog load fetches profiles once');

for (const [selectorId, refreshed] of [
    ['cameraBrand', 'refreshCameraModels'],
    ['cameraModel', 'refreshCameraLenses'],
    ['cameraLens', null],
]) {
    profile.root.lensProfilesListPrepared = true;
    profile.root.selected_manually = false;
    if (refreshed) profile.root[refreshed + 'Calls'] = 0;
    profile.root.updateCameraSearchCalls = 0;
    profile.cameraBrand.currentValue = 'Manual brand';
    profile.cameraModel.currentValue = 'Manual body';
    profile.cameraLens.currentValue = 'Manual lens';
    const applyCount = profile.root.applyCount;
    const searchCount = profile.searchCount;
    const body = activationHandler(selectorId);
    vm.runInContext(`(function () { ${body} })()`, profile);
    assert.equal(profile.root.selected_manually, true, `${selectorId} activation marks the camera filter as manual`);
    if (refreshed) assert.equal(profile.root[refreshed + 'Calls'], 1, `${selectorId} activation refreshes dependent selectors`);
    assert.equal(profile.searchCount, searchCount + 1, `${selectorId} activation updates profile search`);
    const selectedValues = [profile.cameraBrand.currentValue, profile.cameraModel.currentValue, profile.cameraLens.currentValue];
    profile.onTelemetry_loaded(true, 'late-clip.mp4', 'Sony', {
        camera_identifier: { brand: 'Sony', model: 'ILCE-7C', lens_model: 'FE 35mm F1.8' },
    });
    assert.equal(profile.root.applyCount, applyCount, `late metadata after ${selectorId} activation does not replace the manual filter`);
    assert.deepEqual(
        [profile.cameraBrand.currentValue, profile.cameraModel.currentValue, profile.cameraLens.currentValue],
        selectedValues,
        `late metadata after ${selectorId} activation preserves all selector values`
    );
}

console.log('Camera metadata prefill regressions passed');
