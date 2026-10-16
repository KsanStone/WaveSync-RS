// ========================= VERTEX SHADER =========================
struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) frag_coord: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = vec4<f32>(position, 0.0, 1.0);
    out.frag_coord = position * 0.5 + vec2<f32>(0.5);
    return out;
}

// ========================= FRAGMENT SHADER =========================
struct Uniforms {
    width: i32,
    height: i32,
    head_offset: i32,
    buffer_size: i32,
    is_vertical: i32,
    scroll_offset: f32,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// 2D texture with your main data
@group(0) @binding(1) var buffer_tex: texture_2d<f32>;
// Gradient stored as a one-row 2D texture for OpenGL/ANGLE compatibility.
@group(0) @binding(2) var gradient_tex: texture_2d<f32>;
// Coordinate map as storage buffer
@group(0) @binding(3) var<storage, read> coord_map: array<i32>;

@fragment
fn fs_main(@location(0) frag_coord: vec2<f32>) -> @location(0) vec4<f32> {
    let is_vertical = uniforms.is_vertical == 1;
    let time_uv = clamp(select(frag_coord.x, frag_coord.y, is_vertical), 0.0, 1.0);
    let frequency_uv = clamp(select(frag_coord.y, frag_coord.x, is_vertical), 0.0, 1.0);
    let capacity = uniforms.buffer_size;
    // Extra rows retain history behind the visible oldest edge while scrolling.
    let visible_rows = max(capacity - 4, 2);
    let age = clamp((1.0 - time_uv) * f32(visible_rows - 1) + uniforms.scroll_offset, 0.0, f32(capacity - 1));
    let age0 = i32(floor(age));
    let age1 = min(age0 + 1, capacity - 1);
    // Wrap integer row indices only. Never float-modulo screen coordinates.
    let row0 = (uniforms.head_offset + capacity - age0) % capacity;
    let row1 = (uniforms.head_offset + capacity - age1) % capacity;
    let row_mix = fract(age);

    let map_count = min(select(uniforms.height, uniforms.width, is_vertical), i32(arrayLength(&coord_map)) - 1);
    let mapped_pos = clamp(i32(floor(frequency_uv * f32(map_count))), 0, map_count - 1);
    let bin_count = i32(textureDimensions(buffer_tex, 0).x) * 4;
    let mapped_start = clamp(coord_map[mapped_pos], 0, bin_count - 1);
    let mapped_end = clamp(max(coord_map[mapped_pos + 1] - 1, mapped_start), mapped_start, bin_count - 1);
    var value0 = 0.0;
    var value1 = 0.0;
    for (var i = mapped_start; i <= mapped_end; i = i + 1) {
        let px_idx = i / 4;
        let channel_idx = i % 4;
        let texel0 = textureLoad(buffer_tex, vec2<i32>(px_idx, row0), 0);
        let texel1 = textureLoad(buffer_tex, vec2<i32>(px_idx, row1), 0);
        let channels0 = array<f32, 4>(texel0.x, texel0.y, texel0.z, texel0.w);
        let channels1 = array<f32, 4>(texel1.x, texel1.y, texel1.z, texel1.w);
        value0 = max(channels0[channel_idx], value0);
        value1 = max(channels1[channel_idx], value1);
    }
    let val = mix(value0, value1, row_mix);

    // Map value to gradient using textureLoad instead of textureSample
    let gradient_index = i32(val * f32(textureDimensions(gradient_tex, 0).x - 1));
    let color = textureLoad(gradient_tex, vec2<i32>(gradient_index, 0), 0);

    // The gradient originates from egui's premultiplied Color32. Convert it back for the
    // regular alpha blend used by this render pass. Fully transparent floor pixels leave the
    // system backdrop untouched.
    if color.a <= 0.0001 {
        discard;
    }
    return vec4<f32>(color.rgb / color.a, color.a);
}
