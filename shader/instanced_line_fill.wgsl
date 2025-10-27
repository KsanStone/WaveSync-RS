struct VertexInput {
    @location(0) pos: vec2<f32>,
    // x1 y1 x2 y2
    @location(1) rect: vec4<f32>
};

struct Uniforms {
    end_color: vec4<f32>,
    start_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> colors: Uniforms;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) height: f32
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let y_floor = min(input.rect.y, input.rect.w) - 2.0;

    let x = input.rect.x + input.pos.x * (input.rect.z - input.rect.x);
    var y = 0.0;
    if (input.pos.y != 0.0) {
        if (input.pos.x == 0) {
            y = -1.0 + input.pos.y * (input.rect.y + 1.0);
        } else {
            y = -1.0 + input.pos.y * (input.rect.w + 1.0);
        }
    } else {
        y = y_floor;
    }

    out.pos = vec4(x, y, 0.0, 1.0);
    if (input.pos.x == 0) {
        out.height = input.rect.y;
    } else {
        out.height = input.rect.w;
    }
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let factor = (input.height + 1) * 0.5;
    let inv_factor = 1.0 - factor;
    let color = colors.end_color * factor + colors.start_color * inv_factor;
    return vec4(color.xyz, 0.5);
}
