// A custom shader: animated diagonal sweep whose hue cycles with time and
// whose brightness pulses with the audio's bass band. Demonstrates the
// `custom_shader` binding contract end-to-end.

struct Uniforms {
    time: f32,
    frame: f32,
    resolution: vec2<f32>,
    rms: f32,
    bass: f32,
    low_mid: f32,
    high_mid: f32,
    treble: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

// Cheap HSV→RGB. Hue in 0..1.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let k = vec3<f32>(5.0, 3.0, 1.0);
    let p = abs(((h * 6.0 + k) % 6.0) - 3.0) - 1.0;
    return v * mix(vec3<f32>(1.0), clamp(p, vec3<f32>(0.0), vec3<f32>(1.0)), s);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / u.resolution;

    // Diagonal coordinate that drifts with time.
    let d = (uv.x + uv.y) * 0.5 + u.time * 0.1;

    // Soft sin band. The 0.5 + 0.5 * sin keeps it in [0, 1].
    let band = 0.5 + 0.5 * sin(d * 6.2831853 * 2.0);

    let hue = (u.time * 0.05) % 1.0;
    let rgb = hsv_to_rgb(hue, 0.9, band);

    let pulse = 0.5 + 1.5 * u.bass;
    return vec4<f32>(rgb * pulse, 1.0);
}
