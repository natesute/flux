//! `transform` — translate / rotate / scale an input texture about its center.
//!
//! Any of the five params (`offset_x`, `offset_y`, `rotation`, `scale_x`,
//! `scale_y`) can be bound to an audio feature, so e.g. you can punch the
//! image outward on every kick by binding `scale_x` and `scale_y` to bass.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/transform.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    offset_x: f32,
    offset_y: f32,
    rotation: f32,
    scale_x: f32,
    scale_y: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

pub struct TransformNode {
    inputs: Vec<String>,
    offset_x: ParamValue,
    offset_y: ParamValue,
    rotation: ParamValue,
    scale_x: ParamValue,
    scale_y: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl TransformNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 1 {
            return Err(anyhow!(
                "`transform` requires exactly 1 input, got {}",
                spec.inputs.len()
            ));
        }

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transform bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::sampler_entry(2),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "transform", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transform uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            offset_x: spec.scalar_param("offset_x", 0.0)?,
            offset_y: spec.scalar_param("offset_y", 0.0)?,
            rotation: spec.scalar_param("rotation", 0.0)?,
            scale_x: spec.scalar_param("scale_x", 1.0)?,
            scale_y: spec.scalar_param("scale_y", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
        })
    }
}

impl Node for TransformNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "transform"
    }

    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.offset_x = spec.scalar_param("offset_x", 0.0)?;
        self.offset_y = spec.scalar_param("offset_y", 0.0)?;
        self.rotation = spec.scalar_param("rotation", 0.0)?;
        self.scale_x = spec.scalar_param("scale_x", 1.0)?;
        self.scale_y = spec.scalar_param("scale_y", 1.0)?;
        Ok(())
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
            offset_x: self.offset_x.resolve_scalar(&ctx.audio),
            offset_y: self.offset_y.resolve_scalar(&ctx.audio),
            rotation: self.rotation.resolve_scalar(&ctx.audio),
            scale_x: self.scale_x.resolve_scalar(&ctx.audio),
            scale_y: self.scale_y.resolve_scalar(&ctx.audio),
            ..Default::default()
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("transform bg"),
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

        shader_pass::run_fullscreen_pass(ctx.gpu, "transform", &self.pipeline, &bind_group, output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    /// Identity transform on a constant input should pass it through unchanged.
    #[test]
    fn identity_passes_through() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "transform", inputs: ["src"], params: {
                "offset_x": 0.0, "offset_y": 0.0, "rotation": 0.0,
                "scale_x": 1.0, "scale_y": 1.0,
            })"#,
        )
        .unwrap();
        let mut node = TransformNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.4, 0.4, 0.4, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] = &[("src".to_string(), &src)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
