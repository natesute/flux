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
        let (_module, pipeline) =
            shader_pass::build_fullscreen_pipeline(gpu, "gradient", SHADER, &bgl);

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
            inner_color: spec
                .params
                .get("inner_color")
                .cloned()
                .unwrap_or(ParamValue::Color(vec![1.0, 1.0, 1.0, 1.0])),
            outer_color: spec
                .params
                .get("outer_color")
                .cloned()
                .unwrap_or(ParamValue::Color(vec![0.0, 0.0, 0.0, 1.0])),
            radius: spec
                .params
                .get("radius")
                .cloned()
                .unwrap_or(ParamValue::Number(0.5)),
            intensity: spec
                .params
                .get("intensity")
                .cloned()
                .unwrap_or(ParamValue::Number(1.0)),
            pipeline,
            uniform_buffer,
            bind_group,
        })
    }
}

impl Node for GradientNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[(String, &wgpu::Texture)],
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
