//! `instance` — render an instanced 3D cube grid. The first geometry node
//! in flux: it owns vertex/index/instance buffers and a depth attachment,
//! all of which were absent until now.
//!
//! v1 is hardcoded: a 4×4×4 grid of cubes spaced evenly around the origin,
//! each tinted by its grid position, with per-frame scale modulated by
//! audio. Camera params are user-supplied.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/instance.wgsl");
const GRID_SIZE: i32 = 4;
const GRID_SPACING: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 3],
    audio_scale: f32,
    rim_color: [f32; 3],
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct InstanceData {
    offset: [f32; 3],
    scale: f32,
    color: [f32; 3],
    _pad: f32,
}

pub struct InstanceNode {
    inputs: Vec<String>,
    cam_x: ParamValue,
    cam_y: ParamValue,
    cam_z: ParamValue,
    look_x: ParamValue,
    look_y: ParamValue,
    look_z: ParamValue,
    fov: ParamValue,
    base_scale: ParamValue,
    audio_drive: ParamValue,
    light_x: ParamValue,
    light_y: ParamValue,
    light_z: ParamValue,
    rim_color: ParamValue,

    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    index_count: u32,
    instance_count: u32,

    /// Lazily-allocated depth texture; matches output dims.
    depth: Option<wgpu::Texture>,
}

impl InstanceNode {
    pub fn new(spec: &NodeSpec, gpu: &GpuContext) -> Result<Self> {
        let device = &gpu.device;

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("instance layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("instance"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("instance pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 0,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 12,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                        ],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<InstanceData>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 2,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 12,
                                shader_location: 3,
                                format: wgpu::VertexFormat::Float32,
                            },
                            wgpu::VertexAttribute {
                                offset: 16,
                                shader_location: 4,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 28,
                                shader_location: 5,
                                format: wgpu::VertexFormat::Float32,
                            },
                        ],
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.texture_format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let (vertices, indices) = cube_mesh();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance vertices"),
            size: std::mem::size_of_val(vertices.as_slice()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue
            .write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance indices"),
            size: std::mem::size_of_val(indices.as_slice()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue
            .write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));

        let instances = grid_instances();
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance data"),
            size: std::mem::size_of_val(instances.as_slice()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue
            .write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("instance bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            inputs: spec.inputs.clone(),
            cam_x: spec.scalar_param("cam_x", 4.0)?,
            cam_y: spec.scalar_param("cam_y", 3.0)?,
            cam_z: spec.scalar_param("cam_z", 6.0)?,
            look_x: spec.scalar_param("look_x", 0.0)?,
            look_y: spec.scalar_param("look_y", 0.0)?,
            look_z: spec.scalar_param("look_z", 0.0)?,
            fov: spec.scalar_param("fov", 0.8)?,
            base_scale: spec.scalar_param("base_scale", 0.25)?,
            audio_drive: spec.scalar_param("audio_drive", 1.0)?,
            light_x: spec.scalar_param("light_x", 0.5)?,
            light_y: spec.scalar_param("light_y", 0.8)?,
            light_z: spec.scalar_param("light_z", 0.4)?,
            rim_color: spec.color_param("rim_color", [1.0, 0.7, 0.4, 1.0])?,
            pipeline,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            bind_group,
            index_count: indices.len() as u32,
            instance_count: instances.len() as u32,
            depth: None,
        })
    }

    fn ensure_depth(&mut self, gpu: &GpuContext, width: u32, height: u32) -> &wgpu::Texture {
        let needs_alloc = match &self.depth {
            None => true,
            Some(t) => t.width() != width || t.height() != height,
        };
        if needs_alloc {
            self.depth = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("instance depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }));
        }
        self.depth.as_ref().unwrap()
    }
}

impl Node for InstanceNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
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
        let view = Mat4::look_at_rh(cam, target, Vec3::Y);
        let aspect = ctx.width as f32 / ctx.height as f32;
        let proj = Mat4::perspective_rh(self.fov.resolve_scalar(&ctx.audio), aspect, 0.1, 100.0);
        let view_proj = proj * view;

        let rim = self.rim_color.as_color();
        let audio_scale = self.base_scale.resolve_scalar(&ctx.audio)
            * (1.0 + self.audio_drive.resolve_scalar(&ctx.audio) * ctx.audio.rms);

        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            light_dir: [
                self.light_x.resolve_scalar(&ctx.audio),
                self.light_y.resolve_scalar(&ctx.audio),
                self.light_z.resolve_scalar(&ctx.audio),
            ],
            audio_scale,
            rim_color: [rim[0], rim[1], rim[2]],
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // ensure_depth borrows self mutably; release that before the pass borrows self.pipeline.
        let depth_view = {
            let depth = self.ensure_depth(ctx.gpu, ctx.width, ctx.height);
            depth.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color_view = output.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("instance"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("instance pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
        }
        ctx.gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }
}

/// Build a cube with per-face normals (24 vertices, 6 faces × 4 corners).
fn cube_mesh() -> (Vec<Vertex>, Vec<u32>) {
    // Each face is a (normal, two basis vectors) triple. Corners go CCW
    // when viewed from outside.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // +Z
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]), // +X
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), // -X
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]), // +Y
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), // -Y
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, (normal, u, v)) in faces.iter().enumerate() {
        let n = Vec3::from(*normal);
        let uvec = Vec3::from(*u);
        let vvec = Vec3::from(*v);
        let center = n * 0.5;
        // Four corners, order CCW from outside.
        let corners = [
            center - uvec * 0.5 - vvec * 0.5,
            center + uvec * 0.5 - vvec * 0.5,
            center + uvec * 0.5 + vvec * 0.5,
            center - uvec * 0.5 + vvec * 0.5,
        ];
        let base = (face_idx * 4) as u32;
        for c in &corners {
            vertices.push(Vertex {
                position: [c.x, c.y, c.z],
                normal: [n.x, n.y, n.z],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// Build the GRID_SIZE³ instance grid centered on the origin.
fn grid_instances() -> Vec<InstanceData> {
    let n = GRID_SIZE;
    let count = (n * n * n) as usize;
    let mut out = Vec::with_capacity(count);
    let half = (n - 1) as f32 * 0.5;
    let mut i: usize = 0;
    for x in 0..n {
        for y in 0..n {
            for z in 0..n {
                let pos = [
                    (x as f32 - half) * GRID_SPACING,
                    (y as f32 - half) * GRID_SPACING,
                    (z as f32 - half) * GRID_SPACING,
                ];
                // Hue cycles across the grid; gives every cube a different tint.
                let h = i as f32 / count as f32;
                let color = hsv_to_rgb(h, 0.7, 1.0);
                out.push(InstanceData {
                    offset: pos,
                    scale: 1.0,
                    color,
                    _pad: 0.0,
                });
                i += 1;
            }
        }
    }
    out
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match (i as i32).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    #[test]
    fn renders_default_grid() {
        let Some(harness) = TestHarness::try_init(64, 64) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(r#"(type: "instance")"#).unwrap();
        let mut node = InstanceNode::new(&spec, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
