// Animated 2D noise. Uses 3D Simplex noise sampled at (uv * scale, time * speed)
// for free animation, then layers octaves for FBM (fractal brownian motion).
//
// Bindings:
//   group(0) binding(0): uniforms

struct Uniforms {
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    resolution: vec2<f32>,
    scale: f32,
    speed: f32,
    octaves: f32,
    contrast: f32,
    intensity: f32,
    time: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// ---------- Simplex 3D noise (Ashima Arts / Stefan Gustavson, public domain) ----------
fn permute4(x: vec4<f32>) -> vec4<f32> {
    return (((x * 34.0) + 1.0) * x) % vec4<f32>(289.0);
}
fn taylorInvSqrt4(r: vec4<f32>) -> vec4<f32> {
    return 1.79284291400159 - 0.85373472095314 * r;
}
fn snoise3(v: vec3<f32>) -> f32 {
    let C = vec2<f32>(1.0 / 6.0, 1.0 / 3.0);
    let D = vec4<f32>(0.0, 0.5, 1.0, 2.0);

    var i = floor(v + dot(v, C.yyy));
    let x0 = v - i + dot(i, C.xxx);

    let g = step(x0.yzx, x0.xyz);
    let l = 1.0 - g;
    let i1 = min(g.xyz, l.zxy);
    let i2 = max(g.xyz, l.zxy);

    let x1 = x0 - i1 + C.xxx;
    let x2 = x0 - i2 + C.yyy;
    let x3 = x0 - D.yyy;

    i = i % vec3<f32>(289.0);
    let p = permute4(permute4(permute4(
              vec4<f32>(i.z) + vec4<f32>(0.0, i1.z, i2.z, 1.0))
            + vec4<f32>(i.y) + vec4<f32>(0.0, i1.y, i2.y, 1.0))
            + vec4<f32>(i.x) + vec4<f32>(0.0, i1.x, i2.x, 1.0));

    let n_ = 0.142857142857;
    let ns = n_ * D.wyz - D.xzx;

    let j = p - 49.0 * floor(p * ns.z * ns.z);
    let x_ = floor(j * ns.z);
    let y_ = floor(j - 7.0 * x_);

    let x = x_ * ns.x + ns.yyyy;
    let y = y_ * ns.x + ns.yyyy;
    let h = 1.0 - abs(x) - abs(y);

    let b0 = vec4<f32>(x.xy, y.xy);
    let b1 = vec4<f32>(x.zw, y.zw);
    let s0 = floor(b0) * 2.0 + 1.0;
    let s1 = floor(b1) * 2.0 + 1.0;
    let sh = -step(h, vec4<f32>(0.0));
    let a0 = b0.xzyw + s0.xzyw * sh.xxyy;
    let a1 = b1.xzyw + s1.xzyw * sh.zzww;

    var p0 = vec3<f32>(a0.xy, h.x);
    var p1 = vec3<f32>(a0.zw, h.y);
    var p2 = vec3<f32>(a1.xy, h.z);
    var p3 = vec3<f32>(a1.zw, h.w);

    let norm = taylorInvSqrt4(vec4<f32>(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 = p0 * norm.x;
    p1 = p1 * norm.y;
    p2 = p2 * norm.z;
    p3 = p3 * norm.w;

    var m = max(0.6 - vec4<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4<f32>(0.0));
    m = m * m;
    return 42.0 * dot(m * m, vec4<f32>(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

fn fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    for (var i: i32 = 0; i < octaves; i = i + 1) {
        sum = sum + snoise3(p * freq) * amp;
        freq = freq * 2.0;
        amp = amp * 0.5;
    }
    return sum;
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / u.resolution.y;
    var uv = frag.xy / u.resolution;
    uv.x = uv.x * aspect;

    let p = vec3<f32>(uv * u.scale, u.time * u.speed);
    let octaves = i32(clamp(u.octaves, 1.0, 8.0));
    var n = fbm(p, octaves);

    // remap from [-1, 1] to [0, 1]
    n = n * 0.5 + 0.5;
    // contrast: push values toward 0 or 1
    n = pow(n, u.contrast);

    let col = mix(u.color_a.rgb, u.color_b.rgb, n) * u.intensity;
    return vec4<f32>(col, 1.0);
}
