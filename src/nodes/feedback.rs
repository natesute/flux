//! `feedback` — blends the current input with a transformed copy of the
//! previous frame. Single most important node for the TouchDesigner-style
//! "trail" aesthetic.
//!
//! The previous frame is kept in a private `history` texture, copied from
//! the output texture *after* each cook. A small but important detail: on
//! frame 0 there is no history, so we sample black instead.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/feedback.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    decay: f32,
    zoom: f32,
    rotation: f32,
    offset_x: f32,
    offset_y: f32,
    mix_in: f32,
    _pad0: f32,
    _pad1: f32,
}

pub struct FeedbackNode {
    inputs: Vec<String>,
    decay: ParamValue,
    zoom: ParamValue,
    rotation: ParamValue,
    offset_x: ParamValue,
    offset_y: ParamValue,
    mix_in: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,

    /// Previous frame's output. Initialized lazily on first cook so we
    /// can match `ctx.width/height` (which we don't know at construction).
    history: Option<wgpu::Texture>,
    /// Black 1x1 texture used the first frame before history exists.
    black: wgpu::Texture,
}

impl FeedbackNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 1 {
            return Err(anyhow!(
                "`feedback` requires exactly 1 input, got {}",
                spec.inputs.len()
            ));
        }

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("feedback bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::texture_entry(2),
                shader_pass::sampler_entry(3),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "feedback", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("feedback uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 1x1 black fallback texture for the first frame.
        let black = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("feedback black"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.texture_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Write four f16 zeroes (8 bytes) into it.
        gpu.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &black,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8; 8],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        Ok(Self {
            inputs: spec.inputs.clone(),
            decay: spec.scalar_param("decay", 0.92)?,
            zoom: spec.scalar_param("zoom", 1.01)?,
            rotation: spec.scalar_param("rotation", 0.0)?,
            offset_x: spec.scalar_param("offset_x", 0.0)?,
            offset_y: spec.scalar_param("offset_y", 0.0)?,
            mix_in: spec.scalar_param("mix_in", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            history: None,
            black,
        })
    }

    fn ensure_history(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        let needs_alloc = match &self.history {
            None => true,
            Some(t) => t.width() != width || t.height() != height,
        };
        if needs_alloc {
            self.history = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("feedback history"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: gpu.texture_format(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
    }
}

impl Node for FeedbackNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        self.ensure_history(ctx.gpu, ctx.width, ctx.height);
        let history_tex = self
            .history
            .as_ref()
            .expect("ensure_history just allocated");

        // Pick history source: real history on frames > 0, black on frame 0.
        let history_view = if ctx.frame_index == 0 {
            self.black
                .create_view(&wgpu::TextureViewDescriptor::default())
        } else {
            history_tex.create_view(&wgpu::TextureViewDescriptor::default())
        };

        let current_view = inputs[0]
            .1
            .create_view(&wgpu::TextureViewDescriptor::default());

        let uniforms = Uniforms {
            decay: self.decay.resolve_scalar(&ctx.audio),
            zoom: self.zoom.resolve_scalar(&ctx.audio),
            rotation: self.rotation.resolve_scalar(&ctx.audio),
            offset_x: self.offset_x.resolve_scalar(&ctx.audio),
            offset_y: self.offset_y.resolve_scalar(&ctx.audio),
            mix_in: self.mix_in.resolve_scalar(&ctx.audio),
            _pad0: 0.0,
            _pad1: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("feedback bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&current_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&history_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

        shader_pass::run_fullscreen_pass(ctx.gpu, "feedback", &self.pipeline, &bind_group, output);

        // Copy output to history for the next frame.
        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("feedback->history"),
            });
        encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: output,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyTexture {
                texture: history_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: ctx.width,
                height: ctx.height,
                depth_or_array_layers: 1,
            },
        );
        ctx.gpu.queue.submit(Some(encoder.finish()));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    /// Frame 0 has no history, so output should equal input * mix_in.
    #[test]
    fn frame_zero_passes_input_through() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "feedback", inputs: ["src"], params: {
                "decay": 0.9, "zoom": 1.01, "rotation": 0.0,
                "offset_x": 0.0, "offset_y": 0.0, "mix_in": 1.0,
            })"#,
        )
        .unwrap();
        let mut node = FeedbackNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] = &[("src".to_string(), &src)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
