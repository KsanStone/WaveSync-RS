struct Uniforms {
    color: vec4<f32>,
    viewport: vec2<f32>,
    half_width: f32,
    padding: f32,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
struct Input {
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
    @location(2) start_normal: vec2<f32>,
    @location(3) end_normal: vec2<f32>,
    @location(4) weight: f32,
    @location(5) flags: u32,
};
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) segment_length: f32,
    @location(2) @interpolate(flat) weight: f32,
    @location(3) @interpolate(flat) flags: u32,
};
@vertex
fn vs_main(input: Input, @builtin(vertex_index) index: u32) -> Output {
    // Two triangles, six generated vertices, one instance per segment.
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let corner = corners[index];
    let delta = (input.end - input.start) * uniforms.viewport * 0.5;
    let segment_length = length(delta);
    var tangent = vec2(1.0, 0.0);
    if segment_length > 0.00001 { tangent = delta / segment_length; }
    let normal = vec2(-tangent.y, tangent.x);
    let radius = uniforms.half_width + 0.5;
    let independent = (input.flags & 1u) != 0u;
    var offset = mix(input.start_normal, input.end_normal, corner.x) * corner.y * radius;
    if independent {
        offset = normal * corner.y * radius + tangent * (corner.x * 2.0 - 1.0) * radius;
    } else {
        if corner.x == 0.0 && (input.flags & 2u) != 0u { offset -= tangent * 0.5; }
        if corner.x == 1.0 && (input.flags & 4u) != 0u { offset += tangent * 0.5; }
    }
    var out: Output;
    out.position = vec4(mix(input.start, input.end, corner.x) + offset * 2.0 / uniforms.viewport, 0.0, 1.0);
    out.local = vec2(corner.x * segment_length + dot(offset, tangent), dot(offset, normal));
    out.segment_length = segment_length;
    out.weight = input.weight;
    out.flags = input.flags;
    return out;
}
fn coverage(input: Output) -> f32 {
    var distance = abs(input.local.y);
    if (input.flags & 1u) != 0u {
        let along = max(max(-input.local.x, input.local.x - input.segment_length), 0.0);
        distance = length(vec2(along, input.local.y));
    }
    var value = clamp(uniforms.half_width + 0.5 - distance, 0.0, 1.0);
    if (input.flags & 2u) != 0u { value = min(value, clamp(input.local.x + 0.5, 0.0, 1.0)); }
    if (input.flags & 4u) != 0u { value = min(value, clamp(input.segment_length - input.local.x + 0.5, 0.0, 1.0)); }
    return value;
}
@fragment
fn fs_color(input: Output) -> @location(0) vec4<f32> {
    return vec4(uniforms.color.rgb, uniforms.color.a * coverage(input));
}
@fragment
fn fs_intensity(input: Output) -> @location(0) vec4<f32> {
    // Accumulate coverage in the red channel; alpha is unused by R32Float.
    return vec4(input.weight * coverage(input), 0.0, 0.0, 0.0);
}
