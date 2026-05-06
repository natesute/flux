//! `levels` — gain, brightness, contrast, saturation. Per-pixel adjustments.
//!
//! Order of operations: gain → brightness → contrast → saturation. Identity
//! values are gain=1, brightness=0, contrast=1, saturation=1.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/levels.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    gain: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
}

pub struct LevelsNode {
    inputs: Vec<String>,
    gain: ParamValue,
    brightness: ParamValue,
    contrast: ParamValue,
    saturation: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl LevelsNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 1 {
            return Err(anyhow!(
                "`levels` requires exactly 1 input, got {}",
                spec.inputs.len()
            ));
        }

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("levels bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::sampler_entry(2),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "levels", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("levels uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            gain: spec.scalar_param("gain", 1.0)?,
            brightness: spec.scalar_param("brightness", 0.0)?,
            contrast: spec.scalar_param("contrast", 1.0)?,
            saturation: spec.scalar_param("saturation", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
        })
    }
}

impl Node for LevelsNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let view_in = inputs[0]
            .1
            .create_view(&wgpu::TextureViewDescriptor::default());

        let uniforms = Uniforms {
            gain: self.gain.resolve_scalar(&ctx.audio),
            brightness: self.brightness.resolve_scalar(&ctx.audio),
            contrast: self.contrast.resolve_scalar(&ctx.audio),
            saturation: self.saturation.resolve_scalar(&ctx.audio),
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("levels bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view_in),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

        shader_pass::run_fullscreen_pass(ctx.gpu, "levels", &self.pipeline, &bind_group, output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    /// Identity values pass the input through unchanged.
    #[test]
    fn identity_is_passthrough() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "levels", inputs: ["src"], params: {
                "gain": 1.0, "brightness": 0.0, "contrast": 1.0, "saturation": 1.0,
            })"#,
        )
        .unwrap();
        let mut node = LevelsNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.4, 0.4, 0.4, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] = &[("src".to_string(), &src)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }

    /// Saturation = 0 collapses RGB to luma — should produce a uniform grey.
    #[test]
    fn zero_saturation_grays_out() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "levels", inputs: ["src"], params: {
                "gain": 1.0, "brightness": 0.0, "contrast": 1.0, "saturation": 0.0,
            })"#,
        )
        .unwrap();
        let mut node = LevelsNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.7, 0.2, 0.3, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] = &[("src".to_string(), &src)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
