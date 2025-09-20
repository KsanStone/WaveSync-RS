@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5)
    );
    let pos = positions[idx];
    return vec4<f32>(pos, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) coord_in: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(coord_in.x, coord_in.y, 0.0, 1.0);
}
