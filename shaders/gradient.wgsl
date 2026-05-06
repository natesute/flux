// Radial gradient with audio-driven center brightness.
// Bindings:
//   group(0) binding(0): uniforms
struct Uniforms {
    inner_color: vec4<f32>,
    outer_color: vec4<f32>,
    center: vec2<f32>,
    radius: f32,
    intensity: f32,
    resolution: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Fullscreen triangle.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag_coord.xy / u.resolution;
    let d = distance(uv, u.center) / max(u.radius, 0.0001);
    let t = clamp(d, 0.0, 1.0);
    let col = mix(u.inner_color.rgb, u.outer_color.rgb, t) * u.intensity;
    return vec4<f32>(col, 1.0);
}
