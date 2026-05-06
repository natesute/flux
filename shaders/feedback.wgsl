// Feedback: composite the current input with a transformed copy of the
// previous frame's feedback output. The "trails" effect.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_current  (this frame's input)
//   group(0) binding(2): tex_history  (last frame's output)
//   group(0) binding(3): linear sampler

struct Uniforms {
    decay: f32,         // 0..1, how much of last frame to keep
    zoom: f32,          // 1.0 = no zoom, >1 zooms in (drift outward), <1 drifts inward
    rotation: f32,      // radians per frame
    offset_x: f32,
    offset_y: f32,
    mix_in: f32,        // how much of current input to add (typically 1.0)
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_current: texture_2d<f32>;
@group(0) @binding(2) var tex_history: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_current, 0));
    let uv = frag.xy / dim;

    // Center the transform on the middle of the frame.
    var p = uv - vec2<f32>(0.5);

    // Apply zoom (sample from a larger area inverts to "outward drift").
    p = p / max(u.zoom, 0.0001);

    // Rotate.
    let c = cos(u.rotation);
    let s = sin(u.rotation);
    p = vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);

    // Translate.
    p = p + vec2<f32>(0.5) + vec2<f32>(u.offset_x, u.offset_y);

    let history = textureSample(tex_history, samp, p).rgb * u.decay;
    let current = textureSample(tex_current, samp, uv).rgb * u.mix_in;

    return vec4<f32>(history + current, 1.0);
}
