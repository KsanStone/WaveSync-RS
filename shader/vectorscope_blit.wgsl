@group(0) @binding(0) var intensity_tex : texture_storage_2d<r32float, read>;

struct Uniforms {
    decay_factor: f32,
    write_factor: f32,
    fill_color: vec4<f32>,
};

@group(0) @binding(1)
var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    // Pass position for rasterization
    out.pos = vec4<f32>(position, 0.0, 1.0);
    // Map clip space (-1..1) to UVs (0..1)
    out.uv = position * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Get texture size
    let dims: vec2<u32> = textureDimensions(intensity_tex);
    let tex_coord: vec2<i32> = vec2<i32>(uv * vec2<f32>(dims));

    let value = textureLoad(intensity_tex, tex_coord).x;

    // base color (swapped if needed)
    let col = vec3<f32>(uniforms.fill_color.z, uniforms.fill_color.y, uniforms.fill_color.x);

    var out_color: vec3<f32>;
    if value <= 1.0 {
        out_color = col * value;
    } else {
        // logarithmic approach to white
        // adjust the divisor to control how fast it approaches white
        let log_val = log2(value) / 12.0;
        let t = min(log_val, 1.0);
        out_color = mix(col, vec3<f32>(1.0), t);
    }

    let clamped_val = max(min(value, 1.0), 0.0);
    return vec4<f32>(out_color, clamped_val);
}


