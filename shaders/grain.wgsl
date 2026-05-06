// Animated film grain. Hash-based per-pixel noise added to the input,
// scaled by `amount`. `scale` controls how big the grain "specks" are
// (smaller = finer grain). Audio-bindable amount lets you punch grain on
// kick hits.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): linear sampler

struct Uniforms {
    amount: f32,
    scale: f32,
    time: f32,
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

// Cheap deterministic hash; output in [-1, 1].
fn hash21(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(123.34, 456.21));
    let r = q + dot(q, q + 78.233);
    return fract(r.x * r.y) * 2.0 - 1.0;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_in, 0));
    let uv = frag.xy / dim;
    let c = textureSample(tex_in, samp, uv);

    // Quantize the sample point so grain "specks" are bigger than 1px.
    let grain_uv = floor(frag.xy / max(u.scale, 1.0)) + vec2<f32>(u.time * 60.0);
    let n = hash21(grain_uv);

    return vec4<f32>(c.rgb + vec3<f32>(n * u.amount), c.a);
}
