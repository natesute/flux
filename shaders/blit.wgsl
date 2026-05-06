// Final present pass for the preview window. Samples the engine's HDR
// output texture, applies the project's tone map, and writes linear
// values to the swapchain — the swapchain is sRGB so it gamma-encodes for
// display automatically. (Offline rendering does the same math on the
// CPU in src/engine/graph.rs.)
//
// Bindings:
//   group(0) binding(0): tone-map mode (uniforms.mode)
//   group(0) binding(1): the engine output (Rgba16Float)
//   group(0) binding(2): linear sampler

struct Uniforms {
    // 0=ACES, 1=Reinhard, 2=None.
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn reinhard(x: vec3<f32>) -> vec3<f32> {
    return clamp(x / (vec3<f32>(1.0) + x), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex, 0));
    let uv = frag.xy / dim;
    let raw = textureSample(tex, samp, uv).rgb;

    var mapped: vec3<f32>;
    if u.mode == 0u {
        mapped = aces(raw);
    } else if u.mode == 1u {
        mapped = reinhard(raw);
    } else {
        mapped = clamp(raw, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // sRGB swapchain handles the gamma transform.
    return vec4<f32>(mapped, 1.0);
}
