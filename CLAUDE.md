# Working on flux with AI agents

This document is the contract between the human owner of this repo and any AI coding agent (Claude Code, etc.) working on it. Read this before making changes.

## Mental model in one paragraph

`flux` is a pull-based dataflow graph that runs on the GPU once per frame. A `Project` (loaded from `.ron`) declares a graph of `Node`s and a sink (`output`). On each frame, the renderer asks the sink to `cook()`, which recursively cooks its inputs. Each node owns a GPU texture (or buffer of channel data) as its output. Nodes are stateless across runs except where they explicitly maintain history (e.g. `Feedback`). Audio is analyzed once per frame into a `FrameAudioFeatures` struct, which any node can sample.

If you understand that paragraph, you can extend the engine.

## Project structure

```
src/
  main.rs              CLI entry point. Argument parsing, top-level commands.
  lib.rs               Crate root. Re-exports public types.
  engine/              Core graph evaluation, GPU context, frame loop.
    mod.rs
    context.rs         GPU device/queue ownership.
    graph.rs           Graph type, topological cooking.
    frame.rs           Per-frame state.
  audio/               Audio loading and analysis.
    mod.rs
    analysis.rs        FFT, beat detection, feature extraction.
  nodes/               One file per node type. Each implements `Node` trait.
    mod.rs             Registry and dispatch.
    audio_in.rs
    gradient.rs
    noise.rs
    feedback.rs
    blend.rs
    bloom.rs
    ...
  project/             Project file loading and validation.
    mod.rs
    schema.rs          RON-deserializable types.
  output/              Encoding to video.
    mod.rs
    ffmpeg.rs          Pipe frames to FFmpeg.

shaders/               WGSL shaders. Built into the binary via include_str!.
examples/              Example project files and a test audio clip.
docs/                  ROADMAP, ARCHITECTURE, NODE_REFERENCE.
```

## Conventions

### Adding a new node

Every node lives in its own file under `src/nodes/`. The minimum is:

1. Define a struct holding the node's parameters and any GPU resources it owns.
2. Implement `Node` (`cook`, `inputs`, `output_format`).
3. Register it in `src/nodes/mod.rs`'s `node_from_spec` dispatch.
4. Add a section to `docs/NODE_REFERENCE.md`.
5. If it has a shader, put it in `shaders/<node_name>.wgsl`.

Aim for ~100 lines per node file. If a node is getting bigger, it's probably actually multiple nodes.

### Shaders

WGSL only for built-in nodes. A future custom-shader node will accept user shaders by path.

Shader inputs use a fixed binding convention defined in `docs/SHADER_CONVENTIONS.md`. Don't deviate without updating that doc and every shader.

### Project files (.ron)

The schema is defined in `src/project/schema.rs`. Keep it stable. Breaking changes to the schema require a version bump and a migration note in `docs/MIGRATIONS.md`.

### Errors

Use `anyhow::Result<T>` at the binary boundary, `thiserror`-derived enums for library APIs. Never `unwrap()` outside of test code or `main.rs` startup.

### Performance

Rendering must hit the target framerate (default 60fps for offline render at 1080p, can be relaxed via `--fps`). Profile before optimizing. Don't add a feature that drops below target without a config flag to disable it.

### Tests

Each node should have a unit test that cooks it with a deterministic input and snapshots the output texture's hash. See `src/nodes/gradient.rs` for the pattern.

## What agents should NOT do without asking

- **Add new top-level dependencies to Cargo.toml.** Prefer using what's there. If a new dep is genuinely needed, propose it first.
- **Change the project file schema.** This breaks all existing projects.
- **Refactor multiple modules at once.** One concern per PR/change.
- **Add a GUI.** This is a CLI-first tool by design. A GUI is a separate, optional companion project, not part of the core.
- **Replace WGSL with GLSL** or wgpu with another GPU API. The stack is chosen.

## What agents should freely do

- Add new node types following the conventions above.
- Add new shaders.
- Improve docs.
- Write tests.
- Optimize hot paths with profiling data to back it up.
- Add new CLI subcommands that don't change existing behavior.

## Check-in checklist before declaring work done

- [ ] `cargo build --release` succeeds with no warnings
- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] If a new node was added, it appears in `docs/NODE_REFERENCE.md`
- [ ] If the project schema changed, it's documented in `docs/MIGRATIONS.md`
- [ ] At least one example project file exercises the new feature

## Ground truth

The human owner makes design decisions. When unsure, ask. Don't infer "they probably want X" from ambiguous instructions in a way that locks in a design choice.
