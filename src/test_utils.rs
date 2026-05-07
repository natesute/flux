//! Helpers for node snapshot tests.
//!
//! See `src/nodes/gradient.rs` for the canonical usage pattern. The shape
//! of every node test is the same:
//!
//! ```ignore
//! #[test]
//! fn renders_default() {
//!     let Some(harness) = TestHarness::try_init(64, 64) else { return };
//!     let spec = ron::from_str(r#"(type: "gradient", params: { "radius": 0.5 })"#).unwrap();
//!     let mut node = GradientNode::new(&spec, &harness.gpu).unwrap();
//!     let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
//!     insta::assert_snapshot!(stats);
//! }
//! ```
//!
//! Snapshots store coarse statistics (per-channel mean/min/max plus a
//! non-black pixel count) instead of raw bytes. That's stable across GPU
//! drivers — what we want to catch is "node went black" or "node hue
//! shifted dramatically", not sub-pixel floating-point variance.

use std::fmt;

use anyhow::Result;

use crate::audio::FrameAudioFeatures;
use crate::engine::{FrameContext, GpuContext};
use crate::nodes::Node;
use crate::project::ToneMap;

/// All the GPU plumbing a node test needs. Construct once per test with
/// [`TestHarness::try_init`]; each call to [`TestHarness::cook`] produces
/// fresh, deterministic stats for one frame of the node.
pub struct TestHarness {
    pub gpu: GpuContext,
    pub width: u32,
    pub height: u32,
}

impl TestHarness {
    /// Boot a GPU context for testing. Returns `None` when no adapter is
    /// available — callers should early-return rather than fail. CI without
    /// GPU support will skip these tests automatically.
    pub fn try_init(width: u32, height: u32) -> Option<Self> {
        let gpu = pollster::block_on(GpuContext::new()).ok()?;
        Some(Self { gpu, width, height })
    }

    /// Allocate an engine-format output texture with the same usage flags
    /// `Graph` uses, so node behavior matches production.
    pub fn alloc_output(&self) -> wgpu::Texture {
        self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test output"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.gpu.texture_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Allocate and clear an engine-format texture to a constant color.
    /// Useful for synthesizing input textures for nodes that take inputs
    /// (blend, bloom, feedback) without having to cook an upstream node.
    pub fn constant_texture(&self, color: [f32; 4]) -> wgpu::Texture {
        let tex = self.alloc_output();
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("constant_texture"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: color[0] as f64,
                            g: color[1] as f64,
                            b: color[2] as f64,
                            a: color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.gpu.queue.submit(Some(encoder.finish()));
        tex
    }

    /// Run one cook of `node` with the given inputs, audio, and time. Returns
    /// the tone-mapped RGBA8 pixels (same as what would be sent to FFmpeg).
    pub fn cook_to_pixels(
        &self,
        node: &mut dyn Node,
        inputs: &[&wgpu::Texture],
        audio: FrameAudioFeatures,
        time: f32,
    ) -> Result<Vec<u8>> {
        let output = self.alloc_output();
        let ctx = FrameContext {
            gpu: &self.gpu,
            width: self.width,
            height: self.height,
            frame_index: 0,
            time,
            audio,
        };
        node.cook(&ctx, inputs, &output)?;
        crate::engine::graph::readback::texture_to_rgba8(
            &self.gpu,
            &output,
            self.width,
            self.height,
            ToneMap::Aces,
        )
    }

    /// Cook one frame and return summary statistics ready for snapshotting.
    pub fn cook(
        &self,
        node: &mut dyn Node,
        inputs: &[&wgpu::Texture],
        audio: FrameAudioFeatures,
        time: f32,
    ) -> ImageStats {
        let pixels = self
            .cook_to_pixels(node, inputs, audio, time)
            .expect("cook failed");
        ImageStats::from_rgba8(&pixels, self.width, self.height)
    }
}

/// Coarse per-channel statistics over an RGBA8 buffer. Stable across minor
/// floating-point variance, sensitive to "node went black" or "color shifted
/// hard" regressions.
pub struct ImageStats {
    pub width: u32,
    pub height: u32,
    pub mean: [f32; 4],
    pub min: [u8; 4],
    pub max: [u8; 4],
    /// Pixels whose RGB sum exceeds 0; useful for catching "all black" bugs.
    pub nonblack_pixels: u32,
}

impl ImageStats {
    pub fn from_rgba8(rgba: &[u8], width: u32, height: u32) -> Self {
        let total = (width * height) as usize;
        assert_eq!(rgba.len(), total * 4, "rgba buffer size mismatch");

        let mut sum = [0u64; 4];
        let mut min = [u8::MAX; 4];
        let mut max = [0u8; 4];
        let mut nonblack = 0u32;

        for px in rgba.chunks_exact(4) {
            for c in 0..4 {
                sum[c] += px[c] as u64;
                min[c] = min[c].min(px[c]);
                max[c] = max[c].max(px[c]);
            }
            if px[0] as u32 + px[1] as u32 + px[2] as u32 > 0 {
                nonblack += 1;
            }
        }

        let mean = [
            sum[0] as f32 / total as f32,
            sum[1] as f32 / total as f32,
            sum[2] as f32 / total as f32,
            sum[3] as f32 / total as f32,
        ];

        Self {
            width,
            height,
            mean,
            min,
            max,
            nonblack_pixels: nonblack,
        }
    }
}

impl fmt::Display for ImageStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}x{}", self.width, self.height)?;
        writeln!(
            f,
            "mean  R={:>5.1} G={:>5.1} B={:>5.1} A={:>5.1}",
            self.mean[0], self.mean[1], self.mean[2], self.mean[3]
        )?;
        writeln!(
            f,
            "range R={:>3}-{:<3} G={:>3}-{:<3} B={:>3}-{:<3} A={:>3}-{:<3}",
            self.min[0],
            self.max[0],
            self.min[1],
            self.max[1],
            self.min[2],
            self.max[2],
            self.min[3],
            self.max[3]
        )?;
        write!(
            f,
            "nonblack {}/{}",
            self.nonblack_pixels,
            self.width * self.height
        )
    }
}
