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
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// 2D texture with your main data
@group(0) @binding(1) var buffer_tex: texture_2d<f32>;
// Gradient as 1D non-filterable texture
@group(0) @binding(2) var gradient_tex: texture_1d<f32>;
// Coordinate map as storage buffer
@group(0) @binding(3) var<storage, read> coord_map: array<i32>;

@fragment
fn fs_main(@location(0) frag_coord: vec2<f32>) -> @location(0) vec4<f32> {
    let is_vertical = uniforms.is_vertical == 1;
    let relevant_size = f32(select(uniforms.width, uniforms.height, is_vertical));
    let per_px = f32(uniforms.buffer_size) / relevant_size;
    let frag_pos_in_buffer = select(frag_coord.x, frag_coord.y, is_vertical) * f32(uniforms.buffer_size - 1);

    let pixels_x = frag_coord.x * f32(uniforms.width);
    let pixels_y = frag_coord.y * f32(uniforms.height);

    let head_adjusted = frag_pos_in_buffer + f32(uniforms.head_offset);
    let buff_pos = i32((head_adjusted - (head_adjusted % per_px) + per_px + 1.0) % f32(uniforms.buffer_size));

    let mapped_pos = i32(select(pixels_y, pixels_x, is_vertical));
    let mapped_start = coord_map[mapped_pos];
    let mapped_end = max(coord_map[mapped_pos + 1] - 1, mapped_start);

    var val = 0.0;
    for (var i = mapped_start; i <= mapped_end; i = i + 1) {
        // We are packing 4 data-points into the ARGB channels of the texel
        // as the texture size is rather limited. This allows us to effectively
        // quadruple the usable texture size.
        let px_idx = i / 4;
        let channel_idx = i % 4;
        let texel = textureLoad(buffer_tex, vec2<i32>(px_idx, buff_pos), 0);
        let arr = array<f32, 4>(texel.x, texel.y, texel.z, texel.w);

        val = max(arr[channel_idx], val);
    }

    // Map value to gradient using textureLoad instead of textureSample
    let gradient_index = i32(val * f32(textureDimensions(gradient_tex, 0) - 1));
    let color = textureLoad(gradient_tex, gradient_index, 0);

    return color;
}
