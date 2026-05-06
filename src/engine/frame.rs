//! Per-frame state. Created fresh for every rendered frame.

use crate::audio::FrameAudioFeatures;
use crate::engine::GpuContext;

/// Everything a node needs to know about the current frame.
///
/// Passed by mutable reference to `Node::cook` so nodes can record GPU
/// commands; the underlying `gpu` field is shared.
pub struct FrameContext<'a> {
    pub gpu: &'a GpuContext,
    pub width: u32,
    pub height: u32,
    pub frame_index: u32,
    pub time: f32,
    pub audio: FrameAudioFeatures,
}
