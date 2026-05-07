//! `displace` — warp one input by another. The bridge between generators
//! (like `noise`) and final imagery: feed noise into a `displace` and the
//! noise becomes a flow field that pushes the source around.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/displace.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    amount: f32,
    mode: u32,
    _pad0: f32,
    _pad1: f32,
}

pub struct DisplaceNode {
    inputs: Vec<String>,
    amount: ParamValue,
    mode: u32,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
}

fn parse_mode(s: &str) -> Result<u32> {
    Ok(match s {
        "derivative" => 0,
        "vector" => 1,
        other => {
            return Err(anyhow!(
                "unknown displace mode `{other}`. Valid: derivative, vector"
            ))
        }
    })
}

impl DisplaceNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let mode = parse_mode(
            spec.params
                .get("mode")
                .and_then(|v| v.as_string())
                .unwrap_or("derivative"),
        )?;

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("displace bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::texture_entry(2),
                shader_pass::sampler_entry(3),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "displace", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("displace uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            amount: spec.scalar_param("amount", 0.05)?,
            mode,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            bind_group: None,
        })
    }
}

impl Node for DisplaceNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "displace"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn expected_input_count(&self) -> usize {
        2
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.amount = spec.scalar_param("amount", 0.05)?;
        self.mode = parse_mode(
            spec.params
                .get("mode")
                .and_then(|v| v.as_string())
                .unwrap_or("derivative"),
        )?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        if self.bind_group.is_none() {
            let view_src = inputs[0].create_view(&wgpu::TextureViewDescriptor::default());
            let view_map = inputs[1].create_view(&wgpu::TextureViewDescriptor::default());
            self.bind_group = Some(
                ctx.gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("displace bg"),
                        layout: &self.bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: self.uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&view_src),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&view_map),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                        ],
                    }),
            );
        }

        let uniforms = Uniforms {
            amount: self.amount.resolve_scalar(&ctx.audio),
            mode: self.mode,
            ..Default::default()
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "displace",
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

    /// A flat (constant-color) map produces zero gradient, so derivative-mode
    /// displacement is identity. Output should equal the source.
    #[test]
    fn flat_map_is_identity() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "displace", inputs: ["src", "map"], params: {
                "amount": 0.5, "mode": "derivative",
            })"#,
        )
        .unwrap();
        let mut node = DisplaceNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let map = harness.constant_texture([0.5, 0.5, 0.5, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src, &map];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }

    /// Vector mode with a 0.5/0.5 map ((R,G) = (0,0) after recentering)
    /// is also identity — verifies the mode switch works.
    #[test]
    fn vector_mode_neutral_map_is_identity() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "displace", inputs: ["src", "map"], params: {
                "amount": 0.5, "mode": "vector",
            })"#,
        )
        .unwrap();
        let mut node = DisplaceNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let map = harness.constant_texture([0.5, 0.5, 0.0, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src, &map];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
