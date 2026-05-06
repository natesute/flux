//! `flux preview` — live preview window. Drives the engine cook loop at
//! the project's framerate, blits each frame to the surface with tone
//! mapping baked in.
//!
//! ## Hot reload
//!
//! The preview watches the project file and every file it references
//! (custom shader paths, color-grade LUTs). Every ~100 ms the main loop
//! polls their modification times; on change, the engine's graph is
//! rebuilt in place — same GPU device, same surface, same blit pipeline.
//! That makes "agent edits a file → preview reflects it" a sub-second
//! loop with no IPC and no separate process.
//!
//! Audio playback isn't wired yet (v1 reads the file for features only);
//! wall-clock time wraps so loops loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

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
/// closes the window. `project_path` is the file the project was loaded
/// from; the preview watches it (and any files it references) for changes
/// and hot-reloads the graph when they're edited.
pub fn run(project: &Project, project_path: &Path, audio_path: &Path) -> Result<()> {
    let event_loop = EventLoop::new().context("creating event loop")?;
    let mut app = PreviewApp::new(project, project_path, audio_path)?;
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
    project_path: PathBuf,
    audio: AudioTrack,
    engine: Option<Engine>,
    graphics: Option<GraphicsState>,
    start: Instant,
    audio_duration: f32,

    /// Files whose modification time we poll for hot-reload. Always
    /// includes `project_path`; gains entries for any node that loads
    /// a sibling file (custom_shader, color_grade with a path).
    tracked: Vec<TrackedFile>,
    last_reload_check: Instant,
}

struct TrackedFile {
    path: PathBuf,
    mtime: Option<SystemTime>,
}

impl TrackedFile {
    fn from_path(path: PathBuf) -> Self {
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, mtime }
    }
}

impl PreviewApp {
    fn new(project: &Project, project_path: &Path, audio_path: &Path) -> Result<Self> {
        let audio = AudioTrack::load(audio_path)?;
        let audio_duration = audio.duration_seconds().max(0.001);
        let tracked = collect_tracked_files(project, project_path);
        Ok(Self {
            project: project.clone(),
            project_path: project_path.to_path_buf(),
            audio,
            engine: None,
            graphics: None,
            start: Instant::now(),
            audio_duration,
            tracked,
            last_reload_check: Instant::now(),
        })
    }

    /// If any tracked file's mtime advanced since the last check, attempt
    /// to reload the project and rebuild the graph in place. On failure
    /// the existing engine is left untouched and the error is logged —
    /// half-saved files don't crash the preview.
    fn maybe_reload(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_reload_check) < Duration::from_millis(100) {
            return;
        }
        self.last_reload_check = now;

        let mut changed = false;
        for tf in &mut self.tracked {
            let new_mtime = std::fs::metadata(&tf.path).and_then(|m| m.modified()).ok();
            if new_mtime != tf.mtime {
                tf.mtime = new_mtime;
                changed = true;
            }
        }
        if !changed {
            return;
        }

        let new_project = match Project::load(&self.project_path) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("hot reload: project parse failed, keeping old: {e:#}");
                return;
            }
        };

        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        match engine.rebuild_graph(&new_project) {
            Ok(()) => {
                tracing::info!(
                    "hot reload: rebuilt graph from {}",
                    self.project_path.display()
                );
                // Pick up any new file deps (e.g. user added a custom_shader).
                self.tracked = collect_tracked_files(&new_project, &self.project_path);
                self.project = new_project;
            }
            Err(e) => {
                tracing::warn!("hot reload: graph rebuild failed, keeping old: {e:#}");
            }
        }
    }
}

/// Walk the project's nodes and return the project file plus every sibling
/// file the graph depends on. Currently that's `custom_shader` paths and
/// `color_grade` LUT paths. Adding a new file-loading node? Add it here.
fn collect_tracked_files(project: &Project, project_path: &Path) -> Vec<TrackedFile> {
    let mut files = vec![TrackedFile::from_path(project_path.to_path_buf())];
    for spec in project.nodes.values() {
        let watch_path = match spec.kind.as_str() {
            "custom_shader" | "color_grade" => spec
                .params
                .get("path")
                .and_then(|v| v.as_string())
                .map(|p| project.source_dir.join(p)),
            _ => None,
        };
        if let Some(p) = watch_path {
            files.push(TrackedFile::from_path(p));
        }
    }
    files
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
        if self.engine.is_none() || self.graphics.is_none() {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                let engine = self.engine.as_ref().unwrap();
                let graphics = self.graphics.as_mut().unwrap();
                graphics.surface_config.width = size.width;
                graphics.surface_config.height = size.height;
                graphics
                    .surface
                    .configure(&engine.gpu.device, &graphics.surface_config);
            }
            WindowEvent::RedrawRequested => {
                self.maybe_reload();
                let tone_map = self.project.tone_map;
                let start = self.start;
                let audio_duration = self.audio_duration;
                let engine = self.engine.as_mut().unwrap();
                let graphics = self.graphics.as_mut().unwrap();
                if let Err(e) = render_one(
                    engine,
                    graphics,
                    &self.audio,
                    start,
                    audio_duration,
                    tone_map,
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
