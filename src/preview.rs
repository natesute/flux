//! `flux preview` — live preview window with an egui inspector.
//!
//! Layout: a fixed-width inspector panel on the left, the rendered preview
//! filling the rest of the window. Both halves share the same wgpu device.
//!
//! ## Hot reload
//!
//! Two flows write to the project's `.ron`:
//!
//! 1. **External edits** (you in your editor, an AI agent, etc.). The
//!    preview polls the project file and every file it references
//!    (custom shader paths, color-grade LUTs) every ~100 ms; on change,
//!    the engine's graph is rebuilt in place — same GPU device, same
//!    surface, same blit pipeline. Errors keep the old graph running so
//!    half-saved edits don't crash the window.
//!
//! 2. **Inspector edits** (you dragging a slider in the panel). The
//!    inspector mutates an in-memory copy of the project; ~200 ms after
//!    the last change, that copy is serialized back to the same `.ron`
//!    file. The watcher fires, the rebuild runs, and the loop closes.
//!    Both flows use the same code path.
//!
//! Audio playback isn't wired (v1 reads the file for features only); a
//! wall clock advances time and wraps so loops loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu::SurfaceConfiguration;
use egui_wgpu::ScreenDescriptor;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::audio::AudioTrack;
use crate::audio_player::AudioPlayer;
use crate::engine::{Engine, FrameContext, GpuContext};
use crate::inspector::{self, InspectorEnv, UiAction};
use crate::project::Project;

/// Width of the inspector panel in logical pixels.
const PANEL_WIDTH: u32 = 320;

/// How long to wait after the last inspector edit before saving.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(200);

/// Open a window and render `project` in real time. Blocks until the user
/// closes the window. `project_path` is the file the project was loaded
/// from; the preview watches it (and any files it references) for changes
/// and hot-reloads the graph when they're edited, *and* writes inspector
/// edits back to it.
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

/// All wgpu/winit/egui resources tied to the live window. Lives only
/// after `resumed`; reset to `None` if the user closes the window.
struct GraphicsState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,

    /// Tone-mapped 8-bit copy of the engine's HDR output, displayed by
    /// the inspector's central panel as an `egui::Image`. Kept alive
    /// alongside the view we render into and the egui-side texture id.
    #[allow(dead_code)]
    preview_target: wgpu::Texture,
    preview_view: wgpu::TextureView,
    preview_tex_id: egui::TextureId,

    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    blit_uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,

    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct PreviewApp {
    project: Project,
    project_path: PathBuf,
    audio_path: PathBuf,
    audio: AudioTrack,
    /// Owned cpal output stream; dropped when the app exits. `None` if
    /// the host has no default output device.
    _audio_player: Option<AudioPlayer>,
    engine: Option<Engine>,
    graphics: Option<GraphicsState>,
    start: Instant,
    audio_duration: f32,

    /// Files whose modification time we poll for hot-reload.
    tracked: Vec<TrackedFile>,
    last_reload_check: Instant,

    /// `Some(t)` when the project has been edited via the inspector but
    /// not yet written to disk. We wait `SAVE_DEBOUNCE` of quiet before
    /// committing so a slider drag isn't a 60-write storm.
    pending_save: Option<Instant>,

    /// State that persists across inspector frames (record duration,
    /// "rendering…" indicator, etc.).
    inspector_env: InspectorEnv,

    /// Background `flux render` child kicked off by the record button.
    /// Polled each frame; on success we `open` the resulting mp4.
    render_child: Option<RenderChild>,
}

struct RenderChild {
    child: std::process::Child,
    out_path: PathBuf,
    started: Instant,
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
        // Best-effort audio playback. Failure (no device, unsupported
        // format, etc.) shouldn't kill the preview — just log and run silent.
        let _audio_player = match AudioPlayer::try_new(&audio) {
            Ok(player) => player,
            Err(e) => {
                tracing::warn!("audio playback disabled: {e:#}");
                None
            }
        };
        // Default record length to the audio clip's length (clamped to a
        // sensible range), so a typical "render this" click captures the
        // whole loop.
        let inspector_env = InspectorEnv {
            record_seconds: audio_duration.clamp(1.0, 60.0),
            ..InspectorEnv::default()
        };
        Ok(Self {
            project: project.clone(),
            project_path: project_path.to_path_buf(),
            audio_path: audio_path.to_path_buf(),
            audio,
            _audio_player,
            engine: None,
            graphics: None,
            start: Instant::now(),
            audio_duration,
            tracked,
            last_reload_check: Instant::now(),
            pending_save: None,
            inspector_env,
            render_child: None,
        })
    }

    /// Save the engine's currently-cooked output as a PNG on the Desktop.
    /// Uses the same readback + tone-map path as offline rendering.
    fn save_screenshot(&self) -> Result<PathBuf> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("engine not ready"))?;
        let frame_ctx = FrameContext {
            gpu: &engine.gpu,
            width: engine.width,
            height: engine.height,
            frame_index: 0,
            time: 0.0,
            audio: Default::default(),
        };
        let pixels = engine
            .graph
            .read_output_pixels(&frame_ctx, engine.tone_map)?;
        let img = image::RgbaImage::from_raw(engine.width, engine.height, pixels)
            .ok_or_else(|| anyhow::anyhow!("readback produced unexpected pixel count"))?;
        let path = desktop_path(&format!("flux-{}.png", unix_timestamp()));
        img.save(&path)
            .with_context(|| format!("writing screenshot to {}", path.display()))?;
        let _ = std::process::Command::new("open").arg(&path).spawn();
        Ok(path)
    }

    /// Spawn `flux render` as a child process to record `seconds` of the
    /// current project to mp4. Stored as `render_child` and polled each
    /// frame; the result is opened on completion.
    fn start_record(&mut self, seconds: f32) -> Result<()> {
        if self.render_child.is_some() {
            return Ok(());
        }
        let exe = std::env::current_exe().context("locating flux binary")?;
        let out_path = desktop_path(&format!("flux-{}.mp4", unix_timestamp()));
        let child = std::process::Command::new(exe)
            .arg("render")
            .arg(&self.project_path)
            .arg("--audio")
            .arg(&self.audio_path)
            .arg("--out")
            .arg(&out_path)
            .arg("--duration")
            .arg(seconds.to_string())
            .spawn()
            .context("spawning flux render")?;
        self.inspector_env.recording_since = Some(Instant::now());
        self.render_child = Some(RenderChild {
            child,
            out_path,
            started: Instant::now(),
        });
        Ok(())
    }

    /// Non-blocking: if a record child is in flight, check whether it has
    /// finished and open the resulting mp4.
    fn poll_record(&mut self) {
        let Some(rc) = self.render_child.as_mut() else {
            return;
        };
        match rc.child.try_wait() {
            Ok(Some(status)) => {
                let elapsed = rc.started.elapsed().as_secs_f32();
                if status.success() {
                    tracing::info!("rendered {} in {:.1}s", rc.out_path.display(), elapsed);
                    let _ = std::process::Command::new("open").arg(&rc.out_path).spawn();
                } else {
                    tracing::warn!("render child exited with {status}");
                }
                self.render_child = None;
                self.inspector_env.recording_since = None;
            }
            Ok(None) => {} // still running
            Err(e) => {
                tracing::warn!("render child poll error: {e}");
                self.render_child = None;
                self.inspector_env.recording_since = None;
            }
        }
    }

    /// If any tracked file's mtime advanced since the last check, attempt
    /// to reload the project and rebuild the graph in place. On failure
    /// the existing engine is left untouched and the error is logged.
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
                self.tracked = collect_tracked_files(&new_project, &self.project_path);
                self.project = new_project;
            }
            Err(e) => {
                tracing::warn!("hot reload: graph rebuild failed, keeping old: {e:#}");
            }
        }
    }

    /// Persist the current in-memory project back to disk if the debounce
    /// window has elapsed. Updates the tracked mtime so the watcher
    /// doesn't double-fire on our own write.
    fn maybe_save(&mut self) {
        let Some(t) = self.pending_save else {
            return;
        };
        if t.elapsed() < SAVE_DEBOUNCE {
            return;
        }
        self.pending_save = None;

        // Pretty-print so the file stays diff-friendly. ron's pretty
        // printer collapses single-line tuples; that's fine.
        let pretty = ron::ser::PrettyConfig::new()
            .struct_names(false)
            .indentor("    ".to_string());
        let serialized = match ron::ser::to_string_pretty(&self.project, pretty) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("inspector save: serialize failed: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&self.project_path, &serialized) {
            tracing::warn!("inspector save: write failed: {e}");
            return;
        }
        // Refresh the mtime we have on file so maybe_reload's diff doesn't
        // immediately re-load our own write (it would just be a no-op,
        // but it would log a spurious "rebuilt graph" message).
        if let Some(tf) = self
            .tracked
            .iter_mut()
            .find(|t| t.path == self.project_path)
        {
            tf.mtime = std::fs::metadata(&self.project_path)
                .and_then(|m| m.modified())
                .ok();
        }
    }
}

/// Walk the project's nodes and return the project file plus every sibling
/// file the graph depends on.
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

        let total_w = self.project.width + PANEL_WIDTH;
        let window_attrs = Window::default_attributes()
            .with_title(format!(
                "flux — {}×{}",
                self.project.width, self.project.height
            ))
            .with_inner_size(LogicalSize::new(total_w as f64, self.project.height as f64));
        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let engine = match Engine::new(&self.project) {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("engine init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let graphics = match build_graphics(&engine.gpu, window.clone(), &self.project) {
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

        // Let egui peek at the event first; if it consumed it (e.g. a
        // click on a button), we don't propagate.
        let consumed = {
            let g = self.graphics.as_mut().unwrap();
            let resp = g.egui_state.on_window_event(&g.window, &event);
            if resp.repaint {
                g.window.request_redraw();
            }
            resp.consumed
        };

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
                if let Err(e) = self.render_frame() {
                    tracing::error!("preview render error: {e}");
                    event_loop.exit();
                    return;
                }
                self.maybe_save();
                let g = self.graphics.as_ref().unwrap();
                g.window.request_redraw();
            }
            _ if consumed => {}
            _ => {}
        }
    }
}

impl PreviewApp {
    fn render_frame(&mut self) -> Result<()> {
        // Pull what we need out of self before grabbing &mut borrows.
        let elapsed = self.start.elapsed().as_secs_f32();
        let time = elapsed % self.audio_duration;
        let features = self.audio.features_at(time);
        let tone_map = self.project.tone_map;

        let engine = self.engine.as_mut().unwrap();
        let frame_index = (elapsed * engine.fps as f32) as u32;

        // 1. Cook the engine. Borrow ends inline so we don't conflict
        //    with the egui pass below.
        let cooked_view = {
            let cooked = engine.cook_one_frame(time, frame_index, features)?;
            cooked.create_view(&wgpu::TextureViewDescriptor::default())
        };

        // 2. Build the egui frame around the inspector. Slider edits land
        //    in self.project; if anything changed, mark dirty.
        let graphics = self.graphics.as_mut().unwrap();
        let raw_input = graphics.egui_state.take_egui_input(&graphics.window);
        let preview_tex_id = graphics.preview_tex_id;
        let preview_w = engine.width as f32;
        let preview_h = engine.height as f32;

        let inspector_out = std::cell::RefCell::new(inspector::InspectorOutput::default());
        let full_output = graphics.egui_ctx.run(raw_input, |ctx| {
            *inspector_out.borrow_mut() =
                inspector::ui(ctx, &mut self.project, &mut self.inspector_env);
            egui::CentralPanel::default().show(ctx, |ui| {
                let avail = ui.available_size();
                let aspect = preview_w / preview_h;
                let (w, h) = if avail.x / avail.y > aspect {
                    (avail.y * aspect, avail.y)
                } else {
                    (avail.x, avail.x / aspect)
                };
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Image::new((preview_tex_id, egui::vec2(w, h))));
                });
            });
        });
        let inspector_out = inspector_out.into_inner();
        if inspector_out.changed {
            // Apply the edit to the running engine immediately so the next
            // frame reflects it. If the rebuild fails (e.g. you added a
            // node whose required inputs aren't wired yet, or a custom
            // shader path doesn't resolve), keep both the old graph
            // *and* the old on-disk file — saving a broken project would
            // crash the next launch.
            match engine.rebuild_graph(&self.project) {
                Ok(()) => {
                    self.pending_save = Some(Instant::now());
                }
                Err(e) => {
                    tracing::warn!("inspector edit: rebuild failed (not saved): {e:#}");
                }
            }
        }
        // Stash UI actions to dispatch after rendering this frame, so the
        // surface present isn't blocked by a child-process spawn or PNG
        // readback (those would visibly stall the window for a frame or
        // two).
        let pending_actions = inspector_out.actions;

        graphics
            .egui_state
            .handle_platform_output(&graphics.window, full_output.platform_output);

        let pixels_per_point = graphics.egui_ctx.pixels_per_point();
        let paint_jobs = graphics
            .egui_ctx
            .tessellate(full_output.shapes, pixels_per_point);
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [
                graphics.surface_config.width,
                graphics.surface_config.height,
            ],
            pixels_per_point,
        };

        // 3. Acquire the surface texture.
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

        // 4. Update egui's textures (atlas + any newly-registered ones).
        for (id, image_delta) in &full_output.textures_delta.set {
            graphics.egui_renderer.update_texture(
                &engine.gpu.device,
                &engine.gpu.queue,
                *id,
                image_delta,
            );
        }

        let blit_uniforms = BlitUniforms {
            mode: tone_map_index(tone_map),
            ..Default::default()
        };
        engine.gpu.queue.write_buffer(
            &graphics.blit_uniform,
            0,
            bytemuck::bytes_of(&blit_uniforms),
        );

        let blit_bg = engine
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

        let mut encoder =
            engine
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("preview encoder"),
                });

        // 5. Tone-map the engine's HDR output into the 8-bit preview_target
        //    that egui samples from.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit→preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &graphics.preview_view,
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
            pass.set_bind_group(0, &blit_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        graphics.egui_renderer.update_buffers(
            &engine.gpu.device,
            &engine.gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        // 6. egui paints the whole window: panel on the left, image of
        //    preview_target in the center.
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.05,
                                g: 0.05,
                                b: 0.06,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            graphics
                .egui_renderer
                .render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        engine.gpu.queue.submit(Some(encoder.finish()));
        frame.present();

        // 7. Free egui textures the frame retired.
        for id in &full_output.textures_delta.free {
            graphics.egui_renderer.free_texture(id);
        }

        // 8. Now that the frame's on screen, run any queued button
        //    actions. Doing this *after* present means a screenshot
        //    readback or a child-process spawn doesn't stall the frame
        //    the user just clicked the button on.
        for action in pending_actions {
            match action {
                UiAction::Screenshot => match self.save_screenshot() {
                    Ok(path) => tracing::info!("screenshot saved to {}", path.display()),
                    Err(e) => tracing::warn!("screenshot failed: {e:#}"),
                },
                UiAction::StartRecord { seconds } => {
                    if let Err(e) = self.start_record(seconds) {
                        tracing::warn!("record failed to start: {e:#}");
                    }
                }
            }
        }
        self.poll_record();

        Ok(())
    }
}

/// Resolve a filename inside the user's Desktop folder. Falls back to
/// the current directory if `$HOME` isn't set (shouldn't happen on a
/// real desktop session).
fn desktop_path(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.map(|h| h.join("Desktop").join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Unix epoch seconds, formatted compactly for filenames.
fn unix_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn tone_map_index(t: crate::project::ToneMap) -> u32 {
    match t {
        crate::project::ToneMap::Aces => 0,
        crate::project::ToneMap::Reinhard => 1,
        crate::project::ToneMap::None => 2,
    }
}

fn build_graphics(
    gpu: &GpuContext,
    window: Arc<Window>,
    project: &Project,
) -> Result<GraphicsState> {
    let surface = gpu
        .instance
        .create_surface(window.clone())
        .context("creating surface")?;

    let caps = surface.get_capabilities(&gpu.adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);

    let size = window.inner_size();
    let surface_config = SurfaceConfiguration {
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

    // Intermediate target the blit pass writes to. egui samples it via
    // an Image widget; same format as the surface so one pipeline covers
    // both targets if we ever want to render directly to the surface
    // again.
    let preview_target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview target"),
        size: wgpu::Extent3d {
            width: project.width,
            height: project.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let preview_view = preview_target.create_view(&wgpu::TextureViewDescriptor::default());

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

    let egui_ctx = egui::Context::default();
    let egui_state = egui_winit::State::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        &*window,
        Some(window.scale_factor() as f32),
        None,
        None,
    );
    let mut egui_renderer = egui_wgpu::Renderer::new(&gpu.device, format, None, 1, false);
    let preview_tex_id =
        egui_renderer.register_native_texture(&gpu.device, &preview_view, wgpu::FilterMode::Linear);

    Ok(GraphicsState {
        window,
        surface,
        surface_config,
        preview_target,
        preview_view,
        preview_tex_id,
        blit_pipeline,
        blit_bgl,
        blit_uniform,
        sampler,
        egui_ctx,
        egui_state,
        egui_renderer,
    })
}
