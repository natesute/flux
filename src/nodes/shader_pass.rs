//! Helpers shared across shader-driven nodes.
//!
//! Every fullscreen shader node has the same shape: compile a shader,
//! create a uniform buffer, optionally bind some input textures + a sampler,
//! create a render pipeline that targets `Rgba16Float`, and dispatch a
//! 3-vertex fullscreen triangle each frame. This module factors that out.

use crate::engine::GpuContext;

/// Build a render pipeline for a fullscreen pass that:
/// - Has a single bind group at @group(0)
/// - Renders into the engine's standard `Rgba16Float` format
/// - Uses entry points `vs_main` and `fs_main`
///
/// The shader module is consumed during pipeline construction; nothing
/// else needs it, so it isn't returned.
pub fn build_fullscreen_pipeline(
    gpu: &GpuContext,
    label: &str,
    shader_source: &str,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{label} layout")),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("{label} pipeline")),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.texture_format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        })
}

/// Standard sampler used for input textures: linear filtering, clamp.
/// Most nodes want this; for nodes that want repeating patterns, build
/// your own with `AddressMode::Repeat`.
pub fn linear_clamp_sampler(gpu: &GpuContext) -> wgpu::Sampler {
    gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("linear clamp"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    })
}

/// Bind group layout entry for a uniform buffer at the given binding.
pub fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Bind group layout entry for a sampled 2D texture at the given binding.
pub fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Bind group layout entry for a sampler at the given binding.
pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// Run a fullscreen render pass that writes into `target` using the given
/// pipeline and bind group. Submits its own command buffer.
pub fn run_fullscreen_pass(
    gpu: &GpuContext,
    label: &str,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::Texture,
) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.queue.submit(Some(encoder.finish()));
}
