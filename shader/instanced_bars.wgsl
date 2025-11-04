struct VertexInput {
    @location(0) pos: vec2<f32>,
    @location(1) height: f32,
    @location(2) x_1: f32,
    @location(3) x_2: f32,
};

struct Uniforms {
    end_color: vec4<f32>,
    start_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> colors: Uniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) height: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let x = input.x_1 + input.pos.x * (input.x_2 - input.x_1);
    let y = -1.0 + input.pos.y * (input.height + 1.0);

    out.clip_pos = vec4(x, y, 0.0, 1.0);
    out.height = input.height;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let factor = (input.height + 1) * 0.5;
    let inv_factor = 1.0 - factor;
    return colors.end_color * factor + colors.start_color * inv_factor;
}
