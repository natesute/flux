//! `raymarch` — a tiny SDF scene rendered by sphere-tracing. Sphere at the
//! origin with audio-modulated surface ripples; sky gradient; a single
//! directional light. v1 is intentionally small. Users who want richer
//! scenes should clone `shaders/raymarch.wgsl` and load it through the
//! `custom_shader` node instead.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/raymarch.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    cam_pos: [f32; 3],
    fov: f32,
    cam_right: [f32; 3],
    radius: f32,
    cam_up: [f32; 3],
    displacement: f32,
    cam_forward: [f32; 3],
    time: f32,
    light_dir: [f32; 3],
    aspect: f32,
    sky_top: [f32; 3],
    _pad0: f32,
    sky_bottom: [f32; 3],
    _pad1: f32,
    resolution: [f32; 2],
    _pad2: [f32; 2],
}

pub struct RaymarchNode {
    inputs: Vec<String>,
    cam_x: ParamValue,
    cam_y: ParamValue,
    cam_z: ParamValue,
    look_x: ParamValue,
    look_y: ParamValue,
    look_z: ParamValue,
    fov: ParamValue,
    radius: ParamValue,
    displacement: ParamValue,
    light_x: ParamValue,
    light_y: ParamValue,
    light_z: ParamValue,
    sky_top: ParamValue,
    sky_bottom: ParamValue,

    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl RaymarchNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raymarch bgl"),
            entries: &[shader_pass::uniform_entry(0)],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "raymarch", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            cam_x: spec.scalar_param("cam_x", 0.0)?,
            cam_y: spec.scalar_param("cam_y", 0.5)?,
            cam_z: spec.scalar_param("cam_z", 3.0)?,
            look_x: spec.scalar_param("look_x", 0.0)?,
            look_y: spec.scalar_param("look_y", 0.0)?,
            look_z: spec.scalar_param("look_z", 0.0)?,
            fov: spec.scalar_param("fov", 0.9)?,
            radius: spec.scalar_param("radius", 1.0)?,
            displacement: spec.scalar_param("displacement", 0.05)?,
            light_x: spec.scalar_param("light_x", 0.5)?,
            light_y: spec.scalar_param("light_y", 0.8)?,
            light_z: spec.scalar_param("light_z", 0.3)?,
            sky_top: spec.color_param("sky_top", [0.4, 0.6, 0.9, 1.0])?,
            sky_bottom: spec.color_param("sky_bottom", [0.05, 0.05, 0.1, 1.0])?,
            pipeline,
            uniform_buffer,
            bind_group,
        })
    }
}

impl Node for RaymarchNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "raymarch"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.cam_x = spec.scalar_param("cam_x", 0.0)?;
        self.cam_y = spec.scalar_param("cam_y", 0.5)?;
        self.cam_z = spec.scalar_param("cam_z", 3.0)?;
        self.look_x = spec.scalar_param("look_x", 0.0)?;
        self.look_y = spec.scalar_param("look_y", 0.0)?;
        self.look_z = spec.scalar_param("look_z", 0.0)?;
        self.fov = spec.scalar_param("fov", 0.9)?;
        self.radius = spec.scalar_param("radius", 1.0)?;
        self.displacement = spec.scalar_param("displacement", 0.05)?;
        self.light_x = spec.scalar_param("light_x", 0.5)?;
        self.light_y = spec.scalar_param("light_y", 0.8)?;
        self.light_z = spec.scalar_param("light_z", 0.3)?;
        self.sky_top = spec.color_param("sky_top", [0.4, 0.6, 0.9, 1.0])?;
        self.sky_bottom = spec.color_param("sky_bottom", [0.05, 0.05, 0.1, 1.0])?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        // Build the camera basis from position + look-at + world-up.
        let cam = Vec3::new(
            self.cam_x.resolve_scalar(&ctx.audio),
            self.cam_y.resolve_scalar(&ctx.audio),
            self.cam_z.resolve_scalar(&ctx.audio),
        );
        let target = Vec3::new(
            self.look_x.resolve_scalar(&ctx.audio),
            self.look_y.resolve_scalar(&ctx.audio),
            self.look_z.resolve_scalar(&ctx.audio),
        );
        let world_up = Vec3::Y;
        let forward = (target - cam).normalize_or_zero();
        let right = forward.cross(world_up).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();

        let sky_top = self.sky_top.as_color();
        let sky_bottom = self.sky_bottom.as_color();

        let uniforms = Uniforms {
            cam_pos: cam.into(),
            fov: self.fov.resolve_scalar(&ctx.audio),
            cam_right: right.into(),
            radius: self.radius.resolve_scalar(&ctx.audio),
            cam_up: up.into(),
            displacement: self.displacement.resolve_scalar(&ctx.audio),
            cam_forward: forward.into(),
            time: ctx.time,
            light_dir: [
                self.light_x.resolve_scalar(&ctx.audio),
                self.light_y.resolve_scalar(&ctx.audio),
                self.light_z.resolve_scalar(&ctx.audio),
            ],
            aspect: ctx.width as f32 / ctx.height as f32,
            sky_top: [sky_top[0], sky_top[1], sky_top[2]],
            _pad0: 0.0,
            sky_bottom: [sky_bottom[0], sky_bottom[1], sky_bottom[2]],
            _pad1: 0.0,
            resolution: [ctx.width as f32, ctx.height as f32],
            _pad2: [0.0, 0.0],
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "raymarch",
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
    fn renders_default_sphere() {
        let Some(harness) = TestHarness::try_init(64, 64) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(r#"(type: "raymarch")"#).unwrap();
        let mut node = RaymarchNode::new(&spec, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
