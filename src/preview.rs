//! `flux preview` — live preview window. Drives the engine cook loop at
//! the project's framerate, blits each frame to the surface with tone
//! mapping baked in. Audio playback isn't wired yet (v1 just reads the
//! audio file for features); time wraps so loops loop.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::audio::AudioTrack;
use crate::engine::{Engine, GpuContext};
use crate::project::{Project, ToneMap};

/// Open a window and render `project` in real time. Blocks until the user
/// closes the window. The project's `source_dir` is used by nodes that
/// load sibling files (e.g. `custom_shader`) — set it via
/// `Project::load`, not by constructing a `Project` in memory.
pub fn run(project: &Project, _project_dir: &Path, audio_path: &Path) -> Result<()> {
    let event_loop = EventLoop::new().context("creating event loop")?;
    let mut app = PreviewApp::new(project, audio_path)?;
    event_loop.run_app(&mut app).context("running event loop")?;
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct BlitUniforms {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

struct GraphicsState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    blit_uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

struct PreviewApp {
    project: Project,
    audio: AudioTrack,
    engine: Option<Engine>,
    graphics: Option<GraphicsState>,
    start: Instant,
    audio_duration: f32,
}

impl PreviewApp {
    fn new(project: &Project, audio_path: &Path) -> Result<Self> {
        let audio = AudioTrack::load(audio_path)?;
        let audio_duration = audio.duration_seconds().max(0.001);
        Ok(Self {
            project: project.clone(),
            audio,
            engine: None,
            graphics: None,
            start: Instant::now(),
            audio_duration,
        })
    }
}

impl ApplicationHandler for PreviewApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title(format!(
                "flux — {}×{}",
                self.project.width, self.project.height
            ))
            .with_inner_size(LogicalSize::new(
                self.project.width as f64,
                self.project.height as f64,
            ));
        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // Stand up the engine (and its GPU device) only after we have a
        // window — we build the surface from the same wgpu instance the
        // engine owns so they share a device.
        let engine = match Engine::new(&self.project) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("engine init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let graphics = match build_graphics(&engine.gpu, window.clone()) {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("graphics init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        self.engine = Some(engine);
        self.graphics = Some(graphics);
        self.start = Instant::now();
        self.graphics.as_ref().unwrap().window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        let Some(graphics) = self.graphics.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                graphics.surface_config.width = size.width;
                graphics.surface_config.height = size.height;
                graphics
                    .surface
                    .configure(&engine.gpu.device, &graphics.surface_config);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = render_one(
                    engine,
                    graphics,
                    &self.audio,
                    self.start,
                    self.audio_duration,
                    self.project.tone_map,
                ) {
                    tracing::error!("preview render error: {e}");
                    event_loop.exit();
                    return;
                }
                graphics.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn build_graphics(gpu: &GpuContext, window: Arc<Window>) -> Result<GraphicsState> {
    let surface = gpu
        .instance
        .create_surface(window.clone())
        .context("creating surface")?;

    let caps = surface.get_capabilities(&gpu.adapter);
    // Prefer an sRGB format so the swapchain handles gamma; fall back to
    // the first available if none is sRGB.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);

    let size = window.inner_size();
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &surface_config);

    let blit_bgl = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/blit.wgsl").into()),
        });

    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit layout"),
            bind_group_layouts: &[&blit_bgl],
            push_constant_ranges: &[],
        });

    let blit_pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit pipeline"),
            layout: Some(&pipeline_layout),
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
                    format,
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
        });

    let blit_uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blit uniforms"),
        size: std::mem::size_of::<BlitUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("blit sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    Ok(GraphicsState {
        window,
        surface,
        surface_config,
        blit_pipeline,
        blit_bgl,
        blit_uniform,
        sampler,
    })
}

fn render_one(
    engine: &mut Engine,
    graphics: &GraphicsState,
    audio: &AudioTrack,
    start: Instant,
    audio_duration: f32,
    tone_map: ToneMap,
) -> Result<()> {
    let elapsed = start.elapsed().as_secs_f32();
    // Loop the audio when it ends. `time` is what audio features see; the
    // node frame index keeps counting up so feedback nodes don't reset.
    let time = elapsed % audio_duration;
    let frame_index = (elapsed * engine.fps as f32) as u32;
    let features = audio.features_at(time);

    let cooked = engine.cook_one_frame(time, frame_index, features)?;
    let cooked_view = cooked.create_view(&wgpu::TextureViewDescriptor::default());

    let frame = match graphics.surface.get_current_texture() {
        Ok(f) => f,
        Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
            graphics
                .surface
                .configure(&engine.gpu.device, &graphics.surface_config);
            return Ok(());
        }
        Err(e) => return Err(anyhow::anyhow!("surface acquire failed: {e:?}")),
    };
    let frame_view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let uniforms = BlitUniforms {
        mode: match tone_map {
            ToneMap::Aces => 0,
            ToneMap::Reinhard => 1,
            ToneMap::None => 2,
        },
        ..Default::default()
    };
    engine
        .gpu
        .queue
        .write_buffer(&graphics.blit_uniform, 0, bytemuck::bytes_of(&uniforms));

    let bind_group = engine
        .gpu
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit bg"),
            layout: &graphics.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: graphics.blit_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&cooked_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&graphics.sampler),
                },
            ],
        });

    let mut encoder = engine
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blit"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame_view,
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
        pass.set_pipeline(&graphics.blit_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    engine.gpu.queue.submit(Some(encoder.finish()));
    frame.present();
    Ok(())
}
