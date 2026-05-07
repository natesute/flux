//! `noise` — animated 2D fractal noise (FBM over Simplex). Generator node,
//! no inputs. Excellent organic motion source; pair with `displace` (later)
//! or feed into `feedback` to build flowing textures.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/noise.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    color_a: [f32; 4],
    color_b: [f32; 4],
    resolution: [f32; 2],
    scale: f32,
    speed: f32,
    octaves: f32,
    contrast: f32,
    intensity: f32,
    time: f32,
}

pub struct NoiseNode {
    inputs: Vec<String>,
    color_a: ParamValue,
    color_b: ParamValue,
    scale: ParamValue,
    speed: ParamValue,
    octaves: ParamValue,
    contrast: ParamValue,
    intensity: ParamValue,

    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl NoiseNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("noise bgl"),
            entries: &[shader_pass::uniform_entry(0)],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "noise", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("noise uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("noise bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            color_a: spec.color_param("color_a", [0.0, 0.0, 0.0, 1.0])?,
            color_b: spec.color_param("color_b", [1.0, 1.0, 1.0, 1.0])?,
            scale: spec.scalar_param("scale", 3.0)?,
            speed: spec.scalar_param("speed", 0.3)?,
            octaves: spec.scalar_param("octaves", 4.0)?,
            contrast: spec.scalar_param("contrast", 1.0)?,
            intensity: spec.scalar_param("intensity", 1.0)?,
            pipeline,
            uniform_buffer,
            bind_group,
        })
    }
}

impl Node for NoiseNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "noise"
    }

    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.color_a = spec.color_param("color_a", [0.0, 0.0, 0.0, 1.0])?;
        self.color_b = spec.color_param("color_b", [1.0, 1.0, 1.0, 1.0])?;
        self.scale = spec.scalar_param("scale", 3.0)?;
        self.speed = spec.scalar_param("speed", 0.3)?;
        self.octaves = spec.scalar_param("octaves", 4.0)?;
        self.contrast = spec.scalar_param("contrast", 1.0)?;
        self.intensity = spec.scalar_param("intensity", 1.0)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let uniforms = Uniforms {
            color_a: self.color_a.as_color(),
            color_b: self.color_b.as_color(),
            resolution: [ctx.width as f32, ctx.height as f32],
            scale: self.scale.resolve_scalar(&ctx.audio),
            speed: self.speed.resolve_scalar(&ctx.audio),
            octaves: self.octaves.resolve_scalar(&ctx.audio),
            contrast: self.contrast.resolve_scalar(&ctx.audio),
            intensity: self.intensity.resolve_scalar(&ctx.audio),
            time: ctx.time,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "noise",
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

    #[test]
    fn renders_default_fbm() {
        let Some(harness) = TestHarness::try_init(64, 64) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "noise", params: {
                "color_a": [0.0, 0.0, 0.0, 1.0],
                "color_b": [1.0, 1.0, 1.0, 1.0],
                "scale": 3.0,
                "speed": 0.0,
                "octaves": 3.0,
                "contrast": 1.0,
                "intensity": 1.0,
            })"#,
        )
        .unwrap();
        let mut node = NoiseNode::new(&spec, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
