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
use crate::project::Project;

/// Top-level engine. Owns the GPU context and the cooked graph.
pub struct Engine {
    pub gpu: GpuContext,
    pub graph: Graph,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
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
        })
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
        let total_seconds = duration_override.unwrap_or_else(|| track.duration_seconds());
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
            let pixels = self.graph.read_output_pixels(&frame_ctx)?;
            encoder.write_frame(&pixels)?;

            if frame_index % fps == 0 {
                tracing::info!("  frame {frame_index}/{total_frames} ({time:.1}s)");
            }
        }

        encoder.finish()?;
        Ok(())
    }
}
