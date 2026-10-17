@group(0) @binding(0) var intensity_tex : texture_storage_2d<r32float, read>;

struct Uniforms {
    fill_color: vec4<f32>,
    decay_factor: f32,
    write_factor: f32,
    bleed_strength: f32,
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

// Clamp reads at viewport edges: no wraparound or dark edge stripes.
fn intensity_at(position: vec2<i32>) -> f32 {
    let limit = vec2<i32>(textureDimensions(intensity_tex)) - vec2<i32>(1);
    return max(textureLoad(intensity_tex, clamp(position, vec2<i32>(0), limit)).x, 0.0);
}
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var bloom_sampler: sampler;
@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let tex_coord = vec2<i32>(uv * vec2<f32>(textureDimensions(intensity_tex)));
    var value = intensity_at(tex_coord);
    // Both the direct trace and its optical spill are HDR light. Combine them
    // BEFORE the shared phosphor response: bloom can become white-hot too.
    if uniforms.bleed_strength > 0.0 {
        let light = textureSampleLevel(bloom_tex, bloom_sampler, uv, 0.0).rgb;
        let bloom = dot(light, vec3<f32>(1.0)) * 4.0 * uniforms.bleed_strength;
        // Smooth contrast knee: suppress faint haze quadratically, while bright
        // HDR light approaches its original intensity. No hard spatial cutoff.
        value += bloom * (bloom / (bloom + 0.5));
    }
    let white = clamp(log2(max(value, 1.0)) / 5.0, 0.0, 1.0);
    let color = mix(uniforms.fill_color.rgb, vec3<f32>(1.0), white);
    return vec4<f32>(color, 1.0 - exp(-value));
}
