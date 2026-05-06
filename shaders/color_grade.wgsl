// 3D LUT color grading. The LUT is a 16×16×16 cube laid out as a 256×16
// strip texture: 16 horizontal cells of 16×16 each, where the cell index
// is blue (0..15) and the position within each cell is (red, green).
// This is the format Photoshop and most DCC tools export.
//
// `intensity` blends between the input (0.0) and the graded result (1.0),
// so audio bindings can fade the look in and out.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): tex_lut  (256x16)
//   group(0) binding(3): linear sampler (clamp)

struct Uniforms {
    intensity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_in: texture_2d<f32>;
@group(0) @binding(2) var tex_lut: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn lut_sample(c: vec3<f32>) -> vec3<f32> {
    let lut_size = 16.0;
    let lut_w = 256.0;
    let lut_h = 16.0;

    let cc = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));

    // Blue selects which 16x16 cell to read.
    let b_idx = cc.b * (lut_size - 1.0);
    let b_low = floor(b_idx);
    let b_high = min(b_low + 1.0, lut_size - 1.0);
    let b_frac = b_idx - b_low;

    // Within-cell coords (texel-centered).
    let r_x = cc.r * (lut_size - 1.0) + 0.5;
    let g_y = cc.g * (lut_size - 1.0) + 0.5;

    let uv_low = vec2<f32>((b_low * lut_size + r_x) / lut_w, g_y / lut_h);
    let uv_high = vec2<f32>((b_high * lut_size + r_x) / lut_w, g_y / lut_h);

    let c_low = textureSample(tex_lut, samp, uv_low).rgb;
    let c_high = textureSample(tex_lut, samp, uv_high).rgb;

    return mix(c_low, c_high, b_frac);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_in, 0));
    let uv = frag.xy / dim;
    let src = textureSample(tex_in, samp, uv);
    let graded = lut_sample(src.rgb);
    return vec4<f32>(mix(src.rgb, graded, u.intensity), src.a);
}
