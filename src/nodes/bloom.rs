//! `bloom` — adds a soft glow around bright areas of the input.
//!
//! Implementation note: this is a single-pass version. Real bloom is
//! multi-pass (downsampled mip chain with successive gaussian blurs)
//! which gives a wider, more natural-looking glow. The single-pass
//! version is cheaper and good enough for tight radii. Upgrading to
//! multi-pass is a future improvement; the API of this node will not
//! change when that lands.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/bloom.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    threshold: f32,
    intensity: f32,
    radius: f32,
    _pad: f32,
}

pub struct BloomNode {
    inputs: Vec<String>,
    threshold: ParamValue,
    intensity: ParamValue,
    radius: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Bind group is cached across frames — input texture handles are
    /// stable within an Engine instance, and slow-path rebuilds make a
    /// fresh node anyway, so we never need to invalidate.
    bind_group: Option<wgpu::BindGroup>,
}

impl BloomNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::sampler_entry(2),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "bloom", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            threshold: spec.scalar_param("threshold", 0.7)?,
            intensity: spec.scalar_param("intensity", 1.0)?,
            radius: spec.scalar_param("radius", 4.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            bind_group: None,
        })
    }
}

impl Node for BloomNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "bloom"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn expected_input_count(&self) -> usize {
        1
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.threshold = spec.scalar_param("threshold", 0.7)?;
        self.intensity = spec.scalar_param("intensity", 1.0)?;
        self.radius = spec.scalar_param("radius", 4.0)?;
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
                        label: Some("bloom bg"),
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
            threshold: self.threshold.resolve_scalar(&ctx.audio),
            intensity: self.intensity.resolve_scalar(&ctx.audio),
            radius: self.radius.resolve_scalar(&ctx.audio),
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "bloom",
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

    #[test]
    fn passes_through_below_threshold() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "bloom", inputs: ["src"], params: { "threshold": 1.0, "intensity": 1.0, "radius": 4.0 })"#,
        )
        .unwrap();
        let mut node = BloomNode::new(&spec, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.4, 0.4, 0.4, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
