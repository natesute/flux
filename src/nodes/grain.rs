//! `grain` — animated film grain overlay.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/grain.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    amount: f32,
    scale: f32,
    time: f32,
    _pad: f32,
}

pub struct GrainNode {
    inputs: Vec<String>,
    amount: ParamValue,
    scale: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
}

impl GrainNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grain bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::sampler_entry(2),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "grain", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            amount: spec.scalar_param("amount", 0.04)?,
            scale: spec.scalar_param("scale", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            bind_group: None,
        })
    }
}

impl Node for GrainNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "grain"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn expected_input_count(&self) -> usize {
        1
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.amount = spec.scalar_param("amount", 0.04)?;
        self.scale = spec.scalar_param("scale", 1.0)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        if self.bind_group.is_none() {
            let view_in = inputs[0].create_view(&wgpu::TextureViewDescriptor::default());
            self.bind_group = Some(
                ctx.gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("grain bg"),
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
                    }),
            );
        }

        let uniforms = Uniforms {
            amount: self.amount.resolve_scalar(&ctx.audio),
            scale: self.scale.resolve_scalar(&ctx.audio),
            time: ctx.time,
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "grain",
            &self.pipeline,
            self.bind_group.as_ref().unwrap(),
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

    /// Zero amount means no grain — output equals input exactly.
    #[test]
    fn zero_amount_passes_through() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "grain", inputs: ["src"], params: { "amount": 0.0, "scale": 1.0 })"#,
        )
        .unwrap();
        let mut node = GrainNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
