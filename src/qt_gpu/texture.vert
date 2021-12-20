#version 440

layout(location = 0) in vec4 position;
layout(location = 1) in vec2 texcoord;
layout(location = 0) out vec2 v_texcoord;
layout(std140, binding = 0) uniform buf {
    mat4 mvp;
    int flip;
} ubuf;
out gl_PerVertex { vec4 gl_Position; };

void main() {
    v_texcoord = vec2(texcoord.x, texcoord.y);
    if (ubuf.flip != 0)
        v_texcoord.y = 1.0 - v_texcoord.y;
    gl_Position = ubuf.mvp * position;
}
