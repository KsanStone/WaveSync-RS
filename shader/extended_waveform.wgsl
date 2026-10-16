struct Uniforms {
    sample_color: vec4<f32>,
    rms_color: vec4<f32>,
    viewport: vec2<f32>,
    count: u32,
    head: u32,
    scroll: f32,
    range: f32,
    grid_origin: f32,
    guard_columns: u32,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// min, max, mean-square, unused. History lives in this GPU ring.
@group(0) @binding(1) var<storage, read> columns: array<vec4<f32>>;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs_main(@location(0) position: vec2<f32>) -> Output {
    var out: Output;
    out.position=vec4(position,0.0,1.0);
    out.uv=position*0.5+vec2(0.5);
    return out;
}
fn column(index: i32) -> vec4<f32> {
    let logical=u32(clamp(index,0,i32(uniforms.count)-1));
    return columns[(uniforms.head+logical)%uniforms.count];
}
fn interval_coverage(bounds: vec2<f32>, y: f32) -> f32 {
    let outside=max(bounds.x-y,y-bounds.y)*uniforms.viewport.y*0.5/uniforms.range;
    return clamp(0.5-outside,0.0,1.0);
}
// These buckets are anchored in audio-history coordinates, not screen pixels.
// Their membership stays fixed while the image moves across the viewport.
fn envelope(center: f32, footprint: f32) -> vec4<f32> {
    if footprint <= 1.0 { return column(i32(round(center))); }
    let left=center-footprint*0.5;
    let right=center+footprint*0.5;
    var low=3.402823e38;
    var high=-3.402823e38;
    var squares=0.0;
    var weight_sum=0.0;
    for (var i=i32(floor(left+0.5)); i<=i32(ceil(right+0.5)); i=i+1) {
        let weight=clamp(min(right,f32(i)+0.5)-max(left,f32(i)-0.5),0.0,1.0);
        if weight>0.0 {
            let value=column(i);
            low=min(low,value.x);
            high=max(high,value.y);
            squares+=value.z*weight;
            weight_sum+=weight;
        }
    }
    return vec4(low,high,squares/max(weight_sum,0.00001),0.0);
}
fn coverage(value: vec4<f32>, y: f32) -> vec2<f32> {
    let rms=sqrt(max(value.z,0.0));
    let rms_bounds=clamp(vec2(-rms,rms),vec2(value.x),vec2(value.y));
    return vec2(interval_coverage(value.xy,y),interval_coverage(rms_bounds,y));
}
@fragment
fn fs_main(input: Output) -> @location(0) vec4<f32> {
    if uniforms.count <= uniforms.guard_columns { discard; }
    let span=f32(uniforms.count-uniforms.guard_columns-1u);
    let center=input.uv.x*span+f32(uniforms.guard_columns)+uniforms.scroll;
    let footprint=span/max(uniforms.viewport.x,1.0);
    let step=max(footprint,1.0);
    let pixel_position=(center+uniforms.grid_origin)/step;
    let base=floor(pixel_position)*step-uniforms.grid_origin;
    let y=(input.uv.y*2.0-1.0)*uniforms.range;
    // Interpolate final coverage, rather than moving min/max boundaries through
    // a pixel. Narrow gaps therefore fade across adjacent pixels, not blink.
    let values=mix(coverage(envelope(base,footprint),y),
                   coverage(envelope(base+step,footprint),y),fract(pixel_position));
    let sample_alpha=uniforms.sample_color.a*values.x;
    let rms_alpha=uniforms.rms_color.a*values.y;
    let alpha=rms_alpha+sample_alpha*(1.0-rms_alpha);
    if alpha <= 0.0001 { discard; }
    let premultiplied=uniforms.rms_color.rgb*rms_alpha+uniforms.sample_color.rgb*sample_alpha*(1.0-rms_alpha);
    return vec4(premultiplied/alpha,alpha);
}
