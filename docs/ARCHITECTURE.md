# Architecture

This document describes how `flux` is structured, the design choices, and the trade-offs they imply. Read this before making non-trivial changes.

## Pipeline overview

```
.ron project file
       │
       ▼
   parse  ──►  Project (typed schema)
       │
       ▼
   Engine::new
       │  ├─ create GpuContext (wgpu device + queue)
       │  └─ Graph::from_project
       │       ├─ instantiate every Node
       │       ├─ allocate output texture per node
       │       ├─ validate input references
       │       └─ topologically sort
       ▼
   render loop (per frame):
       │
       │  ├─ AudioTrack::features_at(time)  → FrameAudioFeatures
       │  ├─ for each node in eval_order:
       │  │       node.cook(ctx, inputs, output_tex)
       │  ├─ readback output node's texture → RGBA8
       │  └─ pipe to FFmpeg stdin
       ▼
   FFmpeg muxes audio + video → .mp4
```

## Key design decisions

### Pull-based, but cooked top-down

TouchDesigner uses lazy "cook on demand" evaluation where the renderer pulls from the output. We do effectively the same thing, but materialize the order at graph-build time via topological sort, then evaluate top-down each frame. This is functionally equivalent for most graphs and cheaper per frame.

### One output texture per node

Every node owns a single `Rgba16Float` texture sized to the project's resolution. Downstream nodes sample it as input. This is simple, predictable, and matches how TouchDesigner's TOPs work.

The cost: nodes can't have multiple output ports yet. When we need that, the model will extend to "Node returns a tuple of textures" rather than rearchitecting around it.

### HDR throughout

Internal textures are 16-bit float per channel. This is required for bloom and other effects that legitimately need values above 1.0. Final readback applies tone mapping (Reinhard) and gamma (2.2) to produce 8-bit sRGB for video. If you're getting clipped highlights, the bug is almost certainly in your tone-mapping pass, not in the source signal.

### Audio is offline, not realtime

`AudioTrack` loads the entire WAV into memory and lets you sample features at any timestamp. This means rendering is deterministic — you can render the same project twice and get byte-identical video. Real-time audio (cpal) is wired in but not yet used; a future `live` subcommand will route mic/line input through the same FFT pipeline for VJ-style use.

### Project files are RON

Compared to JSON: comments, trailing commas, less line noise. Compared to TOML: better support for nested heterogeneous structures. Compared to YAML: less ambiguity. RON is also Rust-native, so the schema *is* the deserialization target — no marshalling layer.

### One file per node type

This is non-negotiable. Every node type lives in `src/nodes/<name>.rs` and is at most ~150 lines. If a node is bigger, it's almost certainly several nodes pretending to be one.

## Things deliberately *not* done

### No GUI

`flux` is CLI-first. A node-graph editor is a serious application in its own right and would dominate development time. If we eventually want a GUI, it'll be a separate companion crate that reads/writes the same `.ron` files.

### No realtime preview window

For now. Adding one is straightforward (winit + a swapchain) but every preview path is one more thing to maintain. The intended workflow is: render a 5-second preview with `--duration 5`, look at the file, iterate.

### No node hot-reloading

Reloading the project file mid-render is not supported. Re-run the binary.

### No Python scripting (yet)

A future PyO3 binding would let you drive the engine from Python and write nodes in Python. Not in v0.1.

## Performance targets

- 1080p @ 60fps offline render of a typical 8-node graph: faster than realtime on a discrete GPU
- 4K @ 30fps offline: at least real-time on a midrange discrete GPU
- Realtime preview (when added): 60fps at 1080p with 16-node graph

If a change drops below these on a graph that previously hit them, it needs a flag to disable.

## Areas where this *will* hit limits

You will outgrow this if you:

- Need >100 nodes in a graph (the per-frame texture readback approach assumes a small graph; we'd need explicit node compilation/fusion)
- Need realtime sync to a live MIDI/OSC stream
- Need to ship visuals to a NDI/Spout output
- Need physics or particle systems with millions of particles
- Need 3D scene management with lights and PBR materials

When you hit these, the answer is probably "add a backend" or "use TouchDesigner for this piece," not "rewrite the engine."
