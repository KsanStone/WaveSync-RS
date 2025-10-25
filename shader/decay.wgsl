@group(0) @binding(0) var intensity_tex : texture_storage_2d<r32float, read_write>;

struct Uniforms {
    fill_color: vec4<f32>,
    decay_factor: f32,
    write_factor: f32,
};

@group(0) @binding(1)
var<uniform> uniforms: Uniforms;

@compute @workgroup_size(16, 16)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(intensity_tex);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }

    let color = textureLoad(intensity_tex, vec2<i32>(id.xy)).x;
    textureStore(intensity_tex, vec2<i32>(id.xy), vec4<f32>(color * uniforms.decay_factor, 0.0, 0.0, 0.0));
}
