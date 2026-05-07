//! `gradient` — radial gradient.
//!
//! Reference example for the "fullscreen shader, no input textures" pattern.
//! For nodes that take input textures, see `noise.rs` (no input) or
//! `feedback.rs` and `blend.rs` (input textures).

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/gradient.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    inner_color: [f32; 4],
    outer_color: [f32; 4],
    center: [f32; 2],
    radius: f32,
    intensity: f32,
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
}

pub struct GradientNode {
    inputs: Vec<String>,
    inner_color: ParamValue,
    outer_color: ParamValue,
    radius: ParamValue,
    intensity: ParamValue,

    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GradientNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gradient bgl"),
            entries: &[shader_pass::uniform_entry(0)],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "gradient", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gradient uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gradient bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            inner_color: spec.color_param("inner_color", [1.0, 1.0, 1.0, 1.0])?,
            outer_color: spec.color_param("outer_color", [0.0, 0.0, 0.0, 1.0])?,
            radius: spec.scalar_param("radius", 0.5)?,
            intensity: spec.scalar_param("intensity", 1.0)?,
            pipeline,
            uniform_buffer,
            bind_group,
        })
    }
}

impl Node for GradientNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "gradient"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.inner_color = spec.color_param("inner_color", [1.0, 1.0, 1.0, 1.0])?;
        self.outer_color = spec.color_param("outer_color", [0.0, 0.0, 0.0, 1.0])?;
        self.radius = spec.scalar_param("radius", 0.5)?;
        self.intensity = spec.scalar_param("intensity", 1.0)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let uniforms = Uniforms {
            inner_color: self.inner_color.as_color(),
            outer_color: self.outer_color.as_color(),
            center: [0.5, 0.5],
            radius: self.radius.resolve_scalar(&ctx.audio),
            intensity: self.intensity.resolve_scalar(&ctx.audio),
            resolution: [ctx.width as f32, ctx.height as f32],
            time: ctx.time,
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "gradient",
            &self.pipeline,
            &self.bind_group,
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

    /// Canonical node-test pattern. Copy this for new nodes:
    ///   1. `try_init` the harness (skip if no GPU).
    ///   2. Build a `NodeSpec` from a literal RON string.
    ///   3. `cook` one frame and snapshot the resulting stats.
    #[test]
    fn renders_default_radius() {
        let Some(harness) = TestHarness::try_init(64, 64) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "gradient", params: {
                "inner_color": [1.0, 1.0, 1.0, 1.0],
                "outer_color": [0.0, 0.0, 0.0, 1.0],
                "radius": 0.5,
                "intensity": 1.0,
            })"#,
        )
        .unwrap();
        let mut node = GradientNode::new(&spec, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
