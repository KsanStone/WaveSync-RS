@group(0) @binding(0) var intensity_tex : texture_storage_2d<r32float, read_write>;

struct Uniforms {
    decay_factor: f32,
    write_factor: f32,
    fill_color: vec4<f32>,
};

@group(0) @binding(1)
var<uniform> uniforms: Uniforms;

// This is a hacky solution to "render" to a different target
// without either creating a different render pass or
// rasterizing ourselves.
@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(pos.xy);
    let prev = textureLoad(intensity_tex, coord);
    textureStore(intensity_tex, coord, vec4<f32>(prev.x + uniforms.write_factor, 0.0, 0.0, 0.0));
    return vec4<f32>(0.0);
}

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.pos = vec4<f32>(position, 0.0, 1.0);
    return out;
}