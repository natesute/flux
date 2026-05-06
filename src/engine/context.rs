//! GPU device and queue. Owned by the engine, borrowed by every node.

use anyhow::{Context, Result};

/// Owns the wgpu device and queue. Cloning this is cheap; both fields are
/// reference-counted internally.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Format used for all internal textures. RGBA16Float gives us HDR
    /// headroom for bloom and other physically-based post effects.
    pub fn texture_format(&self) -> wgpu::TextureFormat {
        wgpu::TextureFormat::Rgba16Float
    }
}
