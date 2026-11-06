@group(1) @binding(0) var gradient_lut: texture_2d<f32>;

// All stop evaluation is baked on the CPU. Only interpolate adjacent LUT entries here.
fn sample_gradient(value: f32) -> vec4<f32> {
    let x = clamp(value, 0.0, 1.0) * f32(textureDimensions(gradient_lut).x - 1u);
    let lo = i32(floor(x));
    let hi = min(lo + 1, i32(textureDimensions(gradient_lut).x) - 1);
    return mix(textureLoad(gradient_lut, vec2(lo, 0), 0),
               textureLoad(gradient_lut, vec2(hi, 0), 0), fract(x));
}
