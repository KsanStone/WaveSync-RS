@group(0) @binding(0) var intensity: texture_storage_2d<r32float, read>;
@group(0) @binding(1) var horizontal_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var horizontal_in: texture_2d<f32>;
@group(0) @binding(3) var bloom_out: texture_storage_2d<rgba16float, write>;

// Each source pixel distributes its light across three Gaussian widths.
// Stronger sources progressively shift toward the wider bands, rather than
// growing a fixed halo based on the intensity at the receiving (dark) pixel.
fn source_bands(position: vec2<i32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(intensity));
    if any(position < vec2<i32>(0)) || any(position >= dims) { return vec3<f32>(0.0); }
    let value = max(textureLoad(intensity, position).x, 0.0);
    let light = value * (value / (value + 0.25));
    let spread = value / (value + 1.0);
    let wide = value / (value + 4.0);
    return light * vec3(1.0 - spread, spread * (1.0 - wide), spread * wide);
}
fn kernel(offset: i32) -> vec3<f32> {
    // Half-resolution sigma: approximately 1.6, 3.2 and 6 physical pixels.
    let sigma = vec3<f32>(0.8, 1.6, 3.0);
    let x = f32(offset);
    return exp(vec3<f32>(-0.5 * x * x) / (sigma * sigma));
}
@compute @workgroup_size(8, 8)
fn horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(horizontal_out)) { return; }
    var light = vec3<f32>(0.0);
    var total = vec3<f32>(0.0);
    for (var x = -8; x <= 8; x += 1) {
        let origin = (vec2<i32>(id.xy) + vec2(x, 0)) * 2;
        let source = (source_bands(origin) + source_bands(origin + vec2(1, 0))
            + source_bands(origin + vec2(0, 1)) + source_bands(origin + vec2(1, 1))) * 0.25;
        let weight = kernel(x);
        light += source * weight;
        total += weight;
    }
    textureStore(horizontal_out, vec2<i32>(id.xy), vec4(light / total, 0.0));
}
@compute @workgroup_size(8, 8)
fn vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(horizontal_in));
    if any(id.xy >= textureDimensions(bloom_out)) { return; }
    var light = vec3<f32>(0.0);
    var total = vec3<f32>(0.0);
    for (var y = -8; y <= 8; y += 1) {
        let position = vec2<i32>(id.xy) + vec2(0, y);
        let weight = kernel(y);
        if all(position >= vec2<i32>(0)) && all(position < dims) {
            light += textureLoad(horizontal_in, position, 0).rgb * weight;
        }
        total += weight;
    }
    textureStore(bloom_out, vec2<i32>(id.xy), vec4(light / total, 0.0));
}
