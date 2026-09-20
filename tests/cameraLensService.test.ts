import {
    getCameraBrands,
    getCameraModels,
    getLensesByMount,
    getLensesForCameraModel,
    getCompatibleCamerasForLens,
    getCropFactor,
    validateSetup,
} from '../src/services/cameraLensService';

describe('CameraLensService', () => {
    test('should list all known brands', () => {
        const brands = getCameraBrands();
        expect(brands).toContain('Sony');
        expect(brands).toContain('Canon');
    });

    test('should return models for a brand', () => {
        const sonyModels = getCameraModels('Sony');
        const names = sonyModels.map((m) => m.name);
        expect(names).toContain('Alpha a7 III');
        expect(names).toContain('Alpha a6400');
    });

    test('should filter lenses by mount', () => {
        const eMountLenses = getLensesByMount('E-mount');
        expect(eMountLenses.length).toBeGreaterThan(0);
        expect(eMountLenses.every((l) => l.mount === 'E-mount')).toBe(true);
    });

    test('should return lenses compatible with a camera model', () => {
        const lenses = getLensesForCameraModel('Alpha a7 III');
        // Sony FE 24-70mm is compatible with full‑frame E‑mount cameras
        const names = lenses.map((l) => l.model);
        expect(names).toContain('FE 24-70mm f/2.8 GM');
    });

    test('should return compatible cameras for a lens', () => {
        const cameras = getCompatibleCamerasForLens('RF 24-105mm f/4L IS USM');
        const names = cameras.map((c) => c.name);
        expect(names).toContain('EOS R5');
        // Should not contain APS‑C models because the lens is full‑frame only
        expect(names).not.toContain('EOS M50');
    });

    test('should return correct crop factor', () => {
        expect(getCropFactor('Alpha a6400')).toBeCloseTo(1.5);
        expect(getCropFactor('EOS R5')).toBeCloseTo(1.0);
        expect(getCropFactor('Non‑existent')).toBeNull();
    });

    test('validateSetup enforces required fields', () => {
        // valid non‑zoom
        expect(validateSetup('Sony', 'Alpha a7 III', 'E-mount', false)).toBe(true);
        // missing brand
        expect(validateSetup(null, 'Alpha a7 III', 'E-mount', false)).toBe(false);
        // missing model
        expect(validateSetup('Sony', null, 'E-mount', false)).toBe(false);
        // missing mount
        expect(validateSetup('Sony', 'Alpha a7 III', null, false)).toBe(false);
        // zoom without focal length
        expect(validateSetup('Sony', 'Alpha a7 III', 'E-mount', true)).toBe(false);
        // zoom with focal length
        expect(validateSetup('Sony', 'Alpha a7 III', 'E-mount', true, 35)).toBe(true);
    });
});
