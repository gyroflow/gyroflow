/** Types describing the camera‑lens catalogue used by the calibrator UI. */

export interface CameraModel {
    /** Human readable model name, e.g. "Alpha a7 III". */
    name: string;
    /** Sensor size description – "Full Frame", "APS-C", etc. */
    sensorSize: string;
    /** Crop factor relative to full frame (1.0 for full frame). */
    cropFactor: number;
    /** Lens mount identifier, e.g. "E-mount". */
    mount: string;
}

export interface CameraBrand {
    /** Brand name, e.g. "Sony". */
    name: string;
    /** All models belonging to the brand. */
    models: CameraModel[];
}

export interface LensInfo {
    /** Lens manufacturer. */
    brand: string;
    /** Lens model name. */
    model: string;
    /** Mount that the lens uses. */
    mount: string;
    /** Sensor sizes the lens can be used with. */
    compatibleSensors: string[];
}

/** Root object stored in `resources/camera_lens_data.json`. */
export interface CameraLensCatalogue {
    brands: CameraBrand[];
    lenses: LensInfo[];
}
