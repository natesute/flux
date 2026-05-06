// Radial chromatic aberration. Push R outward and B inward from a pivot;
// G stays put. The classic "cheap lens" look.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): linear sampler

struct Uniforms {
    amount: f32,
    center_x: f32,
    center_y: f32,
    _pad: f32,
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
    let center = vec2<f32>(u.center_x, u.center_y);
    let dir = uv - center;

    let r = textureSample(tex_in, samp, uv + dir * u.amount).r;
    let g = textureSample(tex_in, samp, uv).g;
    let b = textureSample(tex_in, samp, uv - dir * u.amount).b;
    let a = textureSample(tex_in, samp, uv).a;

    return vec4<f32>(r, g, b, a);
}
