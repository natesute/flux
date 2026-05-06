// Instanced 3D mesh rendering. The vertex shader receives per-vertex
// position+normal and per-instance offset+scale+color, and the fragment
// shader does basic Lambertian + ambient + rim shading.
//
// Bindings:
//   group(0) binding(0): uniforms

struct Uniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec3<f32>,
    audio_scale: f32,
    rim_color: vec3<f32>,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) inst_offset: vec3<f32>,
    @location(3) inst_scale: f32,
    @location(4) inst_color: vec3<f32>,
    @location(5) _pad: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_view_dir: vec3<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let scale = in.inst_scale * u.audio_scale;
    let world_pos = in.position * scale + in.inst_offset;

    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_normal = in.normal;
    // Cheap "view dir" approximation — assumes eye near origin. Good
    // enough for a small grid centered there.
    out.world_view_dir = normalize(-world_pos);
    out.color = in.inst_color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let l = normalize(u.light_dir);
    let diffuse = max(dot(n, l), 0.0);
    let rim = pow(1.0 - max(dot(n, in.world_view_dir), 0.0), 3.0);
    let lit = in.color * (0.2 + 0.8 * diffuse) + u.rim_color * rim * 0.5;
    return vec4<f32>(lit, 1.0);
}
