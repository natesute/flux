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

*More nodes coming — see [`ROADMAP.md`](ROADMAP.md).*
