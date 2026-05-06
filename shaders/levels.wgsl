// Per-pixel color adjustments. The simplest possible post node and a good
// template for any new "single-input no-neighborhood" effect.
//
// Order of operations (matches what most DCC tools do):
//   1. gain        (multiply)
//   2. brightness  (additive offset)
//   3. contrast    (push values away from 0.5 pivot)
//   4. saturation  (interpolate between luminance grey and original)
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): linear sampler

struct Uniforms {
    gain: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_in: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_in, 0));
    let uv = frag.xy / dim;
    var c = textureSample(tex_in, samp, uv);

    // 1. gain
    c = vec4<f32>(c.rgb * u.gain, c.a);

    // 2. brightness
    c = vec4<f32>(c.rgb + vec3<f32>(u.brightness), c.a);

    // 3. contrast around 0.5
    c = vec4<f32>((c.rgb - vec3<f32>(0.5)) * u.contrast + vec3<f32>(0.5), c.a);

    // 4. saturation. Rec. 709 luma weights.
    let luma = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = vec4<f32>(mix(vec3<f32>(luma), c.rgb, u.saturation), c.a);

    return c;
}
