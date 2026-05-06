# Roadmap

A truthful list of what is and isn't implemented. If you're an AI agent or a contributor: do not claim a feature works unless it appears under "Working" below.

## v0.1 — Foundations (current)

### Working
- [x] Project file loading and validation (`.ron` schema)
- [x] GPU context (wgpu, cross-platform)
- [x] Audio loading (WAV) and per-frame FFT analysis
- [x] Pull-based dataflow graph with topological cooking
- [x] Audio-to-parameter binding (`feature: "bass", scale, bias`)
- [x] FFmpeg-based video output (H.264 + AAC, mp4)
- [x] Configurable tone mapping (`aces` default, `reinhard`, `none`) + 2.2 gamma on readback
- [x] CLI: `render`, `check`, `nodes` subcommands; `--tone-map` override on render
- [x] Node types: `solid`, `gradient`

### Stubbed / not yet built
- [ ] Most useful node types (see below)
- [ ] Live audio input (cpal not yet a dependency; deferred to v0.4)
- [ ] Realtime preview window
- [ ] Custom-shader node (load arbitrary `.wgsl` from project dir)
- [ ] Snapshot tests for nodes

## v0.2 — A useful node library

Goal: enough nodes that you can produce something that resembles a Klsr-style audio-reactive loop.

### Working
- [x] `noise` — animated FBM over Simplex noise, audio-bindable scale/speed/contrast
- [x] `feedback` — frame-N-1 history texture with zoom/rotation/offset/decay; the trails effect
- [x] `blend` — composite two inputs with `over`, `add`, `multiply`, `screen`, `mix` modes
- [x] `bloom` — single-pass thresholded glow (multi-pass upgrade tracked separately)
- [x] `transform` — affine translate/rotate/scale about the frame center
- [x] `levels` — gain, brightness, contrast, saturation (per-pixel)
- [x] `shader_pass` helper module — shared boilerplate so adding a new shader node is ~80 lines instead of ~150
- [x] Snapshot test scaffolding (`TestHarness` + `ImageStats`) and one test per node

### Still to do for v0.2 polish
- [x] `displace` — luminance-derivative or RG-vector warp of one input by another
- [x] `chromatic_aberration` — radial RGB channel splitting around a pivot
- [x] `grain` — animated hash-noise overlay
- [x] `color_grade` — 3D LUT (256×16 PNG strip) color grading; identity LUT when no path
- [x] `examples/atmospheric.ron` rewritten to exercise displace, transform, levels, CA, grain

## v0.3 — 3D and shader-driven generation

- [x] `raymarch` — starter SDF scene (audio-rippled sphere with sky gradient + Lambertian + rim). Users wanting richer scenes should clone `shaders/raymarch.wgsl` and load through `custom_shader`.
- [x] `instance` — instanced cube grid with depth testing, audio-driven scale, per-instance HSV tinting. The first geometry node; vertex/index/instance buffers + a depth attachment now exist in the engine.
- [x] `custom_shader` — load user `.wgsl` shaders from project directory; binding contract in `docs/SHADER_CONVENTIONS.md`.
- HDR environment maps for reflective surfaces (deferred — neither raymarch nor instance is currently reflection-driven).

## v0.4 — Workflow polish

- [x] `flux preview` subcommand opens a winit window and live-renders the project. v1 reads audio for features but does **not** play it back through speakers — wall-clock time loops the audio file.
- Audio playback in preview (cpal — pulls in alsa-sys on Linux)
- Hot-reload of shader files
- Better error messages for malformed projects

## v0.5 — Maybe

- Python bindings via PyO3 (define nodes in Python)
- Headless render farm mode (split a long render across multiple processes/machines)
- Spout/NDI output (Windows/Linux respectively) for live integration

## Probably never

These are explicit non-goals:

- A node-graph GUI editor (separate project if it happens)
- Plugin marketplace or any kind of monetization
- Windows-only or Mac-only features (must work on at least Linux + macOS + Windows)
- Cloning every TouchDesigner operator. Pick the 30 that matter, do them well.
