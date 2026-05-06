# Node reference

Built-in node types and their parameters. Keep this in sync with `src/nodes/`.

## Parameter value types

A parameter in a `.ron` project file is one of:

- **Number**: `0.5`
- **Color**: `[1.0, 0.5, 0.2]` or `[1.0, 0.5, 0.2, 1.0]` (RGB or RGBA, components in 0..1)
- **String**: `"screen"` (used for enums)
- **Audio binding**: `(feature: "bass", scale: 1.0, bias: 0.0)` — value computed at render time from audio features

Available audio features: `rms`, `bass`, `low_mid`, `high_mid`, `treble`.

---

## `solid`

Fills the output with a constant color, optionally scaled by an audio-driven intensity.

**Inputs**: none

**Parameters**:

| Name        | Type   | Default                | Description                              |
|-------------|--------|------------------------|------------------------------------------|
| `color`     | Color  | `[1, 1, 1, 1]`         | Base color.                              |
| `intensity` | Number | `1.0`                  | Multiplier on the color. Bind to audio for pulse effects. |

**Example**:

```ron
"bg": (
    type: "solid",
    params: {
        "color": [0.05, 0.0, 0.1, 1.0],
        "intensity": (feature: "rms", scale: 1.5, bias: 0.5),
    },
),
```

---

## `gradient`

Radial gradient from `inner_color` at the center to `outer_color` at the edge of `radius`. Beyond `radius`, output is `outer_color`.

**Inputs**: none

**Parameters**:

| Name          | Type   | Default          | Description                              |
|---------------|--------|------------------|------------------------------------------|
| `inner_color` | Color  | `[1, 1, 1, 1]`   | Color at the center.                     |
| `outer_color` | Color  | `[0, 0, 0, 1]`   | Color at the edge of `radius`.           |
| `radius`      | Number | `0.5`            | Gradient extent in normalized UV space.  |
| `intensity`   | Number | `1.0`            | Output multiplier.                       |

**Example**:

```ron
"pulse": (
    type: "gradient",
    params: {
        "inner_color": [1.0, 0.6, 0.2, 1.0],
        "outer_color": [0.0, 0.0, 0.0, 1.0],
        "radius": (feature: "bass", scale: 0.6, bias: 0.1),
    },
),
```

---

## `noise`

Animated fractal noise (FBM over Simplex). Generator node, no inputs. The workhorse for organic textures.

**Inputs**: none

**Parameters**:

| Name        | Type   | Default          | Description                              |
|-------------|--------|------------------|------------------------------------------|
| `color_a`   | Color  | `[0, 0, 0, 1]`   | Color where noise = 0.                   |
| `color_b`   | Color  | `[1, 1, 1, 1]`   | Color where noise = 1.                   |
| `scale`     | Number | `3.0`            | Spatial frequency. Higher = finer detail. |
| `speed`     | Number | `0.3`            | Animation speed (units of noise-time per second). |
| `octaves`   | Number | `4.0`            | FBM octaves, clamped 1..8.               |
| `contrast`  | Number | `1.0`            | Power applied to noise; >1 pushes toward black/white. |
| `intensity` | Number | `1.0`            | Output multiplier.                       |

---

## `feedback`

Single most important node for trails. Composites the current input with a transformed copy of the previous frame's output.

**Inputs**: 1 (the source to feed back)

**Parameters**:

| Name        | Type   | Default | Description                                                       |
|-------------|--------|---------|-------------------------------------------------------------------|
| `decay`     | Number | `0.92`  | How much of last frame to keep. Closer to 1 = longer trails.      |
| `zoom`      | Number | `1.01`  | Per-frame zoom. >1 drifts outward, <1 drifts inward.              |
| `rotation`  | Number | `0.0`   | Per-frame rotation in radians.                                    |
| `offset_x`  | Number | `0.0`   | Per-frame translation in normalized UV space.                     |
| `offset_y`  | Number | `0.0`   |                                                                   |
| `mix_in`    | Number | `1.0`   | How much of the current input to add (typically 1.0).             |

Note: on frame 0 there is no history yet, so the node samples black.

---

## `blend`

Composite two inputs.

**Inputs**: 2 (in order: `a` background, `b` foreground)

**Parameters**:

| Name      | Type   | Default  | Description                                            |
|-----------|--------|----------|--------------------------------------------------------|
| `mode`    | String | `"over"` | One of `over`, `add`, `multiply`, `screen`, `mix`.     |
| `factor`  | Number | `1.0`    | Scale on `b` before compositing. Also the mix factor for `mode: "mix"`. |
| `opacity` | Number | `1.0`    | Global multiplier on output.                           |

---

## `bloom`

Soft glow around bright areas. Single-pass implementation; a multi-pass upgrade is planned but the parameter API will not change.

**Inputs**: 1

**Parameters**:

| Name        | Type   | Default | Description                                              |
|-------------|--------|---------|----------------------------------------------------------|
| `threshold` | Number | `0.7`   | Luminance below this contributes nothing to bloom.       |
| `intensity` | Number | `1.0`   | How strong the added glow is.                            |
| `radius`    | Number | `4.0`   | Blur radius in pixels.                                   |

---

## `transform`

Affine 2D transform of an input texture: translate, rotate, and non-uniform scale, all about the center of the frame. Out-of-bounds reads clamp to the edge pixel.

**Inputs**: 1

**Parameters**:

| Name       | Type   | Default | Description                                              |
|------------|--------|---------|----------------------------------------------------------|
| `offset_x` | Number | `0.0`   | Translation in normalized UV (positive = right).         |
| `offset_y` | Number | `0.0`   | Translation in normalized UV (positive = down).          |
| `rotation` | Number | `0.0`   | Rotation in radians (counter-clockwise).                 |
| `scale_x`  | Number | `1.0`   | Horizontal scale. >1 enlarges content, <1 shrinks.       |
| `scale_y`  | Number | `1.0`   | Vertical scale.                                          |

**Example** — punch the image outward on every kick:

```ron
"punch": (
    type: "transform",
    inputs: ["src"],
    params: {
        "scale_x": (feature: "bass", scale: 0.4, bias: 1.0),
        "scale_y": (feature: "bass", scale: 0.4, bias: 1.0),
    },
),
```

---

## `displace`

Warps `src` by sampling at offset UVs derived from `map`. The bridge between generators and final imagery: pipe a `noise` into `displace` and the noise becomes a flow field that pushes the source around.

**Inputs**: 2 (in order: `src` to warp, `map` to drive displacement)

**Parameters**:

| Name     | Type   | Default        | Description                                                            |
|----------|--------|----------------|------------------------------------------------------------------------|
| `amount` | Number | `0.05`         | Displacement strength in normalized UV units. Bind to audio for hits. |
| `mode`   | String | `"derivative"` | `"derivative"`: gradient of map's luminance. `"vector"`: read map's RG channels recentered around 0.5 as the displacement vector. |

**Example** — turn drifting noise into a flow field over a gradient:

```ron
"flow": (
    type: "displace",
    inputs: ["bg", "noise"],
    params: {
        "amount": (feature: "rms", scale: 0.08, bias: 0.02),
        "mode": "derivative",
    },
),
```

---

## `levels`

Per-pixel color adjustments: gain, brightness, contrast (about a 0.5 pivot), and saturation (Rec. 709 luma weights). Identity values are `gain=1, brightness=0, contrast=1, saturation=1`.

**Inputs**: 1

**Parameters**:

| Name         | Type   | Default | Description                                              |
|--------------|--------|---------|----------------------------------------------------------|
| `gain`       | Number | `1.0`   | Multiplicative scale on RGB. Applied first.              |
| `brightness` | Number | `0.0`   | Additive offset on RGB.                                  |
| `contrast`   | Number | `1.0`   | Push values away from 0.5. >1 punchier, <1 flatter.      |
| `saturation` | Number | `1.0`   | 0 = grayscale, 1 = unchanged, >1 = more saturated.       |

---

## `chromatic_aberration`

Radial RGB channel splitting around a pivot. The classic "cheap lens" / glitch look. Push R outward and B inward.

**Inputs**: 1

**Parameters**:

| Name       | Type   | Default | Description                                                |
|------------|--------|---------|------------------------------------------------------------|
| `amount`   | Number | `0.005` | Magnitude of the radial offset, in normalized UV.          |
| `center_x` | Number | `0.5`   | Pivot x in UV.                                             |
| `center_y` | Number | `0.5`   | Pivot y in UV.                                             |

---

## `grain`

Animated hash-based film grain added to the input. Cheap and good for finishing a piece.

**Inputs**: 1

**Parameters**:

| Name     | Type   | Default | Description                                                |
|----------|--------|---------|------------------------------------------------------------|
| `amount` | Number | `0.04`  | Grain magnitude added to RGB. Audio-bind for kick punches. |
| `scale`  | Number | `1.0`   | Speck size in pixels. Higher = chunkier grain.             |

---

## `color_grade`

3D LUT color grading via a 256×16 PNG strip — the format Photoshop's "Export Color LUT" produces. With no `path`, an identity LUT is used and the node is a no-op.

**Inputs**: 1

**Parameters**:

| Name        | Type   | Default | Description                                                |
|-------------|--------|---------|------------------------------------------------------------|
| `path`      | String | —       | LUT PNG (256×16) relative to the project file. Optional.  |
| `intensity` | Number | `1.0`   | Mix between input (0) and graded (1).                      |

---

## `raymarch`

Sphere-traced SDF scene, audio-rippled sphere on a sky gradient, with a single directional light and a rim term. The starter geometry node — clone `shaders/raymarch.wgsl` into `custom_shader` for richer scenes.

**Inputs**: 0

**Parameters** (all numbers; colors as RGBA):

| Name           | Default              | Description                                       |
|----------------|----------------------|---------------------------------------------------|
| `cam_x/y/z`    | `0, 0.5, 3`          | Camera position.                                  |
| `look_x/y/z`   | `0, 0, 0`            | Camera target.                                    |
| `fov`          | `0.9`                | Vertical FOV in radians.                          |
| `radius`       | `1.0`                | Sphere radius. Bind to bass for pulse.            |
| `displacement` | `0.05`               | Surface ripple amplitude. Bind for warble.        |
| `light_x/y/z`  | `0.5, 0.8, 0.3`      | Directional light vector.                         |
| `sky_top`      | `[0.4,0.6,0.9,1]`    | Sky color at +Y.                                  |
| `sky_bottom`   | `[0.05,0.05,0.1,1]`  | Sky color at -Y.                                  |

---

## `instance`

A 4×4×4 grid of unit cubes (64 instances) with depth testing, per-instance HSV tinting, and audio-driven scale. v1 geometry is hardcoded; future versions will accept a mesh path.

**Inputs**: 0

**Parameters**:

| Name           | Default              | Description                                       |
|----------------|----------------------|---------------------------------------------------|
| `cam_x/y/z`    | `4, 3, 6`            | Camera position.                                  |
| `look_x/y/z`   | `0, 0, 0`            | Camera target.                                    |
| `fov`          | `0.8`                | Vertical FOV in radians.                          |
| `base_scale`   | `0.25`               | Per-cube base scale.                              |
| `audio_drive` | `1.0`                 | RMS multiplier on top of base_scale.              |
| `light_x/y/z`  | `0.5, 0.8, 0.4`      | Directional light vector.                         |
| `rim_color`    | `[1,0.7,0.4,1]`      | Rim highlight color.                              |

---

## `custom_shader`

Loads a user-authored WGSL fragment shader from a path relative to the project file. The escape hatch from "whatever nodes flux ships with" — once this exists, anything you can write in WGSL is reachable without touching Rust.

**Inputs**: 0..4

**Parameters**:

| Name   | Type   | Default | Description                                                       |
|--------|--------|---------|-------------------------------------------------------------------|
| `path` | String | —       | Path to a `.wgsl` file, relative to the project file's directory. |

The shader must follow the binding contract documented in [`SHADER_CONVENTIONS.md`](SHADER_CONVENTIONS.md). The number of `inputN` bindings must equal `inputs.len()`. Shader compile errors surface from `flux check` and `flux render`.

**Example**:

```ron
"custom": (
    type: "custom_shader",
    inputs: ["src"],
    params: {
        "path": "shaders/my_effect.wgsl",
    },
),
```

---

*More nodes coming — see [`ROADMAP.md`](ROADMAP.md).*
