// Warp `tex_src` by sampling at offset UVs derived from `tex_map`.
//
// Mode 0 (`derivative`): take the gradient of the map's luminance via
// central differences and use that as the displacement vector. This is the
// most generally useful mode — any greyscale input becomes a flow field.
//
// Mode 1 (`vector`): read the map's R and G channels directly, recentered
// around 0.5, as a 2D displacement. Use when you've authored a normal-map-
// style vector field (R, G ∈ [0, 1]).
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_src
//   group(0) binding(2): tex_map
//   group(0) binding(3): linear sampler

struct Uniforms {
    amount: f32,
    mode: u32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_src: texture_2d<f32>;
@group(0) @binding(2) var tex_map: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_src, 0));
    let uv = frag.xy / dim;

    var disp: vec2<f32>;
    if u.mode == 1u {
        // vector mode
        let v = textureSample(tex_map, samp, uv).rg - vec2<f32>(0.5);
        disp = v;
    } else {
        // derivative mode (default)
        let texel = vec2<f32>(1.0) / dim;
        let l_r = luma(textureSample(tex_map, samp, uv + vec2<f32>(texel.x, 0.0)).rgb);
        let l_l = luma(textureSample(tex_map, samp, uv - vec2<f32>(texel.x, 0.0)).rgb);
        let l_u = luma(textureSample(tex_map, samp, uv + vec2<f32>(0.0, texel.y)).rgb);
        let l_d = luma(textureSample(tex_map, samp, uv - vec2<f32>(0.0, texel.y)).rgb);
        disp = vec2<f32>(l_r - l_l, l_u - l_d) * 0.5;
    }

    let displaced_uv = uv + disp * u.amount;
    return textureSample(tex_src, samp, displaced_uv);
}
