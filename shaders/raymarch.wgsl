// Raymarched SDF scene: a single sphere at the origin whose surface is
// rippled by audio. Demonstrates the camera-and-march pattern; users who
// want richer scenes should clone this shader and use the `custom_shader`
// node.
//
// Bindings:
//   group(0) binding(0): uniforms

struct Uniforms {
    cam_pos: vec3<f32>,
    fov: f32,
    cam_right: vec3<f32>,
    radius: f32,
    cam_up: vec3<f32>,
    displacement: f32,
    cam_forward: vec3<f32>,
    time: f32,
    light_dir: vec3<f32>,
    aspect: f32,
    sky_top: vec3<f32>,
    _pad0: f32,
    sky_bottom: vec3<f32>,
    _pad1: f32,
    resolution: vec2<f32>,
    _pad2: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

// SDF: sphere with surface ripples. The ripples are pure visual — they
// don't affect the *true* SDF (so we use a small step factor to compensate).
fn map(p: vec3<f32>) -> f32 {
    let ripple = sin(p.x * 6.0 + u.time) * sin(p.y * 6.0 + u.time * 1.1) * sin(p.z * 6.0)
        * u.displacement;
    return length(p) - u.radius - ripple;
}

fn estimate_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.001, 0.0);
    return normalize(vec3<f32>(
        map(p + e.xyy) - map(p - e.xyy),
        map(p + e.yxy) - map(p - e.yxy),
        map(p + e.yyx) - map(p - e.yyx),
    ));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // Build a ray through this pixel.
    let uv = frag.xy / u.resolution;                  // [0, 1]
    let ndc = uv * 2.0 - vec2<f32>(1.0);              // [-1, 1], y-down
    let half_fov = tan(u.fov * 0.5);
    let dir = normalize(u.cam_forward
        + u.cam_right * ndc.x * u.aspect * half_fov
        - u.cam_up * ndc.y * half_fov);

    var t: f32 = 0.0;
    var hit: bool = false;
    for (var i: i32 = 0; i < 96; i = i + 1) {
        let p = u.cam_pos + dir * t;
        let d = map(p);
        if d < 0.001 {
            hit = true;
            break;
        }
        if t > 30.0 {
            break;
        }
        // 0.7 step factor compensates for the ripple distortion making the
        // surface a non-strict SDF.
        t = t + d * 0.7;
    }

    if !hit {
        // Sky: gradient on dir.y.
        let sky_t = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
        return vec4<f32>(mix(u.sky_bottom, u.sky_top, sky_t), 1.0);
    }

    let p = u.cam_pos + dir * t;
    let n = estimate_normal(p);
    let l = normalize(u.light_dir);
    let diffuse = max(dot(n, l), 0.0);

    // Cheap fresnel-like rim.
    let view = -dir;
    let rim = pow(1.0 - max(dot(n, view), 0.0), 3.0);

    let base = vec3<f32>(1.0, 0.6, 0.3);
    let lit = base * (0.15 + 0.85 * diffuse) + vec3<f32>(0.4, 0.6, 1.0) * rim * 0.6;
    return vec4<f32>(lit, 1.0);
}
