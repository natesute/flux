// Affine 2D transform of an input texture.
//
// We sample the input at the *inverse* of the requested transform, so
// passing scale=2.0 makes the visible content twice as large (the sample
// area shrinks). Pivot is the center of the frame: rotations rotate
// around the middle, scales grow/shrink from the middle.
//
// Out-of-bounds reads use clamp-to-edge (smearing the edge pixel). A
// future param could swap that for transparent black or wrap.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): linear sampler

struct Uniforms {
    offset_x: f32,
    offset_y: f32,
    rotation: f32,    // radians, counter-clockwise
    scale_x: f32,
    scale_y: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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

    // Center the transform on the middle of the frame.
    var p = uv - vec2<f32>(0.5);

    // Inverse translate (so positive offset shifts content in the
    // intuitive direction).
    p = p - vec2<f32>(u.offset_x, u.offset_y);

    // Inverse rotate.
    let c = cos(-u.rotation);
    let s = sin(-u.rotation);
    p = vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);

    // Inverse scale, with a tiny floor so a zero scale doesn't NaN.
    p = p / vec2<f32>(max(u.scale_x, 0.0001), max(u.scale_y, 0.0001));

    p = p + vec2<f32>(0.5);

    return textureSample(tex_in, samp, p);
}
