# flux

A code-first, agent-friendly audiovisual rendering engine. Inspired by TouchDesigner's dataflow-graph model, but built from the ground up for hobbyists who want to define audio-reactive visuals in plain text and render them from the command line.

## What this is

`flux` lets you describe an audiovisual piece as a graph of nodes in a `.ron` project file, point it at an audio track, and render a video. No GUI. No proprietary file format. No Mac-vs-Windows licensing maze. Just code, shaders, and FFmpeg.

```bash
flux render examples/first_loop.ron --audio song.wav --out loop.mp4
```

## What this is not

This is **not** a TouchDesigner clone. TouchDesigner is the product of 20+ years of work by a team of graphics engineers; reproducing it is not realistic and not the goal. `flux` aims to cover the ~10% of TouchDesigner's surface area that matters for making audio-reactive loops, and do that part well, with an architecture friendly to AI-assisted development.

If you need: realtime VJ performance with MIDI controllers, projection mapping, large-scale installation features, NDI/Spout, or hundreds of prebuilt operators — use TouchDesigner. `flux` is for the offline-render-an-audio-reactive-visual-and-post-it-to-Instagram use case.

## Why this design

- **Plain text projects** — diffable, version-controllable, AI-editable. A future Claude session can read your project file and meaningfully modify it.
- **CLI-first** — every feature is reachable from the command line. Scripting, batch rendering, and automation are first-class.
- **Custom shaders are just files** — drop a `.wgsl` shader into your project directory and reference it by name.
- **Deterministic** — same project file + same audio = byte-identical video output. Render farms, regression tests, and reproducibility all work.
- **Modular nodes** — each node type is a small, isolated Rust struct. Adding new nodes is a 50-line task.

## Stack

- **Rust** for the core engine (predictable performance, no GC, modern tooling)
- **wgpu** for cross-platform GPU rendering (Vulkan/Metal/DX12/WebGPU)
- **WGSL** for shaders (with optional GLSL via `naga`)
- **cpal** + **rustfft** for audio analysis
- **RON** for project files (like JSON, but readable, with comments)
- **FFmpeg** (external dependency) for final video encoding

## Status

Early scaffolding. See [`docs/ROADMAP.md`](docs/ROADMAP.md) for what's working and what isn't.

## Quick start

```bash
# Prerequisites: Rust toolchain (https://rustup.rs), FFmpeg on PATH
git clone <your-fork>
cd flux
cargo run --release -- render examples/first_loop.ron --audio examples/test.wav --out out.mp4
```

## Working with AI agents

This repo is structured to be ergonomic for Claude Code and similar agents. See [`CLAUDE.md`](CLAUDE.md) for conventions agents should follow when extending the codebase.

## License

MIT. See [`LICENSE`](LICENSE).
