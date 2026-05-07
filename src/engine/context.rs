//! GPU device and queue. Owned by the engine, borrowed by every node.

use anyhow::{Context, Result};

/// Owns the wgpu device and queue. Cloning this is cheap; both fields are
/// reference-counted internally.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// 1×1 black texture passed to nodes whose input slot isn't wired
    /// to anything (the user is mid-edit, or just deleted the upstream).
    /// Lets every node always render *something* — even if it's black —
    /// instead of refusing to instantiate and freezing the preview on
    /// a stale graph. See `Graph::cook_frame` for how it's plumbed in.
    pub missing_input: wgpu::Texture,
}

impl GpuContext {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .context("no compatible GPU adapter found")?;

        let info = adapter.get_info();
        tracing::info!(
            "GPU: {} ({:?}, {:?})",
            info.name,
            info.device_type,
            info.backend
        );

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("flux device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .context("requesting GPU device")?;

        // 1×1 black texture used as a fallback when a node's input slot
        // is empty (mid-edit, just deleted, etc.). Format matches the
        // engine's standard so it's a drop-in replacement for any
        // missing input.
        let missing_input = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("missing-input fallback"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Four f16 zeroes = transparent black.
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &missing_input,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8; 8],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            missing_input,
        })
    }

    /// Format used for all internal textures. RGBA16Float gives us HDR
    /// headroom for bloom and other physically-based post effects.
    pub fn texture_format(&self) -> wgpu::TextureFormat {
        wgpu::TextureFormat::Rgba16Float
    }
}
