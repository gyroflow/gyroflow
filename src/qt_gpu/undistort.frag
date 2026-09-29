// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: initUndistortRectifyMap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567

#version 420

layout(location = 0) in vec2 v_texcoord;
layout(location = 0) out vec4 fragColor;

layout(binding = 1) uniform sampler2D texIn;

layout(std140, binding = 2) uniform KernelParams {
    int width;             // 4
    int height;            // 8
    int stride;            // 12
    int output_width;      // 16
    int output_height;     // 4
    int output_stride;     // 8
    int matrix_count;      // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    int interpolation;     // 16
    int background_mode;   // 4
    int flags;             // 8
    int bytes_per_pixel;   // 12
    int pix_element_count; // 16
    vec4 background;    // 16
    vec2 f;             // 8  - focal length in pixels
    vec2 c;             // 16 - lens center
    vec4 k1, k2, k3, k4, k5, k6; // 16 x 6 - distortion coefficients
    float fov;          // 4
    float field_limit;  // 8 - the largest ray angle the lens model can be asked for, in radians
    float lens_correction_amount;   // 12
    float input_vertical_stretch;   // 16
    float input_horizontal_stretch; // 4
    float background_margin;        // 8
    float background_margin_feather;// 12
    float canvas_scale;             // 16
    float input_rotation;           // 4
    float output_rotation;          // 8
    vec2 translation2d;             // 16
    vec4 translation3d;             // 16
    ivec4 source_rect;              // 16 - x, y, w, h - unused in this kernel
    ivec4 output_rect;              // 16 - x, y, w, h - unused in this kernel
    vec4 digital_lens_params[4];    // 16,16,16,16
    vec4 safe_area_rect;            // 16
    float max_pixel_value;          // 4
    int distortion_model;           // 8
    int digital_lens;               // 12
    float pixel_value_limit;        // 16
    float light_refraction_coefficient; // 4
    int plane_index;                // 8
    int output_projection;          // 12
    float reserved2;                // 16
    vec4 ewa_coefs_p;               // 16
    vec4 ewa_coefs_q;               // 16
} params;

LENS_MODEL_FUNCTIONS;

layout(binding = 3) uniform sampler2D texParams;
layout(binding = 4) uniform sampler2D texCanvas;
layout(binding = 5) uniform sampler2D texMeshData;

const vec4 colors[9] = vec4[9](
    vec4(0.0,   0.0,   0.0,     0.0), // None
    vec4(255.0, 0.0,   0.0,   255.0), // Red
    vec4(0.0,   255.0, 0.0,   255.0), // Green
    vec4(0.0,   0.0,   255.0, 255.0), // Blue
    vec4(254.0, 251.0, 71.0,  255.0), // Yellow
    vec4(200.0, 200.0, 0.0,   255.0), // Yellow2
    vec4(255.0, 0.0,   255.0, 255.0), // Magenta
    vec4(0.0,   128.0, 255.0, 255.0), // Blue2
    vec4(0.0,   200.0, 200.0, 255.0)  // Blue3
);
const float alphas[4] = float[4](1.0, 0.75, 0.50, 0.25);
void draw_pixel(inout vec4 out_pix, float x, float y, bool isInput) {
    if (!bool(params.flags & 8)) { // Drawing not enabled
        return;
    }
    int width = max(params.width, params.output_width);
    int height = max(params.height, params.output_height);

    int data = int(ceil(texture(texCanvas, vec2(x / width, y / height)).r * 255.0));
    if (data > 0) {
        int color = (data & 0xF8) >> 3;
        int alpha = (data & 0x06) >> 1;
        int stage = data & 1;
        if (((stage == 0 && isInput) || (stage == 1 && !isInput)) && color < 9) {
            vec4 colorf = colors[color] / 255.0;
            float alphaf = alphas[alpha];
            out_pix = colorf * alphaf + out_pix * (1.0 - alphaf);
            out_pix.a = 1.0;
        }
    }
}
void draw_safe_area(inout vec4 out_pix, float x, float y) {
    bool isSafeArea = x >= params.safe_area_rect.x && x <= params.safe_area_rect.z &&
                      y >= params.safe_area_rect.y && y <= params.safe_area_rect.w;
    if (!isSafeArea) {
        out_pix.x *= 0.5;
        out_pix.y *= 0.5;
        out_pix.z *= 0.5;
        bool isBorder = x >= params.safe_area_rect.x - 5.0 && x <= params.safe_area_rect.z + 5.0 &&
                        y >= params.safe_area_rect.y - 5.0 && y <= params.safe_area_rect.w + 5.0;
        if (isBorder) {
            out_pix.x *= 0.5;
            out_pix.y *= 0.5;
            out_pix.z *= 0.5;
        }
    }
}

float get_param(float row, float idx) {
    int size = bool(params.flags & 16)? params.width : params.height;
    return texture(texParams, vec2(idx / 15.0, row / float(size - 1))).r;
}

float map_coord(float x, float in_min, float in_max, float out_min, float out_max) {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

// The pipeline carries a ray as an angle vector: |.| is the angle from the axis in radians, the direction
// is the azimuth. See stabilization/projection.rs - nothing diverges at 90° the way a z=1 plane point does
vec2 ray_unproject(vec2 n, int proj) {
    float r = length(n);
    if (r < 1e-12) return vec2(0.0, 0.0);
    float theta = atan(r);                                              // rectilinear
    if      (proj == 1) theta = 2.0 * atan(r * 0.5);                    // stereographic
    else if (proj == 2) theta = r;                                      // equidistant
    else if (proj == 3) theta = 2.0 * asin(min(r * 0.5, 1.0));          // equisolid angle
    else if (proj == 4) theta = asin(min(r, 1.0));                      // orthographic
    return n * (theta / r);
}
// Where a ray of angle `theta` lands on the output plane, under the output projection
float ray_radius(float theta, int proj) {
    if (proj == 1) return 2.0 * tan(theta * 0.5);
    if (proj == 2) return theta;
    if (proj == 3) return 2.0 * sin(theta * 0.5);
    if (proj == 4) return sin(theta);
    return theta < 1.5707? tan(theta) : 1e9;
}
// `(1-a)*R(theta) + a*P(theta) - t`: the lens-correction blend (see stabilization/projection.rs), minus
// where this pixel is. `R` carries the refraction, because the render applies it on this side too
float blend_residual(float theta, vec2 u, float t, float a) {
    float th = theta;
    if (params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0) {
        th = asin(clamp(sin(th) * params.light_refraction_coefficient, -1.0, 1.0));
    }
    float s = sin(th), c = cos(th);
    vec2 p = distort_point(u.x * s, u.y * s, c);
    return (1.0 - a) * length(p) + a * ray_radius(theta, params.output_projection) - t;
}
// The ray angle that lands at a given output radius, inverse of `ray_radius`
float ray_theta(float r, int proj) {
    if (proj == 1) return 2.0 * atan(r * 0.5);
    if (proj == 2) return r;
    if (proj == 3) return 2.0 * asin(min(r * 0.5, 1.0));
    if (proj == 4) return asin(min(r, 1.0));
    return atan(r);
}
vec3 ray_to_dir(vec2 ray) {
    float theta = length(ray);
    if (theta < 1e-12) return vec3(ray.x, ray.y, 1.0);
    return vec3(ray * (sin(theta) / theta), cos(theta));
}

vec2 rotate_and_distort(vec2 ray, float idx) {
    vec3 d = ray_to_dir(ray);
    float _x = (d.x * get_param(idx, 0)) + (d.y * get_param(idx, 1)) + (d.z * get_param(idx, 2)) + params.translation3d.x;
    float _y = (d.x * get_param(idx, 3)) + (d.y * get_param(idx, 4)) + (d.z * get_param(idx, 5)) + params.translation3d.y;
    float _w = (d.x * get_param(idx, 6)) + (d.y * get_param(idx, 7)) + (d.z * get_param(idx, 8)) + params.translation3d.z;

    {
        float rxy = length(vec2(_x, _y));
        // The ray's angle in the source camera: past the lens's own field, or past a fold of its
        // calibration, it has no image at all
        float theta = atan(rxy, _w);
        if (params.field_limit > 0.0 && theta > params.field_limit) {
            return vec2(-99999.0, -99999.0);
        }

        // Refraction (underwater): Snell's law on the angle itself, exact for any field. Past the critical angle - or behind the flat port - no ray enters the housing at all: Snell's window ends there
        if (params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 && rxy > 1e-12) {
            float sin_d = sin(theta) * params.light_refraction_coefficient;
            if (sin_d >= 1.0 || _w <= 0.0) return vec2(-99999.0, -99999.0);
            float theta_d = asin(sin_d);
            float s = sin(theta_d) / rxy;
            _x *= s; _y *= s; _w = cos(theta_d);
        }

        vec2 uv = distort_point(_x, _y, _w);
        // Focus breathing: this row's magnification of the source image
        float bz = get_param(idx, 14);
        if (bz > 0.0) uv *= bz;
        uv = params.f * uv + params.c;

        if (get_param(idx, 9) != 0.0 || get_param(idx, 10) != 0.0 || get_param(idx, 11) != 0.0 || get_param(idx, 12) != 0.0 || get_param(idx, 13) != 0.0) {
            float ang_rad = get_param(idx, 11);
            float cos_a = cos(-ang_rad);
            float sin_a = sin(-ang_rad);
            // The camera applies the sensor roll before the sensor/lens shift, so undo the shift first and then the roll
            uv -= params.c;
            uv = vec2(uv.x - get_param(idx, 9) + get_param(idx, 12), uv.y - get_param(idx, 10) + get_param(idx, 13));
            uv = vec2(
                cos_a * uv.x - sin_a * uv.y,
                sin_a * uv.x + cos_a * uv.y
            );
            uv += params.c;
        }
        uv = process_coord(uv, idx);

        if (bool(params.flags & 2)) { // Has digital lens
            uv = digital_distort_point(uv);
        }

        if (params.input_horizontal_stretch > 0.001) { uv.x /= params.input_horizontal_stretch; }
        if (params.input_vertical_stretch   > 0.001) { uv.y /= params.input_vertical_stretch; }

        return uv;
    }
    return vec2(-99999.0, -99999.0);
}

vec2 rotate_point(vec2 pos, float angle, vec2 origin, vec2 origin2) {
     return vec2(cos(angle) * (pos.x - origin.x) - sin(angle) * (pos.y - origin.y) + origin2.x,
                 sin(angle) * (pos.x - origin.x) + cos(angle) * (pos.y - origin.y) + origin2.y);
}
void main() {
    vec2 texPos = v_texcoord.xy * vec2(params.output_width, params.output_height) + params.translation2d;
    vec2 outPos = v_texcoord.xy * vec2(params.output_width, params.output_height);

    if (bool(params.flags & 4)) { // Fill with background
        fragColor = params.background;
        return;
    }

    ///////////////////////////////////////////////////////////////////
    // Output pixel -> ray. The output projection says which ray this pixel stands for; below full lens
    // correction the ray the source lens itself saw at that pixel is mixed in, and since rays are angle
    // vectors that mix is a mix of the two ANGLES
    bool lens_undistort_failed = false;
    float stretch = params.input_horizontal_stretch > 0.01? 1.0 / params.input_horizontal_stretch : 1.0;
    vec2 out_c = vec2(params.output_width / 2.0, params.output_height / 2.0);
    vec2 out_f = params.f * stretch / params.fov;
    float a = params.lens_correction_amount;
    vec2 n_raw = (texPos - out_c) / out_f;
    vec2 n = n_raw;
    if (bool(params.flags & 2) && a < 1.0) { // Has digital lens
        // Apply the digital warp in the UN-zoomed (fov=1) frame so it's FOV-independent. The warp is a
        // frame-relative pixel map; evaluating it on post-zoom pixels made the corrected shape (and the
        // adaptive-zoom bounding box from it) depend on the zoom. Un-zoom -> warp -> re-zoom.
        vec2 uz = (texPos - out_c) * params.fov + out_c;
        vec2 dp = digital_undistort_point(uz);
        n = ((dp - out_c) / params.fov) / out_f;
    }
    vec2 ray = ray_unproject(n_raw, params.output_projection);
    if (a < 1.0) {
        // With a digital warp the two legs read different planes, so the target is mixed the same way they
        // are; without one `n` is `n_raw` and this is just the pixel itself
        vec2 nb = n * (1.0 - a) + n_raw * a;
        float t = length(nb);
        if (t > 1e-9) {
            vec2 u = nb / t;
            float lo = length(ray);
            vec2 src = undistort_point(n);
            bool has_src = src.x > -99998.0;
            lens_undistort_failed = !has_src && !(params.field_limit > 0.0);
            if (!lens_undistort_failed) {
                // Past the edge of its own image the model has no inverse, but the lens still reaches to
                // its field limit and the blend may well land inside `t` before then
                // Under water the lens's angles are the housing's; the bracket is in the water's, the inverse of
                // Snell's law away - and no ray past the flat port's 90 degrees ever enters the housing
                bool refr = params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0;
                float hi = has_src? length(src) : params.field_limit;
                if (refr) hi = asin(clamp(sin(min(hi, 1.5707963)) / params.light_refraction_coefficient, -1.0, 1.0));
                bool edge = !has_src;
                // ... and the root cannot be past the angle at which the output projection alone would already have
                // used up `t` (`a·P(θ) <= t` at the root), and pinning the bracket there keeps both residuals
                // of the order of `t`. Without it a lens that sees past 90 degrees hands `P`'s "no image" sentinel to
                // the solve, and a secant cannot move against 1e9: the blend then silently stalled at the
                // fully corrected angle outside a circle at `R(90 degrees)`
                if (a > 0.0) hi = min(hi, ray_theta(t / a, params.output_projection));
                if (a <= 0.0) {
                    lens_undistort_failed = !has_src;
                    // Exactly the source lens's own ray, tangential terms and all - with the refraction
                    // rotate_and_distort applies on the way out undone here, or the round trip isn't one
                    ray = src;
                    if (refr) {
                        float th = length(src);
                        if (th > 1e-12) ray = src * (asin(clamp(sin(min(th, 1.5707963)) / params.light_refraction_coefficient, -1.0, 1.0)) / th);
                    }
                } else {
                    float l = min(lo, hi), h = max(lo, hi);
                    // The lens ends at its field limit - under water, at Snell's window - and the projection's own
                    // angle may well lie past it; the bracket ends there too, and past it is background
                    float cap = params.field_limit > 0.0? params.field_limit : 3.1415927;
                    if (refr) cap = asin(clamp(sin(min(cap, 1.5707963)) / params.light_refraction_coefficient, -1.0, 1.0));
                    if (h > cap) { h = cap; l = min(l, cap); edge = true; }
                    float gl = blend_residual(l, u, t, a);
                    float gh = blend_residual(h, u, t, a);
                    // Near the axis every projection agrees, so both ends of the bracket are a difference of
                    // near-equal numbers and the sign of `g` there is float noise - GPU transcendentals make
                    // that band tens of pixels wide. An end that IS the source lens's own answer always has a
                    // root beside it, so the end itself is the answer; only when the end is the field limit -
                    // or Snell's window - has the lens really run out, and the pixel is background
                    lens_undistort_failed = gh < 0.0 && edge;
                    if (!lens_undistort_failed) {
                        float theta = h;
                        if (gh >= 0.0) {
                            // The projection's own angle overshoots - refraction magnifies the source leg - so bracket from the axis, where the residual is -t
                            if (gl >= 0.0) { l = 0.0; gl = -t; }
                            // Regula falsi, Illinois variant: the bracket is tight (the two projections'
                            // own answers), and halving the stale end keeps it from crawling in from one side
                            int side = 0;
                            for (int i = 0; i < 6; ++i) {
                                float d = gh - gl;
                                theta = abs(d) > 1e-12? clamp(l - gl * (h - l) / d, l, h) : 0.5 * (l + h);
                                float gt = blend_residual(theta, u, t, a);
                                if (gt < 0.0) { l = theta; gl = gt; if (side == -1) gh *= 0.5; side = -1; }
                                else          { h = theta; gh = gt; if (side ==  1) gl *= 0.5; side =  1; }
                            }
                        }
                        // The source lens's own ray is off its radius by the tangential terms; that offset belongs to the
                        // source leg, so it fades out with it. It is measured against `n`, the point that leg reads: a
                        // digital warp moves `n` off `u`, and that is no offset of the lens's
                        vec2 dir = u;
                        if (has_src) {
                            float sl = length(src), nl = length(n);
                            if (sl > 1e-12 && nl > 1e-12) {
                                vec2 dd = u + (1.0 - a) * (src / sl - n / nl);
                                float dl = length(dd);
                                if (dl > 1e-12) dir = dd / dl;
                            }
                        }
                        ray = dir * theta;
                    }
                }
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    float sy = texPos.y;
    if (bool(params.flags & 16)) { // Horizontal RS
        sy = min(params.width, max(0, floor(0.5 + texPos.x)));
    } else {
        sy = min(params.height, max(0, floor(0.5 + texPos.y)));
    }
    if (params.matrix_count > 1) {
        float idx = params.matrix_count / 2.0; // Use middle matrix
        vec2 uv = rotate_and_distort(ray, idx);
        if (uv.x > -99998.0) {
            if (bool(params.flags & 16)) { // Horizontal RS
                sy = min(params.width, max(0, floor(0.5 + uv.x)));
            } else {
                sy = min(params.height, max(0, floor(0.5 + uv.y)));
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    float idx = min(sy, params.matrix_count - 1.0);

    vec2 uv = rotate_and_distort(ray, idx);
    vec2 frame_size = vec2(params.width, params.height);
    if (params.input_rotation != 0.0) {
        float rotation = params.input_rotation * (3.1415926535897 / 180.0);
        vec2 size = frame_size;
        frame_size = abs(round(rotate_point(size, rotation, vec2(0.0, 0.0), vec2(0.0, 0.0))));
        uv = rotate_point(uv, rotation, size / vec2(2.0), frame_size / vec2(2.0));
    }

    if (!lens_undistort_failed && uv.x > -99998.0) {
        if (params.background_mode == 1) { // edge repeat
            uv = max(vec2(0, 0), min(vec2(params.width - 1, params.height - 1), uv));
        } else if (params.background_mode == 2) { // edge mirror
            float width3 = (params.width - 2);
            float height3 = (params.height - 2);
            if (uv.x > width3)  uv.x = width3  - (uv.x - width3);
            if (uv.x < 2)       uv.x = 2 + params.width - (width3  + uv.x);
            if (uv.y > height3) uv.y = height3 - (uv.y - height3);
            if (uv.y < 2)       uv.y = 2 + params.height - (height3 + uv.y);
        } else if (params.background_mode == 3) { // margin with feather
            float widthf  = (params.width  - 1);
            float heightf = (params.height - 1);

            float feather = max(0.0001, params.background_margin_feather * heightf);
            vec2 pt2 = uv;
            float alpha = 1.0;
            if ((uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather)) {
                alpha = max(0.0, min(1.0, min(min(widthf - uv.x, heightf - uv.y), min(uv.x, uv.y)) / feather));
                pt2 /= vec2(widthf, heightf);
                pt2 = ((pt2 - 0.5) * (1.0 - params.background_margin)) + 0.5;
                pt2 *= vec2(widthf, heightf);
            }

            vec4 c1 = texture(texIn, vec2(uv.x / params.width, uv.y / params.height));
            vec4 c2 = texture(texIn, vec2(pt2.x / params.width, pt2.y / params.height));
            fragColor = c1 * alpha + c2 * (1.0 - alpha);
            fragColor.a = 1.0;
            if (!((pt2.x >= 0 && pt2.x < params.width) && (pt2.y >= 0 && pt2.y < params.height))) {
                fragColor = params.background;
            }
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            draw_safe_area(fragColor, outPos.x, outPos.y);
            return;
        }

        if ((uv.x >= 0 && uv.x < frame_size.x) && (uv.y >= 0 && uv.y < frame_size.y)) {
            fragColor = texture(texIn, vec2(uv.x / frame_size.x, uv.y / frame_size.y));
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            draw_safe_area(fragColor, outPos.x, outPos.y);
            return;
        }
    }

    fragColor = params.background;
    draw_pixel(fragColor, outPos.x, outPos.y, false);
    draw_safe_area(fragColor, outPos.x, outPos.y);
}
