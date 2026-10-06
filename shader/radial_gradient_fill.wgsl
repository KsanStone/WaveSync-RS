struct VertexInput {
    @location(0) pos: vec2<f32>,
};

struct Uniforms {
    end_color: vec4<f32>,
    start_color: vec4<f32>,
    gradient_center: vec2<f32>,
    gradient_radius: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) gl_pos: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    out.clip_pos = vec4<f32>(input.pos, 0.0, 1.0);
    out.gl_pos = input.pos;

    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let distance_from_center = distance(
        input.gl_pos,
        uniforms.gradient_center
    );

    let factor = clamp(
        distance_from_center / uniforms.gradient_radius,
        0.0,
        1.0
    );

    return vec4(mix(
        uniforms.start_color,
        uniforms.end_color,
        factor
    ).xyz, 0.5);
}