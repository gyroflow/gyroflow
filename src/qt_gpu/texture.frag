#version 440

layout(location = 0) in vec2 v_texcoord;
layout(location = 0) out vec4 fragColor;
layout(std140, binding = 0) uniform buf {
    mat4 mvp;
    int flip;
} ubuf;
layout(binding = 1) uniform sampler2D tex;

void main() {
    vec4 c = texture(tex, v_texcoord);
    fragColor = vec4(c.rgb * c.a, c.a);
}
