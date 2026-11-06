// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return pos; }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float fru = ru * (params->k[0] * ru * ru * ru + params->k[1] * ru * ru + params->k[2] * ru + 1.0) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return (float2)(-99999.0f, -99999.0f);
        }

        ru -= fru / (4.0 * params->k[0] * ru * ru * ru + 3.0 * params->k[1] * ru * ru + 2.0 * params->k[2] * ru + 1.0);
    }
    if (ru < 0.0) {
        return (float2)(-99999.0f, -99999.0f);
    }

    // `ru` is the radius in the z=1 plane; the pipeline carries rays as angle vectors
    return pos * (atan(ru) / rd);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    float2 pos = z > 1e-9f? (float2)(x, y) / z : (float2)(x, y) * 1e9f;
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float r = sqrt(ru2);
    float poly3 = params->k[0] * ru2 * r + params->k[1] * ru2 + params->k[2] * r + 1.0;
    return pos * poly3;
}
