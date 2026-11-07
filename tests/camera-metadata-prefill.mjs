// Exercise metadata prefill from the actual QML signal handlers with lightweight UI doubles.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { qmlMethod as functionFrom } from './helpers/qml-method.mjs';
import assert from 'node:assert/strict';

const calibrateSource = readFileSync(new URL('../src/ui/menu/LensCalibrate.qml', import.meta.url), 'utf8');
const profileSource = readFileSync(new URL('../src/ui/menu/LensProfile.qml', import.meta.url), 'utf8');


const calibrateHandler = functionFrom(calibrateSource, 'onTelemetry_loaded', '        ');
const profileHandler = functionFrom(profileSource, 'onTelemetry_loaded', '        ');
const cameraIdentity = vm.createContext({});
vm.runInContext(readFileSync(new URL('../src/ui/menu/CameraIdentity.js', import.meta.url), 'utf8'), cameraIdentity);

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
    root: { isSony: false, detectedCamera: { brand: 'Manual', model: 'Override', lens: 'Custom' }, lensProfilesListPrepared: true, applyCount: 0,
        applyDetectedCamera() { this.applyCount++; } },
    controller: { lens_breathing_enabled: true, lens_metadata_delay_frames: 0 },
    lensBreathing: { checked: false }, lensDelay: { value: 0 }, outputProjection: { setValue() {} },
    focalLengthOnly: { checked: false },
});
vm.runInContext(profileHandler, profile);
profile.onTelemetry_loaded(false, 'gyro.mp4', '', { camera_identifier: { brand: 'Wrong', model: 'Source' } });
assert.deepEqual(JSON.parse(JSON.stringify(profile.root.detectedCamera)), { brand: 'Manual', model: 'Override', lens: 'Custom' },
    'separate gyro metadata must not replace the video camera prefill');
assert.equal(profile.root.applyCount, 0, 'separate gyro metadata must not reapply camera selectors');
profile.onTelemetry_loaded(true, 'clip.mp4', 'Sony', { camera_identifier: { brand: 'Sony', model: 'ILCE-7M4', lens_model: 'FE 24-70mm F2.8 GM' } });
assert.deepEqual(JSON.parse(JSON.stringify(profile.root.detectedCamera)), { brand: 'Sony', model: 'ILCE-7M4', lens: 'FE 24-70mm F2.8 GM' });
assert.equal(profile.root.applyCount, 1, 'main-video camera identity is applied');

console.log('Camera metadata prefill regressions passed');
