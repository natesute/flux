# Working on flux with AI agents

This document is the contract between the human owner of this repo and any AI coding agent (Claude Code, Codex, etc.) working on it. Read this before making changes.

## Mental model in one paragraph

`flux` is a pull-based dataflow graph that runs on the GPU once per frame. A `Project` (loaded from `.ron`) declares a graph of `Node`s and a sink (`output`). On each frame, the renderer asks the sink to `cook()`, which recursively cooks its inputs. Each node owns a GPU texture (or buffer of channel data) as its output. Nodes are stateless across runs except where they explicitly maintain history (e.g. `Feedback`). Audio is analyzed once per frame into a `FrameAudioFeatures` struct, which any node can sample.

If you understand that paragraph, you can extend the engine.

## Project structure

- `src/main.rs` is the CLI entry point; `src/lib.rs` is the crate root.
- `src/engine/` holds graph evaluation, the GPU context and the frame loop. `src/audio/` loads and analyses audio. `src/project/` loads and validates RON project files. `src/output/` encodes video through FFmpeg.
- `src/nodes/` has one file per node type, each implementing the `Node` trait; `mod.rs` is the registry and dispatch.
- The app's interface modules sit at the top of `src/` (launcher, preview, graph view, inspector, timeline, menu, history, audio player). List the directory for the current set rather than trusting a list here.
- `shaders/` holds WGSL built into the binary via `include_str!`. `examples/` has example projects and a test audio clip. `docs/` has ROADMAP, ARCHITECTURE and NODE_REFERENCE.

## Conventions

### Adding a new node

Every node lives in its own file under `src/nodes/`. The minimum is:

1. Define a struct holding the node's parameters and any GPU resources it owns.
2. Implement `Node` (`cook`, `input_refs`).
3. In `new()`, load each scalar/color param via `spec.scalar_param(...)?` / `spec.color_param(...)?` so a malformed project fails at `flux check` time, not mid-render.
4. Register it in `src/nodes/mod.rs`'s `node_from_spec` dispatch and in `registered_names()`.
5. Add a `#[cfg(test)] mod tests` block with at least one snapshot test using `crate::test_utils::TestHarness`. See `src/nodes/gradient.rs` for the canonical pattern.
6. Add a section to `docs/NODE_REFERENCE.md`.
7. If it has a shader, put it in `shaders/<node_name>.wgsl` and follow `docs/SHADER_CONVENTIONS.md`.

Aim for ~100 lines per node file. If a node is getting bigger, it's probably actually multiple nodes.

### Shaders

WGSL only for built-in nodes. User shaders load by path through the custom-shader node (`src/nodes/custom_shader.rs`).

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
- **Replace WGSL with GLSL** or wgpu with another GPU API. The stack is chosen.

## On the GUI

The GUI is in-process Rust (egui) and must stay that way, so it shares the wgpu device with the engine — no IPC, no separate language, no per-frame copy through the OS. Anything that adds latency between "user moves slider" and "preview pixel changes" is the wrong shape. Live param tweaks should land in the running engine on the next frame.

## Agent-edit workflow

When the user is running `flux preview <project>`, the project file and every file it references (custom shader `.wgsl`, color-grade LUT PNGs) are watched: any edit to any of them causes the engine's graph to rebuild in place within ~100 ms. **This means an agent can edit a `.ron` or shader file and the user sees the change immediately, no commands needed.**

If you're collaborating with the user on visuals while their preview window is open:

- Edit `.ron` and shader files freely. The preview reloads on save.
- Don't run `flux render` or `flux preview` yourself; the user has the window open. Just edit the source.
- If the user reports the preview didn't update, the most likely cause is a parse or compile error — check `tracing` logs. The engine keeps the old graph on reload failure, so edits with errors don't crash the window.
- Feedback node history is currently lost on reload. Pieces relying on long trails will visibly restart.

## Launching the app

When the user asks to "open it", "boot it up", or "run it for me":

```
cargo run --release                                                    # launcher window (pick project + audio in-app)
cargo run --release -- preview examples/<piece>.ron --audio <clip.wav> # straight into a specific piece
```

Launch as a background task so the session stays free, then watch the tracing output for panics or shader compile errors before declaring it running. `cargo run --release -- check <project.ron>` validates a project without opening a window.

This is the opposite case from the hot-reload workflow above: launch when asked to; once the window is open, go back to editing files and letting the watcher reload them.

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
