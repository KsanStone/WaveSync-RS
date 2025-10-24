struct Uniforms {
    decay_factor: f32,
    write_factor: f32,
    fill_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(v_in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(uniforms.write_factor * v_in.length);
}

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) length: f32,
};

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) length: f32
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.pos = vec4<f32>(input.position, 0.0, 1.0);
    out.length = input.length;
    return out;
}