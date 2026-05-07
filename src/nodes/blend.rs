//! `blend` — composite two input textures.
//!
//! Inputs (in order): `a` (background), `b` (foreground/overlay).
//! Modes: `over`, `add`, `multiply`, `screen`, `mix`.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/blend.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    factor: f32,
    opacity: f32,
    _pad3: f32,
    _pad4: f32,
}

pub struct BlendNode {
    inputs: Vec<String>,
    mode: u32,
    factor: ParamValue,
    opacity: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

fn parse_mode(s: &str) -> Result<u32> {
    Ok(match s {
        "over" => 0,
        "add" => 1,
        "multiply" => 2,
        "screen" => 3,
        "mix" => 4,
        other => {
            return Err(anyhow!(
                "unknown blend mode `{other}`. Valid: over, add, multiply, screen, mix"
            ))
        }
    })
}

impl BlendNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 2 {
            return Err(anyhow!(
                "`blend` requires exactly 2 inputs, got {}",
                spec.inputs.len()
            ));
        }

        let mode_str = spec
            .params
            .get("mode")
            .and_then(|v| v.as_string())
            .unwrap_or("over");
        let mode = parse_mode(mode_str)?;

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::texture_entry(2),
                shader_pass::sampler_entry(3),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "blend", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blend uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            mode,
            factor: spec.scalar_param("factor", 1.0)?,
            opacity: spec.scalar_param("opacity", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
        })
    }
}

impl Node for BlendNode {
    fn kind(&self) -> &'static str {
        "blend"
    }

    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        let mode_str = spec
            .params
            .get("mode")
            .and_then(|v| v.as_string())
            .unwrap_or("over");
        self.mode = parse_mode(mode_str)?;
        self.factor = spec.scalar_param("factor", 1.0)?;
        self.opacity = spec.scalar_param("opacity", 1.0)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let view_a = inputs[0]
            .1
            .create_view(&wgpu::TextureViewDescriptor::default());
        let view_b = inputs[1]
            .1
            .create_view(&wgpu::TextureViewDescriptor::default());

        let uniforms = Uniforms {
            mode: self.mode,
            factor: self.factor.resolve_scalar(&ctx.audio),
            opacity: self.opacity.resolve_scalar(&ctx.audio),
            ..Default::default()
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Bind group is rebuilt per frame because the input texture views
        // are not stable across renders. This is cheap.
        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blend bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view_a),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view_b),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

        shader_pass::run_fullscreen_pass(ctx.gpu, "blend", &self.pipeline, &bind_group, output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    #[test]
    fn add_mode_sums_inputs() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "blend", inputs: ["a", "b"], params: { "mode": "add", "factor": 1.0, "opacity": 1.0 })"#,
        )
        .unwrap();
        let mut node = BlendNode::new(&spec, &harness.gpu).unwrap();
        let tex_a = harness.constant_texture([0.3, 0.0, 0.0, 1.0]);
        let tex_b = harness.constant_texture([0.0, 0.5, 0.0, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] =
            &[("a".to_string(), &tex_a), ("b".to_string(), &tex_b)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
