use crate::config::WallrConfig;
use crate::ipc::{IpcCommand, IpcResponse, start_ipc_server};
use crate::renderer::Renderer;
use crate::wallpaper::{SetOptions, WallpaperEngine};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

use raw_window_handle::{
    DisplayHandle, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::WaylandSurface,
    shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        LayerSurfaceConfigure,
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
};

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon already running: {0}")]
    AlreadyRunning(String),
    #[error("failed to start daemon: {0}")]
    StartError(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("IPC error: {0}")]
    Ipc(#[from] crate::ipc::IpcError),
    #[error("Config error: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("Wallpaper error: {0}")]
    Wallpaper(#[from] crate::wallpaper::WallpaperError),
}

pub struct WaylandWindow {
    pub display: *mut std::ffi::c_void,
    pub surface: *mut std::ffi::c_void,
}

unsafe impl Send for WaylandWindow {}
unsafe impl Sync for WaylandWindow {}

impl HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
        let surface = std::ptr::NonNull::new(self.surface)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = WaylandWindowHandle::new(surface);
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Wayland(handle))) }
    }
}

impl HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        let display = std::ptr::NonNull::new(self.display)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = WaylandDisplayHandle::new(display);
        unsafe { Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Wayland(handle))) }
    }
}

struct WaylandState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    surfaces: Vec<LayerSurface>,
    width: u32,
    height: u32,
    scale_factor: i32,
}

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState,];
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.scale_factor = new_factor;
    }
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for WaylandState {
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 > 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 > 0 {
            self.height = configure.new_size.1;
        }
        layer.commit();
    }

    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

delegate_compositor!(WaylandState);
delegate_layer!(WaylandState);
delegate_output!(WaylandState);
delegate_registry!(WaylandState);
delegate_shm!(WaylandState);

struct RenderState {
    renderer: std::sync::Arc<Renderer>,
    surface: &'static wgpu::Surface<'static>,
    /// Serializes transition rendering. The lock is only ever held by the
    /// detached render task, never by the IPC loop, so a stalled present
    /// cannot freeze the daemon.
    render_lock: std::sync::Arc<std::sync::Mutex<()>>,
    /// Bumped on every commit. Live playback checks it each frame and stops
    /// as soon as a new wallpaper supersedes the one it is playing.
    playback_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    current_bind: Option<wgpu::BindGroup>,
    current_tex: Option<wgpu::Texture>,
    width: u32,
    height: u32,
    current_width: u32,
    current_height: u32,
    format: wgpu::TextureFormat,
}

/// Everything the transition render task needs; the daemon state has already
/// been promoted to the new wallpaper before a transition is spawned.
struct CommitData {
    bg_bind: wgpu::BindGroup,
    new_bind: wgpu::BindGroup,
    img_width: u32,
    img_height: u32,
    old_img_width: u32,
    old_img_height: u32,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    /// Animated frames to play live after the transition, when the committed
    /// file is a GIF.
    animated: Option<crate::animated::AnimatedImage>,
    /// Playback generation captured at commit time; live playback stops when
    /// it no longer matches `RenderState::playback_gen`.
    generation: u64,
}

impl RenderState {
    async fn set_wallpaper(
        &mut self,
        path: &std::path::Path,
        effect: &crate::animation::Effect,
        duration_ms: u32,
    ) -> anyhow::Result<()> {
        let commit = self.commit_wallpaper(path)?;
        self.spawn_transition(commit, effect, duration_ms);
        Ok(())
    }

    /// Loads the new wallpaper and atomically promotes it to the current
    /// frame. The outgoing bind group stays alive for the transition, so the
    /// render task can keep drawing from it after this commit returns.
    fn commit_wallpaper(&mut self, path: &std::path::Path) -> anyhow::Result<CommitData> {
        use image::ImageReader;

        // Decode animated frames first (cheap for static images: a six-byte
        // magic sniff); the first frame also serves as the transition's
        // incoming image.
        let animated = crate::animated::AnimatedImage::decode(path)?;
        let new_img = ImageReader::open(path)?.decode()?;
        let (new_tex, new_bind) = self.renderer.load_texture(&new_img)?;
        let img_width = new_img.width();
        let img_height = new_img.height();

        let old_bind = self.current_bind.take();
        let (old_img_width, old_img_height) = if old_bind.is_some() {
            (self.current_width.max(1), self.current_height.max(1))
        } else {
            (img_width, img_height)
        };
        // Keep the last image as the outgoing frame. On the first ever run,
        // using the incoming image for both sides is a clean no-op transition;
        // it avoids a black flash while still allowing the cached wallpaper
        // restored at daemon startup to become the real outgoing frame.
        let bg_bind = old_bind.unwrap_or_else(|| new_bind.clone());

        drop(self.current_tex.take());
        self.current_tex = Some(new_tex);
        self.current_bind = Some(new_bind.clone());
        self.current_width = img_width;
        self.current_height = img_height;

        let generation = self.playback_gen.fetch_add(1, Ordering::SeqCst) + 1;

        Ok(CommitData {
            bg_bind,
            new_bind,
            img_width,
            img_height,
            old_img_width,
            old_img_height,
            format: self.format,
            width: self.width,
            height: self.height,
            animated,
            generation,
        })
    }

    /// Renders the committed transition on a detached blocking task. The IPC
    /// path never waits on GPU presents, so a stalled compositor (monitor
    /// off, suspend) cannot hang the daemon. Transitions are serialized by
    /// the render lock: a later one simply waits until the earlier drains.
    fn spawn_transition(
        &self,
        commit: CommitData,
        effect: &crate::animation::Effect,
        duration_ms: u32,
    ) {
        let renderer = self.renderer.clone();
        let surface: &'static wgpu::Surface<'static> = self.surface;
        let render_lock = self.render_lock.clone();
        let playback_gen = self.playback_gen.clone();
        let effect = effect.clone();
        drop(tokio::task::spawn_blocking(move || {
            render_transition(
                renderer,
                surface,
                render_lock,
                playback_gen,
                commit,
                effect,
                duration_ms,
            );
        }));
    }
}

/// Presents one frame per vsync until the wall-clock duration elapses. With
/// PresentMode::Fifo, `get_current_texture` blocks until the previous frame
/// is presented, so this loop is paced to the monitor refresh rate, and the
/// transition lasts exactly `duration_ms` on any refresh rate — frame-count
/// pacing would run too fast on high-refresh panels and too slow when the
/// present rate is low. If the compositor stops presenting, the loop can park
/// inside a present; that is fine here because the task is detached.
fn render_transition(
    renderer: std::sync::Arc<Renderer>,
    surface: &'static wgpu::Surface<'static>,
    render_lock: std::sync::Arc<std::sync::Mutex<()>>,
    playback_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    commit: CommitData,
    effect: crate::animation::Effect,
    duration_ms: u32,
) {
    let _guard = render_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let duration = std::time::Duration::from_millis(u64::from(duration_ms.max(1)));
    let start = std::time::Instant::now();
    loop {
        let progress = start.elapsed().as_secs_f32() / duration.as_secs_f32();
        let uniforms = crate::animation::compute_effect_uniforms(&effect, progress.clamp(0.0, 1.0));
        let status = renderer.render_frame(crate::renderer::FrameRequest {
            surface,
            format: commit.format,
            bg_bind: &commit.bg_bind,
            new_bind: &commit.new_bind,
            effect: &uniforms,
            width: commit.width,
            height: commit.height,
            img_width: commit.img_width,
            img_height: commit.img_height,
            old_img_width: commit.old_img_width,
            old_img_height: commit.old_img_height,
        });
        let status = match status {
            Ok(status) => status,
            Err(err) => {
                eprintln!("wallr: transition render failed: {err}");
                break;
            }
        };
        if progress >= 1.0 || status == crate::renderer::FrameStatus::TimedOut {
            break;
        }
    }

    // The transition ended; if the committed wallpaper is an animated GIF and
    // nothing superseded it while we rendered, keep the render lock and play
    // the frames live until the next commit bumps the generation.
    if let Some(animated) = &commit.animated
        && playback_gen.load(Ordering::SeqCst) == commit.generation
    {
        play_live(&renderer, surface, &commit, animated, &playback_gen);
    }
}

/// Presents live wallpaper frames until the next commit. One frame is
/// presented per vsync (Fifo), and texture uploads only happen when the
/// playhead crosses into a new GIF frame, so playback is smooth without
/// burning GPU bandwidth on unchanged pixels.
fn play_live(
    renderer: &Renderer,
    surface: &'static wgpu::Surface<'static>,
    commit: &CommitData,
    animated: &crate::animated::AnimatedImage,
    playback_gen: &std::sync::atomic::AtomicU64,
) {
    let (texture, bind) = renderer.create_texture(animated.width, animated.height);
    renderer.update_texture(
        &texture,
        animated.first_frame(),
        animated.width,
        animated.height,
    );

    let mut shown = usize::MAX;
    let start = std::time::Instant::now();
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
    loop {
        if playback_gen.load(Ordering::SeqCst) != commit.generation {
            return;
        }
        let index = animated.frame_index_at(start.elapsed());
        if index != shown {
            renderer.update_texture(
                &texture,
                animated.frame_at(index),
                animated.width,
                animated.height,
            );
            shown = index;
        }
        let uniforms = crate::animation::compute_effect_uniforms(&static_effect, 1.0);
        let status = renderer.render_frame(crate::renderer::FrameRequest {
            surface,
            format: commit.format,
            bg_bind: &bind,
            new_bind: &bind,
            effect: &uniforms,
            width: commit.width,
            height: commit.height,
            img_width: animated.width,
            img_height: animated.height,
            old_img_width: animated.width,
            old_img_height: animated.height,
        });
        match status {
            Ok(crate::renderer::FrameStatus::Presented) => {}
            // A stalled present parks inside the acquire; a Timeout or error
            // means the surface is unusable, so give up and let the next
            // transition take over.
            _ => return,
        }
    }
}

pub struct Daemon {
    config: WallrConfig,
    paused: Arc<AtomicBool>,
    engine: Arc<Mutex<WallpaperEngine>>,
}

impl Daemon {
    pub fn new(config: WallrConfig) -> Result<Self, DaemonError> {
        let engine = WallpaperEngine::new(config.clone())?;
        Ok(Self {
            config,
            paused: Arc::new(AtomicBool::new(false)),
            engine: Arc::new(Mutex::new(engine)),
        })
    }

    pub async fn start(self) -> Result<(), DaemonError> {
        let socket_path = crate::config::expand_path(&self.config.daemon.socket);
        if socket_path.exists() {
            if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                return Err(DaemonError::AlreadyRunning(
                    socket_path.to_string_lossy().to_string(),
                ));
            }
            let _ = std::fs::remove_file(&socket_path);
        }

        let renderer = Renderer::new()
            .await
            .map_err(|e| DaemonError::StartError(format!("GPU init failed: {e}")))?;

        let conn = Connection::connect_to_env()
            .map_err(|e| DaemonError::StartError(format!("Failed to connect to Wayland: {e:?}")))?;
        let backend = conn.backend();
        let display_ptr = backend.display_ptr() as *mut std::ffi::c_void;

        let (globals, mut event_queue) = registry_queue_init(&conn)
            .map_err(|e| DaemonError::StartError(format!("registry_queue_init failed: {e:?}")))?;
        let qh = event_queue.handle();

        let compositor_state = CompositorState::bind(&globals, &qh)
            .map_err(|e| DaemonError::StartError(format!("compositor bind failed: {e:?}")))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|e| DaemonError::StartError(format!("layer_shell bind failed: {e:?}")))?;
        let shm = Shm::bind(&globals, &qh)
            .map_err(|e| DaemonError::StartError(format!("shm bind failed: {e:?}")))?;

        let mut wayland_state = WaylandState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            compositor_state,
            shm,
            surfaces: Vec::new(),
            width: 1920,
            height: 1080,
            scale_factor: 1,
        };

        let wl_surface = wayland_state.compositor_state.create_surface(&qh);
        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            wl_surface,
            Layer::Background,
            Some("wallr"),
            None,
        );
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer_surface.commit();

        event_queue
            .roundtrip(&mut wayland_state)
            .map_err(|e| DaemonError::StartError(format!("roundtrip failed: {e:?}")))?;
        event_queue
            .roundtrip(&mut wayland_state)
            .map_err(|e| DaemonError::StartError(format!("roundtrip2 failed: {e:?}")))?;

        let scale_factor = if wayland_state.scale_factor > 0 {
            wayland_state.scale_factor
        } else {
            1
        };
        layer_surface.wl_surface().set_buffer_scale(scale_factor);

        let width = wayland_state.width * scale_factor as u32;
        let height = wayland_state.height * scale_factor as u32;

        let raw_surface = layer_surface.wl_surface().id().as_ptr() as *mut std::ffi::c_void;
        wayland_state.surfaces.push(layer_surface);

        let window_handle = WaylandWindow {
            display: display_ptr,
            surface: raw_surface,
        };

        let wgpu_surface = renderer
            .instance
            .create_surface(&window_handle)
            .map_err(|e| DaemonError::StartError(format!("wgpu surface creation failed: {e:?}")))?;

        let adapter = renderer
            .instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&wgpu_surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
            })
            .await;
        let surf_format = adapter
            .as_ref()
            .map(|a| {
                let caps = wgpu_surface.get_capabilities(a);
                caps.formats
                    .into_iter()
                    .next()
                    .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb)
            })
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);

        let surf_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surf_format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        wgpu_surface.configure(&renderer.device, &surf_config);

        // SAFETY: We transmute the surface lifetime to 'static so it can be moved
        // into the shared Arc. The surface is tied to window_handle / wayland_state
        // both of which live as long as the process.
        let wgpu_surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };
        // The daemon lives for the whole process, so leaking one surface is fine
        // and gives every detached transition task a stable reference to present
        // to without holding the RenderState lock during the blocking acquire.
        let surface: &'static wgpu::Surface<'static> = Box::leak(Box::new(wgpu_surface));

        let render_state = Arc::new(Mutex::new(RenderState {
            renderer: std::sync::Arc::new(renderer),
            surface,
            render_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            playback_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            current_bind: None,
            current_tex: None,
            width,
            height,
            current_width: 0,
            current_height: 0,
            format: surf_format,
        }));

        {
            let state_path = dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("wallr/last_wallpaper");
            if let Ok(path_str) = std::fs::read_to_string(&state_path) {
                let p = std::path::Path::new(path_str.trim());
                if p.exists() {
                    let mut rs = render_state.lock().await;
                    let effect =
                        crate::animation::Effect::Fade(crate::animation::FadeParams::default());
                    let _ = rs.set_wallpaper(p, &effect, 0).await;
                }
            }
        }

        let paused_clone = self.paused.clone();
        let engine_clone = self.engine.clone();
        let render_state_clone = render_state.clone();

        start_ipc_server(&socket_path, move |cmd| {
            let paused = paused_clone.clone();
            let engine = engine_clone.clone();
            let rs = render_state_clone.clone();
            async move {
                match cmd {
                    IpcCommand::Pause => {
                        paused.store(true, Ordering::SeqCst);
                        IpcResponse {
                            success: true,
                            message: Some("Paused".into()),
                        }
                    }
                    IpcCommand::Resume => {
                        paused.store(false, Ordering::SeqCst);
                        IpcResponse {
                            success: true,
                            message: Some("Resumed".into()),
                        }
                    }
                    IpcCommand::Reload => {
                        let lock = engine.lock().await;
                        match lock.reload() {
                            Ok(_) => IpcResponse {
                                success: true,
                                message: Some("Reloaded".into()),
                            },
                            Err(e) => IpcResponse {
                                success: false,
                                message: Some(e.to_string()),
                            },
                        }
                    }
                    IpcCommand::Preview {
                        path,
                        effect,
                        duration_ms,
                        no_theme,
                        theme_override,
                        monitor,
                    } => {
                        if paused.load(Ordering::SeqCst) {
                            return IpcResponse {
                                success: false,
                                message: Some("Daemon is paused".into()),
                            };
                        }
                        let p = std::path::PathBuf::from(&path);
                        if !p.exists() {
                            return IpcResponse {
                                success: false,
                                message: Some(format!("File not found: {}", path)),
                            };
                        }

                        let state_path = dirs::cache_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                            .join("wallr/last_wallpaper");
                        if let Some(parent) = state_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&state_path, &path);

                        let effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        let duration = duration_ms.unwrap_or(2000);

                        let rs_clone = rs.clone();
                        let p_clone = p.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let rt = tokio::runtime::Handle::current();
                            rt.block_on(async {
                                let mut lock = rs_clone.lock().await;
                                lock.set_wallpaper(&p_clone, &effect, duration).await
                            })
                        })
                        .await;

                        match result {
                            Ok(Ok(())) => {
                                let opts = SetOptions {
                                    no_theme,
                                    theme_provider: theme_override,
                                    monitor,
                                };
                                let mut eng = engine.lock().await;
                                match eng.set_wallpaper(&p, &opts).await {
                                    Ok(()) => IpcResponse {
                                        success: true,
                                        message: None,
                                    },
                                    Err(e) => IpcResponse {
                                        success: true,
                                        message: Some(format!(
                                            "Wallpaper set, but hooks/theme failed: {e}"
                                        )),
                                    },
                                }
                            }
                            Ok(Err(e)) => IpcResponse {
                                success: false,
                                message: Some(format!("Render failed: {}", e)),
                            },
                            Err(e) => IpcResponse {
                                success: false,
                                message: Some(format!("Task spawn failed: {}", e)),
                            },
                        }
                    }
                    IpcCommand::Stop => {
                        tokio::spawn(async {
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            std::process::exit(0);
                        });
                        IpcResponse {
                            success: true,
                            message: Some("Stopping".into()),
                        }
                    }
                    IpcCommand::Status => {
                        let state = if paused.load(Ordering::SeqCst) {
                            "paused"
                        } else {
                            "running"
                        };
                        IpcResponse {
                            success: true,
                            message: Some(format!("wallr daemon {}", state)),
                        }
                    }
                }
            }
        })
        .await?;

        // Start file watcher if configured
        if self.config.watch.enabled
            && let Some(ref watch_dir) = self.config.watch.dir
        {
            let watch_path = crate::config::expand_path(watch_dir);
            self.start_watcher(watch_path, render_state.clone()).await?;
        }

        tokio::task::spawn_blocking(move || {
            loop {
                if let Err(e) = event_queue.blocking_dispatch(&mut wayland_state) {
                    eprintln!("Wayland dispatch error: {e:?}");
                    break;
                }
            }
        });

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    }

    async fn start_watcher(
        &self,
        dir: PathBuf,
        render_state: Arc<Mutex<RenderState>>,
    ) -> Result<(), DaemonError> {
        let engine = self.engine.clone();
        let paused = self.paused.clone();
        let debounce = crate::config::parse_duration(&self.config.watch.debounce)
            .unwrap_or(std::time::Duration::from_millis(500));

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res
                && let EventKind::Create(_) = event.kind
            {
                for path in event.paths {
                    let _ = tx.blocking_send(path);
                }
            }
        })
        .map_err(|e| DaemonError::StartError(e.to_string()))?;

        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|e| DaemonError::StartError(e.to_string()))?;

        tokio::spawn(async move {
            let _watcher = watcher;
            let mut last: Option<(PathBuf, std::time::Instant)> = None;

            while let Some(path) = rx.recv().await {
                if paused.load(Ordering::SeqCst) {
                    continue;
                }
                if let Some((ref lp, ref lt)) = last
                    && lp == &path
                    && lt.elapsed() < debounce
                {
                    continue;
                }
                let ext = path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if !["jpg", "jpeg", "png", "gif", "webp"].contains(&ext.as_str()) {
                    continue;
                }
                last = Some((path.clone(), std::time::Instant::now()));

                let rs = render_state.clone();
                let eng = engine.clone();
                let p = path.clone();
                tokio::spawn(async move {
                    let mut lock = rs.lock().await;
                    let effect =
                        crate::animation::Effect::Fade(crate::animation::FadeParams::default());
                    let _ = lock.set_wallpaper(&p, &effect, 600).await;
                    drop(lock);
                    let opts = SetOptions {
                        no_theme: false,
                        theme_provider: None,
                        monitor: None,
                    };
                    let mut elock = eng.lock().await;
                    let _ = elock.set_wallpaper(&p, &opts).await;
                });
            }
        });

        Ok(())
    }
}
