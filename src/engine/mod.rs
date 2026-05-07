//! Core rendering engine: GPU context, graph evaluation, frame loop.

mod context;
mod frame;
pub(crate) mod graph;

pub use context::GpuContext;
pub use frame::FrameContext;
pub use graph::{Graph, NodeId};

use std::path::Path;

use anyhow::{Context, Result};

use crate::audio::AudioTrack;
use crate::output::VideoEncoder;
use crate::project::{Project, ToneMap};

/// Top-level engine. Owns the GPU context and the cooked graph.
pub struct Engine {
    pub gpu: GpuContext,
    pub graph: Graph,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub tone_map: ToneMap,
}

impl Engine {
    /// Build an engine from a project. Initializes the GPU device and
    /// instantiates every node in the graph.
    pub fn new(project: &Project) -> Result<Self> {
        let gpu = pollster::block_on(GpuContext::new()).context("creating GPU context")?;
        let graph = Graph::from_project(project, &gpu)?;
        Ok(Self {
            gpu,
            graph,
            width: project.width,
            height: project.height,
            fps: project.fps,
            tone_map: project.tone_map,
        })
    }

    /// Apply a (presumably reloaded) project to the engine.
    ///
    /// Fast path: if the new project has the same topology — same set of
    /// node names in the same order, same `kind` per node, same inputs,
    /// same output, same resolution — then we just patch each node's
    /// parameter values in place. No pipeline recreation, no buffer
    /// allocation, no texture realloc. This is what the slider-drag and
    /// agent-edit hot paths almost always hit, since neither typically
    /// changes the topology.
    ///
    /// Slow path: a full `Graph::from_project` rebuild against the
    /// existing GPU device. Used when nodes are added/removed/rewired,
    /// resolution changes, or a `custom_shader`/`color_grade` path
    /// changes (those need to recompile/reload from disk).
    ///
    /// Either way: GPU device, surface, blit pipeline, and audio stream
    /// all stay alive across the swap. On failure the engine is left
    /// untouched and the error is returned for the caller to log.
    ///
    /// Feedback node history is **not** preserved across slow-path
    /// rebuilds; tracked as a follow-up. The fast path leaves history
    /// alone, which is part of why it's worth taking.
    pub fn rebuild_graph(&mut self, project: &Project) -> Result<()> {
        let resolution_changed = project.width != self.width || project.height != self.height;
        if !resolution_changed && self.graph.topology_matches(project) {
            // Try the fast path; if a node refuses (e.g. custom_shader path
            // changed), fall through to the full rebuild.
            match self.graph.update_params(project) {
                Ok(()) => {
                    self.fps = project.fps;
                    self.tone_map = project.tone_map;
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!("fast-path update_params declined ({e:#}); rebuilding graph");
                }
            }
        }
        let mut graph = Graph::from_project(project, &self.gpu)?;
        // Carry history textures across the swap so feedback trails
        // don't reset every time the topology changes.
        graph.transfer_preservable_state_from(&mut self.graph);
        self.graph = graph;
        self.width = project.width;
        self.height = project.height;
        self.fps = project.fps;
        self.tone_map = project.tone_map;
        Ok(())
    }

    /// Cook one frame and return a borrow of the output texture. The
    /// preview loop calls this once per redraw, then blits the texture to
    /// the surface; offline rendering uses `render_to_file` instead.
    pub fn cook_one_frame(
        &mut self,
        time: f32,
        frame_index: u32,
        audio: crate::audio::FrameAudioFeatures,
    ) -> Result<&wgpu::Texture> {
        let mut frame_ctx = FrameContext {
            gpu: &self.gpu,
            width: self.width,
            height: self.height,
            frame_index,
            time,
            audio,
        };
        self.graph.cook_frame(&mut frame_ctx)?;
        Ok(self.graph.output_texture())
    }

    /// Render the project to a video file, driven by the given audio track.
    pub fn render_to_file(
        &mut self,
        audio_path: &Path,
        out_path: &Path,
        fps_override: Option<u32>,
        duration_override: Option<f32>,
    ) -> Result<()> {
        let track = AudioTrack::load(audio_path)?;
        let fps = fps_override.unwrap_or(self.fps);
        let audio_seconds = track.duration_seconds();
        let requested_seconds = duration_override.unwrap_or(audio_seconds);
        // FFmpeg is invoked with `-shortest`, so writing video frames past
        // the audio length closes the video pipe and crashes the encoder.
        // Clamp explicitly with a clear log line instead of a broken pipe.
        let total_seconds = if requested_seconds > audio_seconds {
            tracing::warn!(
                "requested duration {:.2}s exceeds audio length {:.2}s — clamping",
                requested_seconds,
                audio_seconds
            );
            audio_seconds
        } else {
            requested_seconds
        };
        let total_frames = (total_seconds * fps as f32).round() as u32;

        let mut encoder = VideoEncoder::start(out_path, self.width, self.height, fps, audio_path)?;

        tracing::info!(
            "Rendering {total_frames} frames at {}x{} {fps}fps",
            self.width,
            self.height
        );

        for frame_index in 0..total_frames {
            let time = frame_index as f32 / fps as f32;
            let audio_features = track.features_at(time);
            let mut frame_ctx = FrameContext {
                gpu: &self.gpu,
                width: self.width,
                height: self.height,
                frame_index,
                time,
                audio: audio_features,
            };
            self.graph.cook_frame(&mut frame_ctx)?;
            let pixels = self.graph.read_output_pixels(&frame_ctx, self.tone_map)?;
            encoder.write_frame(&pixels)?;

            if frame_index % fps == 0 {
                tracing::info!("  frame {frame_index}/{total_frames} ({time:.1}s)");
            }
        }

        encoder.finish()?;
        Ok(())
    }
}
