# Shader conventions

WGSL shaders for built-in nodes follow a consistent structure so they can be edited with minimal context.

## File location

`shaders/<node_name>.wgsl`. The Rust node loads it via `include_str!("../../shaders/<name>.wgsl")` so it's compiled into the binary.

## Bind group layout

For shader-driven nodes:

- **`@group(0) @binding(0)`** — uniforms (per-frame parameters)
- **`@group(0) @binding(1..)`** — input textures (when the node has inputs)
- **`@group(0) @binding(N)`** — sampler (typically a linear clamp sampler)

## Vertex shader pattern

Every screen-filling node uses the same fullscreen-triangle trick:

```wgsl
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}
```

Then `pass.draw(0..3, 0..1)` from the Rust side. Faster than a quad (one less vertex, no overdraw across the diagonal).

## Uniform struct layout

WGSL has strict alignment rules. Pad to 16-byte boundaries. The Rust `#[repr(C)]` struct must match the WGSL `struct` field-for-field.

```wgsl
struct Uniforms {
    color: vec4<f32>,    // 16 bytes
    resolution: vec2<f32>, // 8 bytes
    time: f32,             // 4 bytes
    intensity: f32,        // 4 bytes  (total: 32 bytes, naturally 16-aligned)
};
```

When in doubt, add explicit `_pad` fields. Misaligned uniforms produce silent garbage, not errors.

## Output color space

Every shader writes linear-light values into an `Rgba16Float` target. Tone mapping and gamma happen in the readback path, *not* in the shader. Don't apply `pow(c, 2.2)` or similar in your shaders — you'll double-correct.

## Custom shaders (`custom_shader` node)

User-authored shaders loaded via `custom_shader` use a fixed binding contract. The number of `inputN` texture bindings must equal the node's `inputs.len()`.

```wgsl
struct Uniforms {
    time: f32,            // seconds since start of render
    frame: f32,            // frame index, as f32
    resolution: vec2<f32>, // output texture size in pixels
    rms: f32,              // audio amplitude, ~0..1
    bass: f32,             // ~20-250 Hz band, ~0..1
    low_mid: f32,          // ~250-1000 Hz band
    high_mid: f32,         // ~1000-4000 Hz band
    treble: f32,           // ~4000-16000 Hz band
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var input0: texture_2d<f32>;
// up to input3 at @binding(5)

@vertex   fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { /* fullscreen triangle */ }
@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> { /* your code */ }
```

Custom shaders receive the same color-space rules: write linear-light values; tone mapping and gamma happen on readback. Shader compile errors surface as a real error from `flux check` and `flux render` — they aren't silent.
