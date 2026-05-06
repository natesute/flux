// Simple single-pass bloom: extract bright pixels via threshold, blur with
// a separable-ish gaussian kernel, add back to the original. This produces
// the soft-glow look. For more realistic bloom, multiple downsampled passes
// are needed; that's a future upgrade.
//
// Bindings:
//   group(0) binding(0): uniforms
//   group(0) binding(1): tex_in
//   group(0) binding(2): linear sampler

struct Uniforms {
    threshold: f32,
    intensity: f32,
    radius: f32,    // in pixels
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

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex_in, 0));
    let uv = frag.xy / dim;
    let texel = vec2<f32>(1.0) / dim;

    let original = textureSample(tex_in, samp, uv).rgb;

    // Extract + blur in a single loop. 13-tap kernel arranged in a rough
    // disk; cheap and good enough.
    var bloom = vec3<f32>(0.0);
    let r = u.radius;
    var total_w: f32 = 0.0;

    for (var y: i32 = -2; y <= 2; y = y + 1) {
        for (var x: i32 = -2; x <= 2; x = x + 1) {
            let off = vec2<f32>(f32(x), f32(y)) * r * texel;
            let s = textureSample(tex_in, samp, uv + off).rgb;
            // Threshold: only contribute the part above threshold.
            let l = luminance(s);
            let bright = s * max(l - u.threshold, 0.0);
            // Gaussian-ish weight by distance.
            let d = length(vec2<f32>(f32(x), f32(y)));
            let w = exp(-d * d * 0.5);
            bloom = bloom + bright * w;
            total_w = total_w + w;
        }
    }
    bloom = bloom / max(total_w, 0.0001) * u.intensity;

    return vec4<f32>(original + bloom, 1.0);
}
