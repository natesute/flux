// Composite two input textures.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_a
//   group(0) binding(2): tex_b
//   group(0) binding(3): linear sampler

struct Uniforms {
    // Mode: 0 over, 1 add, 2 multiply, 3 screen, 4 mix
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    factor: f32,    // for `mix` and as a scale on b
    opacity: f32,   // global multiplier on output
    _pad3: f32,
    _pad4: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_a: texture_2d<f32>;
@group(0) @binding(2) var tex_b: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_a, 0));
    let uv = frag.xy / dim;
    let a = textureSample(tex_a, samp, uv);
    let b = textureSample(tex_b, samp, uv) * u.factor;

    var rgb: vec3<f32>;
    switch u.mode {
        case 0u: { // over (b on top of a, premultiplied alpha-style)
            let alpha = b.a;
            rgb = a.rgb * (1.0 - alpha) + b.rgb * alpha;
        }
        case 1u: { rgb = a.rgb + b.rgb; }                     // add
        case 2u: { rgb = a.rgb * b.rgb; }                     // multiply
        case 3u: { rgb = 1.0 - (1.0 - a.rgb) * (1.0 - b.rgb); } // screen
        case 4u: { rgb = mix(a.rgb, b.rgb, u.factor); }       // mix
        default: { rgb = a.rgb; }
    }
    return vec4<f32>(rgb * u.opacity, 1.0);
}
