// SPDX-License-Identifier: GPL-3.0-or-later
#version 440
layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;
layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float brightness;
    float contrast;
    float lutSize;
};
layout(binding = 1) uniform sampler2D source;
layout(binding = 2) uniform sampler2D lutTexture;

float entryChannel(int index) {
    ivec2 dimensions = textureSize(lutTexture, 0);
    ivec2 first = ivec2(index % dimensions.x, index / dimensions.x);
    ivec2 second = ivec2((index + 1) % dimensions.x, (index + 1) / dimensions.x);
    uvec3 low = uvec3(round(texelFetch(lutTexture, first, 0).rgb * 255.0));
    uint high = uint(round(texelFetch(lutTexture, second, 0).r * 255.0));
    return uintBitsToFloat(low.r | (low.g << 8) | (low.b << 16) | (high << 24));
}
vec3 entry(ivec3 p) {
    int n = int(lutSize);
    int i = (p.r + p.g * n + p.b * n * n) * 6;
    return vec3(entryChannel(i), entryChannel(i + 2), entryChannel(i + 4));
}
vec3 applyLut(vec3 rgb) {
    vec3 p = clamp(rgb, 0.0, 1.0) * (lutSize - 1.0);
    ivec3 lo = ivec3(floor(p));
    ivec3 hi = min(lo + ivec3(1), ivec3(int(lutSize) - 1));
    vec3 d = p - vec3(lo);
    // Sort the fractional coordinates; these select the tetrahedron's two
    // intermediate vertices between the lower and upper cube corners.
    ivec3 order;
    if (d.r >= d.g) {
        if (d.g >= d.b) order = ivec3(0, 1, 2);
        else if (d.r >= d.b) order = ivec3(0, 2, 1);
        else order = ivec3(2, 0, 1);
    } else {
        if (d.r >= d.b) order = ivec3(1, 0, 2);
        else if (d.g >= d.b) order = ivec3(1, 2, 0);
        else order = ivec3(2, 1, 0);
    }
    ivec3 a = lo;
    a[order.x] = hi[order.x];
    ivec3 b = a;
    b[order.y] = hi[order.y];
    vec3 v0 = entry(lo), v1 = entry(a), v2 = entry(b), v3 = entry(hi);
    return v0 + d[order.x] * (v1 - v0) + d[order.y] * (v2 - v1) + d[order.z] * (v3 - v2);
}
void main() {
    vec4 pixel = texture(source, qt_TexCoord0);
    vec3 rgb = pixel.a > 0.0 ? pixel.rgb / pixel.a : vec3(0.0);
    if (lutSize >= 2.0) rgb = applyLut(rgb);
    rgb = clamp((rgb - 0.5) * (1.0 + contrast) + 0.5 + brightness, 0.0, 1.0);
    fragColor = vec4(rgb * pixel.a, pixel.a) * qt_Opacity;
}
