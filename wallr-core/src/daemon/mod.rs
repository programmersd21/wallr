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
    Connection, Dispatch, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_compositor, wl_output, wl_surface},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::{self, WpViewport},
    wp_viewporter::{self, WpViewporter},
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

#[derive(Clone)]
struct OutputInfo {
    name: String,
    width: u32,
    height: u32,
    scale_factor: i32,
    wl_output: wl_output::WlOutput,
}

#[derive(Clone)]
struct OutputLifecycle {
    name: String,
    render_state: std::sync::Arc<tokio::sync::Mutex<RenderState>>,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct WaylandState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    outputs: std::collections::HashMap<u32, OutputInfo>,
    surfaces: Vec<(u32, LayerSurface)>,
    viewporter: Option<WpViewporter>,
    viewports: std::collections::HashMap<u32, WpViewport>,
    output_lifecycles: std::collections::HashMap<u32, OutputLifecycle>,
    pending_restores: std::collections::HashSet<u32>,
    /// Layer shell protocol object for creating background surfaces.
    layer_shell: LayerShell,
    /// Compositor protocol object for creating input regions.
    compositor: wl_compositor::WlCompositor,
    /// Shared daemon context for hotplug: creates and destroys render states
    /// when outputs appear or disappear.
    hotplug: Option<DaemonHotplug>,
}

/// Wrapper around `*mut c_void` that implements `Send`. The pointer is a
/// Wayland display pointer that lives for the entire process lifetime.
struct SendDisplayPtr(*mut std::ffi::c_void);
unsafe impl Send for SendDisplayPtr {}

/// Shared context for hotplug operations. Stored in `WaylandState` so the
/// output callbacks can create/destroy render states without needing access
/// to the full `Daemon` state.
struct DaemonHotplug {
    renderer: std::sync::Arc<Renderer>,
    config: crate::config::WallrConfig,
    display_ptr: SendDisplayPtr,
    /// Shared render-state map. Protected by `tokio::sync::Mutex` so the IPC
    /// handler (async) and the Wayland callbacks (sync, via `blocking_dispatch`)
    /// can both access it. The Wayland thread never holds this across an await,
    /// so there is no risk of deadlocking the event loop.
    render_states: std::sync::Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<RenderState>>>,
        >,
    >,
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
        _new_factor: i32,
    ) {
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

impl wayland_client::Dispatch<wayland_client::protocol::wl_region::WlRegion, ()> for WaylandState {
    fn event(
        _state: &mut WaylandState,
        _region: &wayland_client::protocol::wl_region::WlRegion,
        _event: wayland_client::protocol::wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<WaylandState>,
    ) {
    }
}

impl Dispatch<WpViewporter, ()> for WaylandState {
    fn event(
        _state: &mut WaylandState,
        _proxy: &WpViewporter,
        _event: wp_viewporter::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<WaylandState>,
    ) {
    }
}

impl Dispatch<WpViewport, ()> for WaylandState {
    fn event(
        _state: &mut WaylandState,
        _proxy: &WpViewport,
        _event: wp_viewport::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<WaylandState>,
    ) {
    }
}

fn viewport_destination(configured: (u32, u32), physical: (u32, u32)) -> Option<(i32, i32)> {
    let (width, height) = configured;
    let (physical_width, physical_height) = physical;
    match (width, height) {
        (0, 0) => None,
        (0, height) if physical_height > 0 => {
            let width = (u64::from(height) * u64::from(physical_width)
                + u64::from(physical_height) / 2)
                / u64::from(physical_height);
            Some((i32::try_from(width).ok()?, i32::try_from(height).ok()?))
        }
        (width, 0) if physical_width > 0 => {
            let height = (u64::from(width) * u64::from(physical_height)
                + u64::from(physical_width) / 2)
                / u64::from(physical_width);
            Some((i32::try_from(width).ok()?, i32::try_from(height).ok()?))
        }
        (width, height) => Some((i32::try_from(width).ok()?, i32::try_from(height).ok()?)),
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::{VideoPresentAction, video_present_action, viewport_destination};
    use crate::renderer::FrameStatus;

    #[test]
    fn preserves_complete_configure_size() {
        assert_eq!(
            viewport_destination((3072, 1728), (3840, 2160)),
            Some((3072, 1728))
        );
    }

    #[test]
    fn derives_missing_dimension_from_physical_aspect_ratio() {
        assert_eq!(
            viewport_destination((0, 1728), (3840, 2160)),
            Some((3072, 1728))
        );
        assert_eq!(
            viewport_destination((3072, 0), (3840, 2160)),
            Some((3072, 1728))
        );
        assert_eq!(viewport_destination((0, 0), (3840, 2160)), None);
    }

    #[test]
    fn retries_recoverable_video_surface_failures() {
        assert_eq!(
            video_present_action(FrameStatus::TimedOut),
            VideoPresentAction::Retry
        );
        assert_eq!(
            video_present_action(FrameStatus::Outdated),
            VideoPresentAction::Reconfigure
        );
        assert_eq!(
            video_present_action(FrameStatus::Lost),
            VideoPresentAction::Reconfigure
        );
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
        // SCTK acknowledges the configure. wgpu owns subsequent buffer commits,
        // which must not race with a bufferless commit when explicit sync is active.
        let output_id = self.surfaces.iter().find_map(|(output_id, surface)| {
            (surface.wl_surface().id() == layer.wl_surface().id()).then_some(*output_id)
        });
        let Some(output_id) = output_id else {
            return;
        };
        let destination = self.outputs.get(&output_id).and_then(|output| {
            viewport_destination(configure.new_size, (output.width, output.height))
        });
        if let Some(viewport) = self.viewports.get(&output_id) {
            let Some((logical_width, logical_height)) = destination else {
                tracing::warn!(
                    "Output {output_id} configure omitted both dimensions; waiting for a usable size"
                );
                return;
            };
            viewport.set_destination(logical_width, logical_height);
            tracing::debug!(
                "Configured viewport destination for output {output_id}: {logical_width}x{logical_height}"
            );
        }
        if !self.pending_restores.remove(&output_id) {
            return;
        }
        let Some(lifecycle) = self.output_lifecycles.get(&output_id).cloned() else {
            return;
        };
        let Some(render_states) = self
            .hotplug
            .as_ref()
            .map(|hotplug| hotplug.render_states.clone())
        else {
            return;
        };

        tokio::spawn(async move {
            if !lifecycle.active.load(Ordering::SeqCst) {
                return;
            }
            restore_cached_wallpaper(&lifecycle.name, &lifecycle.render_state).await;

            let mut states = render_states.lock().await;
            if lifecycle.active.load(Ordering::SeqCst) {
                states.insert(lifecycle.name.clone(), lifecycle.render_state);
                tracing::info!("Output configured: {}", lifecycle.name);
            }
        });
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
        output: wl_output::WlOutput,
    ) {
        let id = output.id().protocol_id();
        tracing::info!("Output detected: protocol_id={id}");

        let mut info = OutputInfo {
            name: format!("output-{id}"),
            width: 1920,
            height: 1080,
            scale_factor: 1,
            wl_output: output,
        };

        // Resolve the compositor-provided name (e.g. "DP-1", "HDMI-A-1").
        if let Some(info_data) = self.output_state.info(&info.wl_output) {
            if let Some(mode) = info_data.modes.iter().find(|m| m.current) {
                info.width = mode.dimensions.0 as u32;
                info.height = mode.dimensions.1 as u32;
            }
            info.scale_factor = info_data.scale_factor;
            let resolved = info_data
                .name
                .as_deref()
                .filter(|n| !n.is_empty())
                .or(info_data.description.as_deref().filter(|n| !n.is_empty()));
            if let Some(real_name) = resolved {
                info.name = real_name.to_string();
            } else if !info_data.make.is_empty() || !info_data.model.is_empty() {
                let fallback = format!("{} {}", info_data.make, info_data.model)
                    .trim()
                    .to_string();
                if !fallback.is_empty() {
                    info.name = fallback;
                }
            }
        }

        self.outputs.insert(id, info.clone());

        // Create a render state for the new output.
        // Extract values from hotplug before passing &mut self to avoid
        // borrow checker conflicts (hotplug is inside self).
        if self.hotplug.is_some() {
            let name = info.name.clone();
            let renderer = self.hotplug.as_ref().unwrap().renderer.clone();
            let display_ptr = self.hotplug.as_ref().unwrap().display_ptr.0;
            let config = self.hotplug.as_ref().unwrap().config.clone();
            match create_render_state_for_output_sync(
                &renderer,
                display_ptr,
                self,
                _qh,
                &info,
                &config,
            ) {
                Ok(rs) => {
                    let rs = std::sync::Arc::new(tokio::sync::Mutex::new(rs));
                    self.output_lifecycles.insert(
                        id,
                        OutputLifecycle {
                            name,
                            render_state: rs,
                            active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                        },
                    );
                    self.pending_restores.insert(id);
                }
                Err(e) => {
                    tracing::error!("Hotplug: failed to create render state for {name}: {e}");
                }
            }
        }
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = output.id().protocol_id();
        if let Some(info) = self.outputs.get_mut(&id) {
            let mut new_width = info.width;
            let mut new_height = info.height;
            let mut new_scale = info.scale_factor;

            if let Some(mode) = self
                .output_state
                .info(&output)
                .and_then(|i| i.modes.iter().find(|m| m.current).cloned())
            {
                new_width = mode.dimensions.0 as u32;
                new_height = mode.dimensions.1 as u32;
            }
            if let Some(info_data) = self.output_state.info(&output) {
                new_scale = info_data.scale_factor;
                let resolved = info_data
                    .name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .or(info_data.description.as_deref().filter(|n| !n.is_empty()));
                if let Some(real_name) = resolved {
                    if real_name != info.name {
                        tracing::info!(
                            "Output {id}: resolved name '{}' -> '{}'",
                            info.name,
                            real_name
                        );
                        info.name = real_name.to_string();
                    }
                } else if info.name.starts_with("output-")
                    && (!info_data.make.is_empty() || !info_data.model.is_empty())
                {
                    let fallback = format!("{} {}", info_data.make, info_data.model)
                        .trim()
                        .to_string();
                    if !fallback.is_empty() {
                        tracing::info!(
                            "Output {id}: fallback name '{}' -> '{}'",
                            info.name,
                            fallback
                        );
                        info.name = fallback;
                    }
                }
            }

            let changed = new_width != info.width
                || new_height != info.height
                || new_scale != info.scale_factor;

            info.width = new_width;
            info.height = new_height;
            info.scale_factor = new_scale;

            // Reconfigure the wgpu surface when dimensions or scale change.
            if changed {
                let name = info.name.clone();
                if let Some(ref hotplug) = self.hotplug {
                    let render_states = hotplug.render_states.clone();
                    let renderer = hotplug.renderer.clone();
                    tokio::spawn(async move {
                        let states = render_states.lock().await;
                        if let Some(rs) = states.get(&name) {
                            let mut lock = rs.lock().await;
                            lock.width = new_width;
                            lock.height = new_height;
                            let surf_config = wgpu::SurfaceConfiguration {
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                format: lock.format,
                                width: new_width,
                                height: new_height,
                                present_mode: wgpu::PresentMode::Fifo,
                                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                                view_formats: vec![],
                                desired_maximum_frame_latency: 2,
                            };
                            lock.surface.configure(&renderer.device, &surf_config);
                            tracing::info!(
                                "Hotplug: reconfigured {name} to {new_width}x{new_height}"
                            );
                        }
                    });
                }
            }
        }
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = output.id().protocol_id();
        if let Some(info) = self.outputs.remove(&id) {
            tracing::info!("Output disconnected: {} (protocol_id={})", info.name, id);
            self.pending_restores.remove(&id);
            let viewport = self.viewports.remove(&id);
            let layer_surface = self
                .surfaces
                .iter()
                .position(|(pid, _)| *pid == id)
                .map(|position| self.surfaces.swap_remove(position).1);
            let lifecycle = self.output_lifecycles.remove(&id);
            if let Some(lifecycle) = &lifecycle {
                lifecycle.active.store(false, Ordering::SeqCst);
            }
            // Remove the render state from the shared map and stop playback.
            if let (Some(hotplug), Some(lifecycle)) = (&self.hotplug, lifecycle) {
                let render_states = hotplug.render_states.clone();
                tokio::spawn(async move {
                    let mut states = render_states.lock().await;
                    if states
                        .get(&lifecycle.name)
                        .is_some_and(|state| Arc::ptr_eq(state, &lifecycle.render_state))
                    {
                        states.remove(&lifecycle.name);
                    }
                    drop(states);

                    let state = lifecycle.render_state.lock().await;
                    state.playback_gen.fetch_add(1, Ordering::SeqCst);
                    state.pacer.notify();
                    state.video_playback.stop();
                    let render_lock = state.render_lock.clone();
                    drop(state);

                    let _ = tokio::task::spawn_blocking(move || {
                        drop(
                            render_lock
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()),
                        );
                    })
                    .await;
                    if let Some(viewport) = viewport {
                        viewport.destroy();
                    }
                    drop(layer_surface);
                    tracing::info!("Hotplug: cleaned up render state for {}", lifecycle.name);
                });
            } else {
                if let Some(viewport) = viewport {
                    viewport.destroy();
                }
                drop(layer_surface);
            }
        }
    }
}

delegate_compositor!(WaylandState);
delegate_layer!(WaylandState);
delegate_output!(WaylandState);
delegate_registry!(WaylandState);
delegate_shm!(WaylandState);

/// Wakes paced live-playback loops when a new commit bumps the generation.
struct LivePacer {
    lock: std::sync::Mutex<()>,
    cond: std::sync::Condvar,
}

impl LivePacer {
    fn new() -> Self {
        Self {
            lock: std::sync::Mutex::new(()),
            cond: std::sync::Condvar::new(),
        }
    }

    fn notify(&self) {
        let _guard = self.lock.lock().unwrap();
        self.cond.notify_all();
    }

    /// Blocks until `deadline` or until `notify` is called, whichever comes
    /// first.
    fn wait_until(&self, deadline: std::time::Instant) {
        let guard = self.lock.lock().unwrap();
        let now = std::time::Instant::now();
        if deadline <= now {
            return;
        }
        let _ = self
            .cond
            .wait_timeout_while(guard, deadline - now, |_| true);
    }
}

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
    /// Wakes paced live-playback loops when a new commit bumps the generation,
    /// so an old player exits immediately instead of after its sleep quantum.
    pacer: std::sync::Arc<LivePacer>,
    current_bind: Option<wgpu::BindGroup>,
    current_tex: Option<wgpu::Texture>,
    width: u32,
    height: u32,
    current_width: u32,
    current_height: u32,
    format: wgpu::TextureFormat,
    /// Video playback manager
    video_playback: std::sync::Arc<crate::video::VideoPlayback>,
    /// Hardware backend to request for new decoders (from `video.hw_decode`).
    hw_accel: crate::video::HwAccel,
    /// Maximum decoded frames buffered ahead of presentation.
    preload_frames: usize,
    /// Optional cap for live video presentation.
    max_fps: Option<u32>,
    /// Current scaling mode for live playback.
    scaling_mode: u32,
    /// Per-output uniform buffer + bind group (Issue #9 race fix).
    per_output_uniforms: std::sync::Arc<crate::renderer::PerOutputUniforms>,
    /// Path of the last wallpaper set on this output (for restore).
    last_wallpaper: Option<std::path::PathBuf>,
    /// Previous wallpaper state before blank (for restore).
    pre_blank: Option<(std::path::PathBuf, u32)>,
    /// Whether this output is currently blanked.
    blanked: bool,
    /// GIF playback paused state (shared with play_live task).
    gif_paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
    /// Video metadata when committed file is a video.
    is_video: bool,
    /// Plane and conversion resources retained across transition and playback.
    video_texture: Option<crate::renderer::VideoTexture>,
    /// Playback generation captured at commit time; live playback stops when
    /// it no longer matches `RenderState::playback_gen`.
    generation: u64,
    /// Scaling mode: 0=Fill, 1=Fit, 2=Stretch, 3=Center, 4=Tile.
    scaling_mode: u32,
    /// Optional cap for live video presentation.
    max_fps: Option<u32>,
}

impl RenderState {
    async fn set_wallpaper(
        &mut self,
        path: &std::path::Path,
        effect: &crate::animation::Effect,
        duration_ms: u32,
        scaling_mode: u32,
    ) -> anyhow::Result<()> {
        self.scaling_mode = scaling_mode;
        let commit = self.commit_wallpaper(path, scaling_mode)?;
        self.spawn_transition(commit, effect, duration_ms);
        // Update last_wallpaper after successful commit
        self.last_wallpaper = Some(path.to_path_buf());
        Ok(())
    }

    /// Loads the new wallpaper and atomically promotes it to the current
    /// frame. The outgoing bind group stays alive for the transition, so the
    /// render task can keep drawing from it after this commit returns.
    fn commit_wallpaper(
        &mut self,
        path: &std::path::Path,
        scaling_mode: u32,
    ) -> anyhow::Result<CommitData> {
        use image::ImageReader;

        // Check if this is a video file FIRST
        if crate::video::VideoDecoder::is_video_file(path) {
            tracing::info!("Video file detected: {:?}", path);

            // Invalidate any in-flight video render task BEFORE touching the
            // shared decoder: the old task checks the generation on every
            // iteration, so bumping first makes it exit (or skip the upload)
            // before it could pull a frame of the new resolution from the
            // freshly started decoder.
            let generation = self.playback_gen.fetch_add(1, Ordering::SeqCst) + 1;
            self.pacer.notify();

            // Start video playback (replaces any previous playback and joins
            // its decode thread, releasing the old decoder's buffers).
            let metadata =
                self.video_playback
                    .start(path, self.hw_accel, self.preload_frames, generation)?;

            // Wait for the first frame so the transition's incoming image is
            // the real first frame, not a black placeholder.
            let first_frame = self
                .video_playback
                .wait_first_frame(std::time::Duration::from_millis(1000));

            let (tex_width, tex_height) = first_frame
                .as_ref()
                .map(|frame| (frame.width, frame.height))
                .unwrap_or((metadata.width, metadata.height));
            let video_texture = self.renderer.create_video_texture(tex_width, tex_height);
            let (img_width, img_height) = if let Some(frame) = first_frame {
                self.renderer
                    .update_video_texture(&video_texture, &frame.data)?;
                (frame.width, frame.height)
            } else {
                // WebGPU initializes the output to black when no frame arrives.
                tracing::warn!("No first frame available, using black texture");
                (metadata.width, metadata.height)
            };
            let new_tex = video_texture.texture().clone();
            let new_bind = video_texture.bind_group().clone();

            let old_bind = self.current_bind.take();
            let (old_img_width, old_img_height) = if old_bind.is_some() {
                (self.current_width.max(1), self.current_height.max(1))
            } else {
                (img_width, img_height)
            };
            let bg_bind = old_bind.unwrap_or_else(|| new_bind.clone());

            drop(self.current_tex.take());
            self.current_tex = Some(new_tex);
            self.current_bind = Some(new_bind.clone());
            self.current_width = img_width;
            self.current_height = img_height;

            return Ok(CommitData {
                bg_bind,
                new_bind,
                img_width,
                img_height,
                old_img_width,
                old_img_height,
                format: self.format,
                width: self.width,
                height: self.height,
                animated: None,
                is_video: true,
                video_texture: Some(video_texture),
                generation,
                scaling_mode,
                max_fps: self.max_fps,
            });
        }

        // A static image or GIF supersedes any video: release the video
        // decoder and its buffers immediately (the generation bump also stops
        // the video render task on its next vsync).
        self.video_playback.stop();

        // Stream animated frames (GIF) on demand during playback; the
        // transition's incoming texture is the GIF's first frame.
        let mut animated = crate::animated::AnimatedImage::decode(path)?;
        let (new_tex, new_bind, img_width, img_height) = if let Some(anim) = animated.as_mut() {
            let (w, h) = (anim.width, anim.height);
            let (tex, bind) = self.renderer.create_texture(w, h);
            let first = anim.first_frame();
            if !first.is_empty() {
                self.renderer.update_texture(&tex, first, w, h);
            }
            (tex, bind, w, h)
        } else {
            let new_img = ImageReader::open(path)?.decode()?;
            let (tex, bind) = self.renderer.load_texture(&new_img)?;
            (tex, bind, new_img.width(), new_img.height())
        };

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
        self.pacer.notify();

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
            is_video: false,
            video_texture: None,
            generation,
            scaling_mode,
            max_fps: self.max_fps,
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
        let pacer = self.pacer.clone();
        let video_playback = self.video_playback.clone();
        let per_output_uniforms = std::sync::Arc::clone(&self.per_output_uniforms);
        let gif_paused = self.gif_paused.clone();
        let effect = effect.clone();
        drop(tokio::task::spawn_blocking(move || {
            render_transition(
                renderer,
                surface,
                render_lock,
                playback_gen,
                pacer,
                video_playback,
                gif_paused,
                commit,
                effect,
                duration_ms,
                &per_output_uniforms,
            );
        }));
    }
}

async fn restore_cached_wallpaper(name: &str, render_state: &Arc<Mutex<RenderState>>) {
    let state_path = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(format!("wallr/last_wallpaper/{name}"));
    let Ok(path_str) = std::fs::read_to_string(state_path) else {
        return;
    };
    let path = std::path::Path::new(path_str.trim());
    if !path.exists() {
        return;
    }

    let mut state = render_state.lock().await;
    let effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
    if let Err(err) = state.set_wallpaper(path, &effect, 1000, 0).await {
        tracing::warn!("Failed to restore wallpaper for {name}: {err}");
    }
}

/// Presents one frame per vsync until the wall-clock duration elapses. With
/// PresentMode::Fifo, `get_current_texture` blocks until the previous frame
/// is presented, so this loop is paced to the monitor refresh rate, and the
/// transition lasts exactly `duration_ms` on any refresh rate — frame-count
/// pacing would run too fast on high-refresh panels and too slow when the
/// present rate is low. If the compositor stops presenting, the loop can park
/// inside a present; that is fine here because the task is detached.
#[allow(clippy::too_many_arguments)]
fn render_transition(
    renderer: std::sync::Arc<Renderer>,
    surface: &'static wgpu::Surface<'static>,
    render_lock: std::sync::Arc<std::sync::Mutex<()>>,
    playback_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pacer: std::sync::Arc<LivePacer>,
    video_playback: std::sync::Arc<crate::video::VideoPlayback>,
    gif_paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mut commit: CommitData,
    effect: crate::animation::Effect,
    duration_ms: u32,
    per_output_uniforms: &crate::renderer::PerOutputUniforms,
) {
    let _guard = render_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let duration = std::time::Duration::from_millis(u64::from(duration_ms.max(1)));
    let start = std::time::Instant::now();
    loop {
        let progress = start.elapsed().as_secs_f32() / duration.as_secs_f32();
        let uniforms = crate::animation::compute_effect_uniforms(&effect, progress.clamp(0.0, 1.0));
        let status = renderer.render_frame(
            crate::renderer::FrameRequest {
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
                scaling_mode: commit.scaling_mode,
            },
            per_output_uniforms,
        );
        let status = match status {
            Ok(status) => status,
            Err(err) => {
                eprintln!("wallr: transition render failed: {err}");
                break;
            }
        };
        if progress >= 1.0 || status != crate::renderer::FrameStatus::Presented {
            break;
        }
    }

    // The transition ended; if the committed wallpaper is an animated GIF and
    // nothing superseded it while we rendered, keep the render lock and play
    // the frames live until the next commit bumps the generation.
    let mut animated = commit.animated.take();
    if let Some(animated) = animated.as_mut()
        && playback_gen.load(Ordering::SeqCst) == commit.generation
    {
        play_live(
            &renderer,
            surface,
            &commit,
            animated,
            &playback_gen,
            &pacer,
            &gif_paused,
            per_output_uniforms,
        );
    } else if commit.is_video && playback_gen.load(Ordering::SeqCst) == commit.generation {
        play_video(
            &renderer,
            surface,
            &commit,
            &video_playback,
            &playback_gen,
            &pacer,
            per_output_uniforms,
        );
    }
}

/// Presents live wallpaper frames until the next commit. One frame is
/// presented per GIF frame boundary instead of at the monitor refresh rate.
/// Two textures are double-buffered and frames are decompressed directly
/// into a mapped staging ring (no intermediate copy), so the wake path only
/// presents and the pacing sleep hides the decode/upload entirely.
#[allow(clippy::too_many_arguments)]
fn play_live(
    renderer: &Renderer,
    surface: &'static wgpu::Surface<'static>,
    commit: &CommitData,
    animated: &mut crate::animated::AnimatedImage,
    playback_gen: &std::sync::atomic::AtomicU64,
    pacer: &LivePacer,
    gif_paused: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    per_output_uniforms: &crate::renderer::PerOutputUniforms,
) {
    let (tex_a, bind_a) = renderer.create_texture(animated.width, animated.height);
    let (tex_b, bind_b) = renderer.create_texture(animated.width, animated.height);
    let (frame_w, frame_h) = (animated.width, animated.height);
    let (bytes_per_row, rows) = (frame_w * 4, frame_h);
    let frame_bytes = bytes_per_row as u64 * rows as u64;

    // Map+decompress+copy path needs a byte-per-row multiple of the copy
    // alignment; fall back to write_texture for odd widths.
    let direct_upload = bytes_per_row % 256 == 0;
    let staging: Vec<wgpu::Buffer> = if direct_upload {
        (0..2)
            .map(|_| {
                renderer.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("wallr-gif-staging"),
                    size: frame_bytes,
                    usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    let first = animated.first_frame();
    if !first.is_empty() {
        renderer.update_texture(&tex_a, first, frame_w, frame_h);
        renderer.update_texture(&tex_b, first, frame_w, frame_h);
    }
    let binds = [bind_a, bind_b];
    let textures = [tex_a, tex_b];

    // Uploads frame `index` into `textures[tgt]`. Returns true when the GPU
    // copy was recorded.
    let upload = |renderer: &Renderer,
                  tgt: usize,
                  index: usize,
                  slot: usize,
                  animated: &mut crate::animated::AnimatedImage|
     -> bool {
        if direct_upload {
            let buffer = &staging[slot];
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Write, |_| {});
            renderer.device.poll(wgpu::Maintain::Wait);
            let ok = {
                let mut mapped = slice.get_mapped_range_mut();
                animated.decompress_into(index, &mut mapped)
            };
            buffer.unmap();
            if ok {
                let mut encoder = renderer
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bytes_per_row),
                            rows_per_image: Some(rows),
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &textures[tgt],
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: frame_w,
                        height: frame_h,
                        depth_or_array_layers: 1,
                    },
                );
                renderer.queue.submit([encoder.finish()]);
                return true;
            }
        } else {
            let frame = animated.frame_at(index);
            if !frame.is_empty() {
                renderer.update_texture(&textures[tgt], frame, frame_w, frame_h);
                return true;
            }
        }
        false
    };

    let mut cur = 0usize; // texture index currently holding the presented frame
    let mut cur_frame = 0usize; // frame index currently in texture `cur`
    let mut next_frame = 0usize; // frame index currently in the idle texture
    let mut slot = 0usize; // staging ring slot for the next upload
    let start = std::time::Instant::now();
    let mut paused_elapsed = std::time::Duration::ZERO; // Accumulated pause time
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
    loop {
        if playback_gen.load(Ordering::SeqCst) != commit.generation {
            return;
        }

        // Check if paused - if so, keep presenting the current frame but don't advance
        if gif_paused.load(Ordering::SeqCst) {
            let pause_start = std::time::Instant::now();
            // Keep presenting the current frame while paused
            let uniforms = crate::animation::compute_effect_uniforms(&static_effect, 1.0);
            let status = renderer.render_frame(
                crate::renderer::FrameRequest {
                    surface,
                    format: commit.format,
                    bg_bind: &binds[cur],
                    new_bind: &binds[cur],
                    effect: &uniforms,
                    width: commit.width,
                    height: commit.height,
                    img_width: animated.width,
                    img_height: animated.height,
                    old_img_width: animated.width,
                    old_img_height: animated.height,
                    scaling_mode: commit.scaling_mode,
                },
                per_output_uniforms,
            );
            match status {
                Ok(crate::renderer::FrameStatus::Presented) => {}
                _ => return,
            }
            // Wait a bit before checking again
            std::thread::sleep(std::time::Duration::from_millis(16));
            paused_elapsed += pause_start.elapsed();
            continue;
        }

        let index = animated.frame_index_at(start.elapsed() - paused_elapsed);
        if index != cur_frame {
            if next_frame != index {
                upload(renderer, cur ^ 1, index, slot, animated);
                slot ^= 1;
                next_frame = index;
            }
            cur ^= 1;
            cur_frame = index;
        }
        let uniforms = crate::animation::compute_effect_uniforms(&static_effect, 1.0);
        let status = renderer.render_frame(
            crate::renderer::FrameRequest {
                surface,
                format: commit.format,
                bg_bind: &binds[cur],
                new_bind: &binds[cur],
                effect: &uniforms,
                width: commit.width,
                height: commit.height,
                img_width: animated.width,
                img_height: animated.height,
                old_img_width: animated.width,
                old_img_height: animated.height,
                scaling_mode: commit.scaling_mode,
            },
            per_output_uniforms,
        );
        match status {
            Ok(crate::renderer::FrameStatus::Presented) => {}
            _ => return,
        }

        // Pace to the next GIF frame boundary instead of presenting at the
        // monitor refresh rate: an animated wallpaper only needs a present
        // when its frame changes. A commit wakes us via the pacer. The
        // boundary is computed in absolute time (frame_start is loop-relative,
        // so add the completed loops) to stay correct after the animation
        // wraps. While waiting, warm the idle texture with the next frame so
        // the wake path stays on the hot critical section.
        let elapsed = start.elapsed() - paused_elapsed;
        let total: std::time::Duration = animated.total_duration();
        let loops = (elapsed.as_millis() / total.as_millis().max(1)) as u64;
        let next_change = animated.frame_start(index + 1) + total * (loops as u32);
        let wait = next_change.saturating_sub(elapsed);
        if wait > std::time::Duration::ZERO {
            let next = index + 1;
            if next_frame != next {
                upload(renderer, cur ^ 1, next, slot, animated);
                slot ^= 1;
                next_frame = next;
            }
            pacer.wait_until(std::time::Instant::now() + wait);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoPresentAction {
    Presented,
    Retry,
    Reconfigure,
}

fn video_present_action(status: crate::renderer::FrameStatus) -> VideoPresentAction {
    match status {
        crate::renderer::FrameStatus::Presented => VideoPresentAction::Presented,
        crate::renderer::FrameStatus::TimedOut => VideoPresentAction::Retry,
        crate::renderer::FrameStatus::Outdated | crate::renderer::FrameStatus::Lost => {
            VideoPresentAction::Reconfigure
        }
    }
}

/// Live video playback loop: continuously updates texture with decoded frames.
fn play_video(
    renderer: &Renderer,
    surface: &'static wgpu::Surface<'static>,
    commit: &CommitData,
    video_playback: &std::sync::Arc<crate::video::VideoPlayback>,
    playback_gen: &std::sync::atomic::AtomicU64,
    pacer: &LivePacer,
    per_output_uniforms: &crate::renderer::PerOutputUniforms,
) {
    let (width, height) = (commit.img_width, commit.img_height);

    let Some(texture) = commit.video_texture.as_ref() else {
        tracing::warn!("Video conversion resources unavailable");
        return;
    };
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());

    let min_frame_interval = commit
        .max_fps
        .filter(|fps| *fps > 0)
        .map(|fps| std::time::Duration::from_secs_f64(1.0 / f64::from(fps)));
    let mut last_present = None;
    let mut warned_size_mismatch = false;

    loop {
        // A newer commit superseded us. Do NOT touch the shared
        // `video_playback` here: the successor commit already replaced the
        // decoder (video) or stopped it (static image), and stopping it now
        // would kill the successor's playback too.
        if playback_gen.load(Ordering::SeqCst) != commit.generation {
            return;
        }

        if let (Some(interval), Some(previous)) = (min_frame_interval, last_present) {
            pacer.wait_until(previous + interval);
            if playback_gen.load(Ordering::SeqCst) != commit.generation {
                return;
            }
        }

        // Pull the next displayable frame. The decoder queue is bounded, so
        // this never blocks; unchanged frames need no upload or presentation.
        let frame_uploaded = if let Some(frame) =
            video_playback.next_frame_in_generation(commit.generation)
        {
            // The shared decoder can be replaced between commits; never
            // upload a frame whose size does not match this task's texture.
            if frame.width != width || frame.height != height {
                if !warned_size_mismatch {
                    tracing::warn!(
                        "Skipping video frame with unexpected size {}x{} (expected {}x{})",
                        frame.width,
                        frame.height,
                        width,
                        height
                    );
                    warned_size_mismatch = true;
                }
                pacer.wait_until(std::time::Instant::now() + std::time::Duration::from_millis(2));
                continue;
            }
            if let Err(err) = renderer.update_video_texture(texture, &frame.data) {
                tracing::warn!("Video frame upload failed: {err}");
                return;
            }
            true
        } else {
            false
        };

        if !frame_uploaded {
            if playback_gen.load(Ordering::SeqCst) != commit.generation {
                return;
            }
            let wait = video_playback
                .time_until_next_frame_in_generation(commit.generation)
                .unwrap_or(std::time::Duration::from_millis(2));
            if playback_gen.load(Ordering::SeqCst) != commit.generation {
                return;
            }
            pacer.wait_until(std::time::Instant::now() + wait);
            continue;
        }

        let uniforms = crate::animation::compute_effect_uniforms(&static_effect, 1.0);
        let status = renderer.render_frame(
            crate::renderer::FrameRequest {
                surface,
                format: commit.format,
                bg_bind: texture.bind_group(),
                new_bind: texture.bind_group(),
                effect: &uniforms,
                width: commit.width,
                height: commit.height,
                img_width: width,
                img_height: height,
                old_img_width: width,
                old_img_height: height,
                scaling_mode: commit.scaling_mode,
            },
            per_output_uniforms,
        );

        match status.map(video_present_action) {
            Ok(VideoPresentAction::Presented) => {}
            Ok(action) => {
                if action == VideoPresentAction::Reconfigure {
                    surface.configure(
                        &renderer.device,
                        &wgpu::SurfaceConfiguration {
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format: commit.format,
                            width: commit.width,
                            height: commit.height,
                            present_mode: wgpu::PresentMode::Fifo,
                            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                            view_formats: vec![],
                            desired_maximum_frame_latency: 2,
                        },
                    );
                }
                pacer.wait_until(std::time::Instant::now() + std::time::Duration::from_millis(100));
            }
            Err(err) => {
                tracing::warn!("Video present failed ({err}), stopping playback");
                video_playback.stop();
                return;
            }
        }
    }
}

/// Resolve target render states from an optional monitor name.
/// Returns all states when `monitor` is None, or the specific named state.
/// Returns an empty vec for unknown monitor names (letting callers return
/// an error).
async fn resolve_targets(
    render_states: &std::collections::HashMap<
        String,
        std::sync::Arc<tokio::sync::Mutex<RenderState>>,
    >,
    monitor: Option<&str>,
) -> Vec<std::sync::Arc<tokio::sync::Mutex<RenderState>>> {
    match monitor {
        Some(name) => {
            if let Some(rs) = render_states.get(name) {
                vec![rs.clone()]
            } else {
                Vec::new()
            }
        }
        None => render_states.values().cloned().collect(),
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

        // Bind compositor once for creating empty input regions (passthrough).
        let compositor = globals
            .bind::<wl_compositor::WlCompositor, WaylandState, smithay_client_toolkit::globals::GlobalData>(
                &qh,
                1..=4,
                smithay_client_toolkit::globals::GlobalData,
            )
            .map_err(|e| DaemonError::StartError(format!("compositor bind failed: {e:?}")))?;
        let viewporter = globals
            .bind::<WpViewporter, WaylandState, ()>(&qh, 1..=1, ())
            .ok();
        if viewporter.is_none() {
            tracing::warn!(
                "wp_viewporter is unavailable; fractional outputs will use integer buffer scaling"
            );
        }

        let mut wayland_state = WaylandState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            compositor_state,
            shm,
            outputs: std::collections::HashMap::new(),
            surfaces: Vec::new(),
            viewporter,
            viewports: std::collections::HashMap::new(),
            output_lifecycles: std::collections::HashMap::new(),
            pending_restores: std::collections::HashSet::new(),
            layer_shell,
            compositor,
            hotplug: None,
        };

        // Multiple roundtrips: some compositors deliver output events lazily
        // across several dispatch cycles. Five roundtrips ensures all outputs
        // are discovered and their modes/scale are populated.
        for i in 0..5 {
            event_queue
                .roundtrip(&mut wayland_state)
                .map_err(|e| DaemonError::StartError(format!("roundtrip {i} failed: {e:?}")))?;
        }

        if wayland_state.outputs.is_empty() {
            return Err(DaemonError::StartError(
                "no outputs detected after roundtrip".into(),
            ));
        }

        tracing::info!(
            "Detected {} output(s): {:?}",
            wayland_state.outputs.len(),
            wayland_state
                .outputs
                .values()
                .map(|o| format!("{} ({}x{})", o.name, o.width, o.height))
                .collect::<Vec<_>>()
        );

        let renderer = std::sync::Arc::new(renderer);

        // Create a LayerSurface, wgpu Surface, and RenderState for every
        // known output. The key is the output's human-readable name (e.g.
        // "DP-1", "eDP-1") so IPC can target a specific monitor.
        let render_states_map: std::collections::HashMap<String, Arc<Mutex<RenderState>>> =
            std::collections::HashMap::new();

        // Collect output info first so we can pass &mut wayland_state to the
        // helper (we need &mut to push LayerSurfaces into the surfaces vec).
        let output_info: Vec<(u32, OutputInfo)> = wayland_state
            .outputs
            .iter()
            .map(|(k, v)| {
                (
                    *k,
                    OutputInfo {
                        name: v.name.clone(),
                        width: v.width,
                        height: v.height,
                        scale_factor: v.scale_factor,
                        wl_output: v.wl_output.clone(),
                    },
                )
            })
            .collect();

        for (proto_id, info) in &output_info {
            let name = info.name.clone();
            let rs = Self::create_render_state_for_output(
                &renderer,
                display_ptr,
                &mut wayland_state,
                &qh,
                info,
                &self.config,
            )
            .await?;
            let rs = Arc::new(Mutex::new(rs));
            wayland_state.output_lifecycles.insert(
                *proto_id,
                OutputLifecycle {
                    name: name.clone(),
                    render_state: rs,
                    active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                },
            );
            wayland_state.pending_restores.insert(*proto_id);

            tracing::info!("Output ready: {name} ({proto_id})");
        }

        // Wrap the render-state map in Arc<Mutex<...>> so it can be shared
        // between the Wayland event loop (hotplug) and the IPC handler.
        let render_states: std::sync::Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, Arc<Mutex<RenderState>>>>,
        > = std::sync::Arc::new(tokio::sync::Mutex::new(render_states_map));

        // Store the hotplug context in WaylandState so output callbacks can
        // create/destroy render states when outputs appear or disappear.
        wayland_state.hotplug = Some(DaemonHotplug {
            renderer: renderer.clone(),
            config: self.config.clone(),
            display_ptr: SendDisplayPtr(display_ptr),
            render_states: render_states.clone(),
        });

        // One more roundtrip to catch outputs that appeared between the
        // initial roundtrips and the hotplug context being stored.
        event_queue
            .roundtrip(&mut wayland_state)
            .map_err(|e| DaemonError::StartError(format!("hotplug roundtrip failed: {e:?}")))?;

        let paused_clone = self.paused.clone();
        let engine_clone = self.engine.clone();
        let render_states_clone = render_states.clone();

        // Graceful shutdown on POSIX signals: stop video decoding, remove the
        // IPC socket, and exit. The compositor releases the layer-shell
        // surface automatically when the process exits.
        {
            let rs_map = render_states.clone();
            let socket_path = socket_path.clone();
            tokio::spawn(async move {
                use tokio::signal::unix::{SignalKind, signal};
                let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
                let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
                let mut hup = signal(SignalKind::hangup()).expect("SIGHUP handler");
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                    _ = hup.recv() => {}
                }
                tracing::info!("Signal received, shutting down gracefully");
                let states = rs_map.lock().await;
                for rs in states.values() {
                    if let Ok(state) = rs.try_lock() {
                        state.video_playback.stop();
                    }
                }
                drop(states);
                let _ = std::fs::remove_file(&socket_path);
                std::process::exit(0);
            });
        }

        let ipc_socket_path = socket_path.clone();
        start_ipc_server(&socket_path, move |cmd| {
            let paused = paused_clone.clone();
            let engine = engine_clone.clone();
            let render_states = render_states_clone.clone();
            let stop_socket = ipc_socket_path.clone();
            async move {
                let render_states = render_states.lock().await;
                match cmd {
                    IpcCommand::Pause { monitor } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: Some("No matching outputs".into()),
                            };
                        }
                        if monitor.is_none() {
                            paused.store(true, Ordering::SeqCst);
                        }
                        for rs in targets {
                            let rs_lock = rs.lock().await;
                            rs_lock.video_playback.pause();
                            rs_lock.gif_paused.store(true, Ordering::SeqCst);
                        }
                        IpcResponse {
                            success: true,
                            message: Some("Paused".into()),
                        }
                    }
                    IpcCommand::Resume { monitor } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: Some("No matching outputs".into()),
                            };
                        }
                        if monitor.is_none() {
                            paused.store(false, Ordering::SeqCst);
                        }
                        for rs in targets {
                            let rs_lock = rs.lock().await;
                            rs_lock.video_playback.resume();
                            rs_lock.gif_paused.store(false, Ordering::SeqCst);
                        }
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
                        scaling_mode,
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

                        // Resolve targets: unknown monitor = error, no monitor = all outputs.
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }

                        // Persist per-output last wallpaper for all targeted outputs.
                        {
                            let persist_names: Vec<&str> = match &monitor {
                                Some(name) => vec![name.as_str()],
                                None => render_states.keys().map(|s| s.as_str()).collect(),
                            };
                            for name in persist_names {
                                let state_path = dirs::cache_dir()
                                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                                    .join(format!("wallr/last_wallpaper/{name}"));
                                if let Some(parent) = state_path.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                let _ = std::fs::write(&state_path, &path);
                            }
                        }

                        let effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        // Live playback only starts after the transition, so for
                        // videos an unrequested 2s fade reads as a long "load".
                        // Default to a short fade unless the user asked for one.
                        let is_video = crate::video::VideoDecoder::is_video_file(&p);
                        let duration = duration_ms.unwrap_or(if is_video { 150 } else { 2000 });
                        let sm = scaling_mode.unwrap_or(crate::config::ScalingMode::Fill);
                        let scaling_mode_u32 = match sm {
                            crate::config::ScalingMode::Fill => 0u32,
                            crate::config::ScalingMode::Fit => 1,
                            crate::config::ScalingMode::Stretch => 2,
                            crate::config::ScalingMode::Center => 3,
                            crate::config::ScalingMode::Tile => 4,
                        };

                        let mut last_err = None;
                        for rs in &targets {
                            let rs_clone = rs.clone();
                            let p_clone = p.clone();
                            let effect_clone = effect.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                let rt = tokio::runtime::Handle::current();
                                rt.block_on(async {
                                    let mut lock = rs_clone.lock().await;
                                    lock.set_wallpaper(
                                        &p_clone,
                                        &effect_clone,
                                        duration,
                                        scaling_mode_u32,
                                    )
                                    .await
                                })
                            })
                            .await;

                            if let Err(e) = result {
                                last_err = Some(format!("Task spawn failed: {e}"));
                                continue;
                            }
                            if let Err(e) = result.unwrap() {
                                last_err = Some(format!("Render failed: {e}"));
                            }
                        }

                        match last_err {
                            Some(e) => IpcResponse {
                                success: false,
                                message: Some(e),
                            },
                            None => {
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
                        }
                    }
                    IpcCommand::Stop => {
                        for rs in render_states.values() {
                            let state = rs.lock().await;
                            state.playback_gen.fetch_add(1, Ordering::SeqCst);
                            state.pacer.notify();
                            state.video_playback.stop();
                        }
                        let sp = stop_socket.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            let _ = std::fs::remove_file(&sp);
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
                    IpcCommand::Seek {
                        timestamp_ms,
                        monitor,
                    } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }
                        // When monitor is unspecified, seek all outputs
                        let mut seek_count = 0u32;
                        let mut errors = Vec::new();
                        for (name, rs) in render_states.iter() {
                            if monitor.as_deref() != Some(name.as_str()) && monitor.is_some() {
                                continue;
                            }
                            let rs_lock = rs.lock().await;
                            match rs_lock
                                .video_playback
                                .seek(std::time::Duration::from_millis(timestamp_ms))
                            {
                                Ok(()) => {
                                    seek_count += 1;
                                }
                                Err(e) => {
                                    errors.push(format!("{}: {}", name, e));
                                }
                            }
                        }
                        if seek_count == 0 {
                            IpcResponse {
                                success: false,
                                message: Some(format!(
                                    "Seek failed on all outputs: {}",
                                    errors.join("; ")
                                )),
                            }
                        } else if !errors.is_empty() {
                            IpcResponse {
                                success: true,
                                message: Some(format!(
                                    "Seeked {} output(s) to {}ms, {} failed: {}",
                                    seek_count,
                                    timestamp_ms,
                                    errors.len(),
                                    errors.join("; ")
                                )),
                            }
                        } else {
                            IpcResponse {
                                success: true,
                                message: Some(format!(
                                    "Seeked {} output(s) to {}ms",
                                    seek_count, timestamp_ms
                                )),
                            }
                        }
                    }
                    IpcCommand::Info { monitor } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }

                        let mut lines = vec![
                            format!("wallr v{}", env!("CARGO_PKG_VERSION")),
                            String::new(),
                            format!("Outputs: {}", render_states.len()),
                        ];
                        for name in render_states.keys() {
                            lines.push(format!("  - {name}"));
                        }

                        // Collect target output info
                        for (name, rs) in render_states.iter() {
                            if monitor.is_some() && monitor.as_deref() != Some(name.as_str()) {
                                continue;
                            }
                            let rs_lock = rs.lock().await;
                            let gpu_info =
                                crate::video::gpu::adapter_diagnostics(&rs_lock.renderer.adapter);

                            lines.push(String::new());
                            lines.push(format!("[{name}] {}x{}", rs_lock.width, rs_lock.height));
                            lines.push(gpu_info);

                            match rs_lock.video_playback.metadata() {
                                Some(meta) => {
                                    let decoder_info = rs_lock.video_playback.decoder_info();
                                    let hw = rs_lock.video_playback.hw_accel_in_use();
                                    let state = if rs_lock.video_playback.is_paused() {
                                        "paused"
                                    } else {
                                        "playing"
                                    };
                                    let position = rs_lock
                                        .video_playback
                                        .position()
                                        .map(|p| format!("{:.2}s", p.as_secs_f64()))
                                        .unwrap_or_else(|| "?".to_string());
                                    lines.push(String::new());
                                    lines.push("Video:".into());
                                    lines.push(format!(
                                        "  Resolution: {}x{}",
                                        meta.width, meta.height
                                    ));
                                    lines.push(format!("  FPS: {:.2}", meta.fps));
                                    lines.push(format!(
                                        "  Duration: {:.2}s",
                                        meta.duration.as_secs_f64()
                                    ));
                                    lines.push(format!(
                                        "  Codec: {}",
                                        decoder_info
                                            .as_ref()
                                            .map(|d| d.codec_name.as_str())
                                            .unwrap_or("unknown")
                                    ));
                                    lines.push(format!("  Container: {}", meta.format));
                                    lines.push(format!("  Decoder: {}", hw.name()));
                                    lines.push(format!(
                                        "  GPU Decode: {}",
                                        if hw == crate::video::HwAccel::Software {
                                            "disabled"
                                        } else {
                                            "enabled"
                                        }
                                    ));
                                    lines.push(format!("  State: {} @ {}", state, position));
                                }
                                None => {
                                    lines.push(String::new());
                                    lines.push("Video: none active".into());
                                    lines.push("Decoder: idle".into());
                                }
                            }
                        }

                        IpcResponse {
                            success: true,
                            message: Some(lines.join("\n")),
                        }
                    }
                    IpcCommand::MonitorList => {
                        let mut lines = Vec::new();
                        for (name, rs) in render_states.iter() {
                            let lock = rs.lock().await;
                            lines.push(format!("{}: {}x{}", name, lock.width, lock.height));
                        }
                        if lines.is_empty() {
                            IpcResponse {
                                success: true,
                                message: Some("No monitors connected".into()),
                            }
                        } else {
                            IpcResponse {
                                success: true,
                                message: Some(lines.join("\n")),
                            }
                        }
                    }
                    IpcCommand::MonitorCurrent => {
                        // Return info for the first output as "current".
                        if let Some((name, rs)) = render_states.iter().next() {
                            let lock = rs.lock().await;
                            IpcResponse {
                                success: true,
                                message: Some(format!("{}: {}x{}", name, lock.width, lock.height)),
                            }
                        } else {
                            IpcResponse {
                                success: false,
                                message: Some("No monitors connected".into()),
                            }
                        }
                    }
                    IpcCommand::Blank {
                        monitor,
                        effect,
                        duration_ms,
                    } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }
                        let mut blanked_count = 0u32;
                        let black_effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        let duration = duration_ms.unwrap_or(800);

                        for (name, rs) in render_states.iter() {
                            if monitor.as_deref() != Some(name.as_str()) && monitor.is_some() {
                                continue;
                            }
                            let mut lock = rs.lock().await;
                            if lock.blanked {
                                continue;
                            }
                            lock.pre_blank = Some((
                                lock.last_wallpaper.clone().unwrap_or_default(),
                                lock.scaling_mode,
                            ));
                            lock.blanked = true;
                            let tmp = std::env::temp_dir().join("wallr_blank.png");
                            {
                                let img =
                                    image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
                                let _ = img.save(&tmp);
                            }
                            let _ = lock.set_wallpaper(&tmp, &black_effect, duration, 0).await;
                            blanked_count += 1;
                        }
                        IpcResponse {
                            success: true,
                            message: Some(format!("Blanked {blanked_count} output(s)")),
                        }
                    }
                    IpcCommand::Restore {
                        monitor,
                        effect,
                        duration_ms,
                    } => {
                        let targets = resolve_targets(&render_states, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }
                        let mut restored_count = 0u32;
                        let mut errors = Vec::new();
                        let restore_effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        let duration = duration_ms.unwrap_or(800);

                        for (name, rs) in render_states.iter() {
                            if monitor.as_deref() != Some(name.as_str()) && monitor.is_some() {
                                continue;
                            }
                            let mut lock = rs.lock().await;
                            if !lock.blanked {
                                continue;
                            }
                            if let Some((ref path, scaling_mode)) = lock.pre_blank.clone() {
                                if !path.exists() {
                                    errors
                                        .push(format!("{}: wallpaper path no longer exists", name));
                                    lock.blanked = false;
                                    lock.pre_blank = None;
                                    continue;
                                }
                                match lock
                                    .set_wallpaper(
                                        std::path::Path::new(&path),
                                        &restore_effect,
                                        duration,
                                        scaling_mode,
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        lock.blanked = false;
                                        lock.pre_blank = None;
                                        restored_count += 1;
                                    }
                                    Err(e) => {
                                        errors.push(format!("{}: restore failed: {}", name, e));
                                        lock.blanked = false;
                                        lock.pre_blank = None;
                                    }
                                }
                            } else {
                                errors.push(format!("{}: no previous wallpaper to restore", name));
                                lock.blanked = false;
                            }
                        }
                        if !errors.is_empty() {
                            IpcResponse {
                                success: restored_count > 0,
                                message: Some(format!(
                                    "Restored {} output(s), {} error(s): {}",
                                    restored_count,
                                    errors.len(),
                                    errors.join("; ")
                                )),
                            }
                        } else {
                            IpcResponse {
                                success: true,
                                message: Some(format!("Restored {restored_count} output(s)")),
                            }
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
            self.start_watcher(watch_path, render_states.clone())
                .await?;
        }

        tokio::task::spawn_blocking(move || {
            loop {
                if let Err(e) = event_queue.blocking_dispatch(&mut wayland_state) {
                    eprintln!("Wayland dispatch error: {e:?}");
                    break;
                }
            }
            // The compositor connection is dead (e.g. the compositor exited
            // or killed our layer surface with a protocol error). Rendering
            // can never recover, so exit and let the supervisor restart us.
            eprintln!("wallr: Wayland connection lost, exiting");
            std::process::exit(1);
        });

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    }

    async fn start_watcher(
        &self,
        dir: PathBuf,
        render_states: std::sync::Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, Arc<Mutex<RenderState>>>>,
        >,
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

                // Apply new wallpaper to every connected output.
                let states = render_states.lock().await;
                for (name, rs) in states.iter() {
                    let rs = rs.clone();
                    let eng = engine.clone();
                    let p = path.clone();
                    let name = name.clone();
                    tokio::spawn(async move {
                        let mut lock = rs.lock().await;
                        let effect =
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default());
                        let _ = lock.set_wallpaper(&p, &effect, 600, 0).await;
                        drop(lock);
                        let opts = SetOptions {
                            no_theme: false,
                            theme_provider: None,
                            monitor: Some(name),
                        };
                        let mut elock = eng.lock().await;
                        let _ = elock.set_wallpaper(&p, &opts).await;
                    });
                }
            }
        });

        Ok(())
    }

    /// Creates a LayerSurface, wgpu Surface, and RenderState for a single
    /// Wayland output.
    #[allow(clippy::too_many_arguments)]
    async fn create_render_state_for_output(
        renderer: &std::sync::Arc<Renderer>,
        display_ptr: *mut std::ffi::c_void,
        wayland_state: &mut WaylandState,
        qh: &QueueHandle<WaylandState>,
        output: &OutputInfo,
        config: &WallrConfig,
    ) -> Result<RenderState, DaemonError> {
        let wl_surface = wayland_state.compositor_state.create_surface(qh);
        let layer_surface = wayland_state.layer_shell.create_layer_surface(
            qh,
            wl_surface,
            Layer::Background,
            Some("wallr"),
            Some(&output.wl_output),
        );
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        // Empty input region so clicks pass through to the desktop.
        let empty_region = wayland_state.compositor.create_region(qh, ());
        layer_surface
            .wl_surface()
            .set_input_region(Some(&empty_region));
        let output_id = output.wl_output.id().protocol_id();
        let viewport = wayland_state
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(layer_surface.wl_surface(), qh, ()));
        let scale_factor = if viewport.is_some() {
            1
        } else if output.scale_factor > 0 {
            output.scale_factor
        } else {
            1
        };
        layer_surface.wl_surface().set_buffer_scale(scale_factor);
        layer_surface.commit();
        empty_region.destroy();
        if let Some(viewport) = viewport {
            wayland_state.viewports.insert(output_id, viewport);
        }

        // mode.dimensions already returns physical pixels; do not multiply by scale.
        let width = output.width;
        let height = output.height;

        let raw_surface = layer_surface.wl_surface().id().as_ptr() as *mut std::ffi::c_void;
        wayland_state.surfaces.push((output_id, layer_surface));

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

        // SAFETY: The surface is tied to wayland_state + window_handle, both
        // of which live for the entire process.
        let wgpu_surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };
        let surface: &'static wgpu::Surface<'static> = Box::leak(Box::new(wgpu_surface));

        Ok(RenderState {
            renderer: renderer.clone(),
            surface,
            render_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            playback_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pacer: std::sync::Arc::new(LivePacer::new()),
            current_bind: None,
            current_tex: None,
            width,
            height,
            current_width: 0,
            current_height: 0,
            format: surf_format,
            video_playback: std::sync::Arc::new(crate::video::VideoPlayback::new()),
            hw_accel: crate::video::HwAccel::from_config(&config.video.hw_decode),
            preload_frames: config.video.preload_frames,
            max_fps: config.daemon.max_fps,
            scaling_mode: 0,
            per_output_uniforms: std::sync::Arc::new(renderer.create_per_output_uniforms()),
            last_wallpaper: None,
            pre_blank: None,
            blanked: false,
            gif_paused: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

/// Synchronous version of `Daemon::create_render_state_for_output` for hotplug.
/// Reuses the existing adapter from the renderer instead of requesting a
/// new one, avoiding the async requirement.
#[allow(clippy::too_many_arguments)]
fn create_render_state_for_output_sync(
    renderer: &std::sync::Arc<Renderer>,
    display_ptr: *mut std::ffi::c_void,
    wayland_state: &mut WaylandState,
    qh: &QueueHandle<WaylandState>,
    output: &OutputInfo,
    config: &WallrConfig,
) -> Result<RenderState, DaemonError> {
    let wl_surface = wayland_state.compositor_state.create_surface(qh);
    let layer_surface = wayland_state.layer_shell.create_layer_surface(
        qh,
        wl_surface,
        Layer::Background,
        Some("wallr"),
        Some(&output.wl_output),
    );
    layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

    let empty_region = wayland_state.compositor.create_region(qh, ());
    layer_surface
        .wl_surface()
        .set_input_region(Some(&empty_region));
    let output_id = output.wl_output.id().protocol_id();
    let viewport = wayland_state
        .viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(layer_surface.wl_surface(), qh, ()));
    let scale_factor = if viewport.is_some() {
        1
    } else if output.scale_factor > 0 {
        output.scale_factor
    } else {
        1
    };
    layer_surface.wl_surface().set_buffer_scale(scale_factor);
    layer_surface.commit();
    empty_region.destroy();
    if let Some(viewport) = viewport {
        wayland_state.viewports.insert(output_id, viewport);
    }

    // mode.dimensions already returns physical pixels; do not multiply by scale.
    let width = output.width;
    let height = output.height;

    let raw_surface = layer_surface.wl_surface().id().as_ptr() as *mut std::ffi::c_void;
    wayland_state.surfaces.push((output_id, layer_surface));

    let window_handle = WaylandWindow {
        display: display_ptr,
        surface: raw_surface,
    };

    let wgpu_surface = renderer
        .instance
        .create_surface(&window_handle)
        .map_err(|e| DaemonError::StartError(format!("wgpu surface creation failed: {e:?}")))?;

    let surf_format = {
        let caps = wgpu_surface.get_capabilities(&renderer.adapter);
        caps.formats
            .into_iter()
            .next()
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb)
    };

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

    let wgpu_surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };
    let surface: &'static wgpu::Surface<'static> = Box::leak(Box::new(wgpu_surface));

    Ok(RenderState {
        renderer: renderer.clone(),
        surface,
        render_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        playback_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        pacer: std::sync::Arc::new(LivePacer::new()),
        current_bind: None,
        current_tex: None,
        width,
        height,
        current_width: 0,
        current_height: 0,
        format: surf_format,
        video_playback: std::sync::Arc::new(crate::video::VideoPlayback::new()),
        hw_accel: crate::video::HwAccel::from_config(&config.video.hw_decode),
        preload_frames: config.video.preload_frames,
        max_fps: config.daemon.max_fps,
        scaling_mode: 0,
        per_output_uniforms: std::sync::Arc::new(renderer.create_per_output_uniforms()),
        last_wallpaper: None,
        pre_blank: None,
        blanked: false,
        gif_paused: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
}
