//! `chromatic_aberration` — cheap-lens RGB channel splitting around a pivot.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/chromatic_aberration.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    amount: f32,
    center_x: f32,
    center_y: f32,
    _pad: f32,
}

pub struct ChromaticAberrationNode {
    inputs: Vec<String>,
    amount: ParamValue,
    center_x: ParamValue,
    center_y: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl ChromaticAberrationNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 1 {
            return Err(anyhow!(
                "`chromatic_aberration` requires exactly 1 input, got {}",
                spec.inputs.len()
            ));
        }

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chromatic_aberration bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::sampler_entry(2),
            ],
        });
        let pipeline =
            shader_pass::build_fullscreen_pipeline(gpu, "chromatic_aberration", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chromatic_aberration uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            amount: spec.scalar_param("amount", 0.005)?,
            center_x: spec.scalar_param("center_x", 0.5)?,
            center_y: spec.scalar_param("center_y", 0.5)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
        })
    }
}

impl Node for ChromaticAberrationNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "chromatic_aberration"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.amount = spec.scalar_param("amount", 0.005)?;
        self.center_x = spec.scalar_param("center_x", 0.5)?;
        self.center_y = spec.scalar_param("center_y", 0.5)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let view_in = inputs[0].create_view(&wgpu::TextureViewDescriptor::default());

        let uniforms = Uniforms {
            amount: self.amount.resolve_scalar(&ctx.audio),
            center_x: self.center_x.resolve_scalar(&ctx.audio),
            center_y: self.center_y.resolve_scalar(&ctx.audio),
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("chromatic_aberration bg"),
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

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "chromatic_aberration",
            &self.pipeline,
            &bind_group,
            output,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    #[test]
    fn zero_amount_passes_through() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "chromatic_aberration", inputs: ["src"], params: { "amount": 0.0 })"#,
        )
        .unwrap();
        let mut node = ChromaticAberrationNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
