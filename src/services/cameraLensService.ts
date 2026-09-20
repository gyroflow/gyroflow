import type {
    CameraBrand,
    CameraModel,
    LensInfo,
    CameraLensCatalogue,
} from '../types/CameraLens';

// The JSON file is bundled with the front‑end via webpack / Vite.
// Using a static import guarantees that the data is available at runtime
// without needing an async fetch.
import catalogueJson from '../../resources/camera_lens_data.json' assert { type: 'json' };

const catalogue: CameraLensCatalogue = catalogueJson as CameraLensCatalogue;

/**
 * Returns a list of all known camera brands.
 */
export function getCameraBrands(): string[] {
    return catalogue.brands.map((b) => b.name);
}

/**
 * Returns all models for a given brand.
 * @param brandName Exact brand name (case‑sensitive) as stored in the catalogue.
 */
export function getCameraModels(brandName: string): CameraModel[] {
    const brand = catalogue.brands.find((b) => b.name === brandName);
    return brand ? brand.models : [];
}

/**
 * Returns all lens entries for a given mount.
 * @param mount Lens mount identifier, e.g. "E-mount".
 */
export function getLensesByMount(mount: string): LensInfo[] {
    return catalogue.lenses.filter((l) => l.mount === mount);
}

/**
 * Returns lenses compatible with a specific camera model.
 * @param cameraModel Exact camera model name.
 */
export function getLensesForCameraModel(cameraModel: string): LensInfo[] {
    const model = catalogue.brands
        .flatMap((b) => b.models)
        .find((m) => m.name === cameraModel);
    if (!model) return [];

    return catalogue.lenses.filter((lens) =>
        lens.mount === model.mount &&
        lens.compatibleSensors.includes(model.sensorSize)
    );
}

/**
 * Returns a list of camera models that are compatible with a given lens.
 * @param lensModel Exact lens model name.
 */
export function getCompatibleCamerasForLens(lensModel: string): CameraModel[] {
    const lens = catalogue.lenses.find((l) => l.model === lensModel);
    if (!lens) return [];

    return catalogue.brands
        .flatMap((b) => b.models)
        .filter(
            (cam) =>
                cam.mount === lens.mount &&
                lens.compatibleSensors.includes(cam.sensorSize)
        );
}

/**
 * Retrieves the crop factor for a given camera model.
 * Returns `null` if the model cannot be found.
 */
export function getCropFactor(cameraModel: string): number | null {
    const model = catalogue.brands
        .flatMap((b) => b.models)
        .find((m) => m.name === cameraModel);
    return model ? model.cropFactor : null;
}

/**
 * Validates that a camera‑lens setup is sufficiently specific for
 * submission of a new lens profile.
 *
 * Rules enforced (as required by the issue):
 *  - Camera brand and model must be selected (or "Other" with manual entry).
 *  - Lens mount must be known.
 *  - If the lens is a zoom, a focal length must be supplied (handled elsewhere).
 *
 * Returns `true` if the setup passes validation, otherwise `false`.
 */
export function validateSetup(
    brand: string | null,
    model: string | null,
    lensMount: string | null,
    isZoom: boolean,
    focalLength?: number
): boolean {
    if (!brand || !model) {
        // "Other" case is handled by the UI – it must provide a manual string.
        return false;
    }
    if (!lensMount) return false;
    if (isZoom && (!focalLength || focalLength <= 0)) return false;
    return true;
}

/**
 * Helper to retrieve the full catalogue (read‑only).
 */
export function getCatalogue(): Readonly<CameraLensCatalogue> {
    return catalogue;
}
