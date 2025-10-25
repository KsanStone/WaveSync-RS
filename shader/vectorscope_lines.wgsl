struct Uniforms {
    fill_color: vec4<f32>,
    decay_factor: f32,
    write_factor: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(v_in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(uniforms.write_factor * v_in.length_inv, 0.0, 0.0, 0.0);
}

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) length_inv: f32,
};

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) length_inv: f32
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.pos = vec4<f32>(input.position, 0.0, 1.0);
    out.length_inv = input.length_inv;
    return out;
}