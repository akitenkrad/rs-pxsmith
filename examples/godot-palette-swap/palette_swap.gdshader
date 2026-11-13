shader_type canvas_item;

uniform sampler2D palettes : filter_nearest;
uniform int row = 0;

void fragment() {
    vec4 idx = texture(TEXTURE, UV);
    if (idx.a < 0.5) { discard; }
    int i = int(round(idx.r * 255.0));
    COLOR = texelFetch(palettes, ivec2(i, row), 0);
}
