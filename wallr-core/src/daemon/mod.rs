use crate::config::WallrConfig;
use crate::ipc::{IpcCommand, IpcResponse, start_ipc_server};
use crate::renderer::Renderer;
use crate::wallpaper::{SetOptions, WallpaperEngine};
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
    shm::slot::{Buffer as ShmBuffer, SlotPool},
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
    renderer: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Renderer>>>>,
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
    use super::{
        VideoPresentAction, clamp_max_fps, clamp_preload_frames, is_transient_wallpaper_error,
        is_usable_wallpaper_file, persist_wallpaper_at, read_wallpaper_state, validate_live_config,
        video_present_action, viewport_destination, write_wallpaper_state,
    };
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

    #[test]
    fn rotates_wallpaper_state_only_after_successful_persistence() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let first = temporary.path().join("first.jpg");
        let second = temporary.path().join("second.jpg");
        std::fs::write(&first, b"first").expect("first wallpaper");
        std::fs::write(&second, b"second").expect("second wallpaper");

        persist_wallpaper_at(temporary.path(), "DP-1", &first).expect("persist first");
        assert_eq!(
            read_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1"),
            Some(first.clone())
        );
        assert_eq!(
            read_wallpaper_state(temporary.path(), "previous_wallpaper", "DP-1"),
            None
        );

        persist_wallpaper_at(temporary.path(), "DP-1", &second).expect("persist second");
        assert_eq!(
            read_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1"),
            Some(second)
        );
        assert_eq!(
            read_wallpaper_state(temporary.path(), "previous_wallpaper", "DP-1"),
            Some(first)
        );

        let missing = temporary.path().join("missing.jpg");
        write_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1", &missing)
            .expect("persist missing path for restore test");
        assert_eq!(
            read_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1"),
            Some(missing)
        );
    }

    #[test]
    fn wallpaper_state_preserves_non_utf8_and_whitespace() {
        use std::os::unix::ffi::OsStringExt;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let wallpaper = std::path::PathBuf::from(std::ffi::OsString::from_vec(
            b" /tmp/wallpaper-\xff.jpg ".to_vec(),
        ));

        write_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1", &wallpaper)
            .expect("persist wallpaper path");

        assert_eq!(
            read_wallpaper_state(temporary.path(), "last_wallpaper", "DP-1"),
            Some(wallpaper)
        );
    }

    #[test]
    fn retries_only_explicitly_transient_errors() {
        let timed_out = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "temporary timeout",
        ));
        let missing = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing wallpaper",
        ));
        let recoverable_video = anyhow::Error::new(crate::video::VideoError::QueueFull);

        assert!(is_transient_wallpaper_error(&timed_out));
        assert!(is_transient_wallpaper_error(&recoverable_video));
        assert!(!is_transient_wallpaper_error(&missing));
        assert!(!is_transient_wallpaper_error(&anyhow::anyhow!(
            "invalid dimensions"
        )));
    }

    #[test]
    fn live_video_params_stay_bounded() {
        assert_eq!(clamp_preload_frames(0), 1);
        assert_eq!(clamp_preload_frames(2), 2);
        assert_eq!(clamp_preload_frames(100), 3);
        assert_eq!(clamp_max_fps(None), None);
        assert_eq!(clamp_max_fps(Some(0)), None);
        assert_eq!(clamp_max_fps(Some(60)), Some(60));
        assert_eq!(clamp_max_fps(Some(10_000)), Some(240));
    }

    #[test]
    fn watcher_requires_non_empty_files() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let empty = temporary.path().join("empty.jpg");
        let filled = temporary.path().join("filled.jpg");
        std::fs::write(&empty, b"").expect("empty file");
        std::fs::write(&filled, b"data").expect("filled file");
        assert!(!is_usable_wallpaper_file(&empty));
        assert!(is_usable_wallpaper_file(&filled));
        assert!(!is_usable_wallpaper_file(
            &temporary.path().join("missing.jpg")
        ));
    }

    #[test]
    fn reload_rejects_absurd_live_values() {
        let mut cfg = crate::config::WallrConfig::default();
        assert!(validate_live_config(&cfg).is_ok());
        cfg.video.preload_frames = 1000;
        assert!(validate_live_config(&cfg).is_err());
        cfg.video.preload_frames = 2;
        cfg.daemon.max_fps = Some(0);
        assert!(validate_live_config(&cfg).is_err());
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
                    tokio::spawn(async move {
                        let states = render_states.lock().await;
                        if let Some(rs) = states.get(&name) {
                            let mut lock = rs.lock().await;
                            lock.width = new_width;
                            lock.height = new_height;
                            if let Some(gpu) = lock.gpu.as_ref() {
                                if let Ok(shared) = lock.renderer.lock() {
                                    if let Some(renderer) = shared.as_ref() {
                                        gpu.surface.configure(
                                            &renderer.device,
                                            &wgpu::SurfaceConfiguration {
                                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                                format: gpu.format,
                                                width: new_width,
                                                height: new_height,
                                                present_mode: wgpu::PresentMode::Fifo,
                                                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                                                view_formats: vec![],
                                                desired_maximum_frame_latency: 1,
                                            },
                                        );
                                    }
                                }
                                tracing::info!(
                                    "Hotplug: reconfigured {name} to {new_width}x{new_height}"
                                );
                            }
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
        // A notification is a real scheduling event: it means a newer
        // wallpaper may have superseded the current player.  Using
        // `wait_timeout_while` with an always-true predicate turns every
        // notify into a spurious wakeup and makes the waiter sleep until its
        // full deadline anyway (up to 24h for paused GIFs).  A plain timed
        // wait returns on notify; the caller then checks the generation and
        // exits without another decode/upload/present.
        let _ = self.cond.wait_timeout(guard, deadline - now);
    }
}

struct RenderState {
    /// GPU resources are grouped behind one ownership boundary so the next
    /// lazy-GPU step can replace this with an optional, on-demand state
    /// without changing static Wayland/shm fields.
    gpu: Option<GpuState>,
    renderer: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Renderer>>>>,
    display_ptr: SendDisplayPtr,
    raw_surface: SendDisplayPtr,
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
    /// Effect used by the active request, for idempotent switch detection.
    current_effect: Option<crate::animation::Effect>,
    /// Per-output uniform buffer + bind group (Issue #9 race fix).
    /// Path of the last wallpaper set on this output (for restore).
    last_wallpaper: Option<std::path::PathBuf>,
    /// Previous wallpaper state before blank (for restore).
    pre_blank: Option<(std::path::PathBuf, u32)>,
    /// Whether this output is currently blanked.
    blanked: bool,
    /// GIF playback paused state (shared with play_live task).
    gif_paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Shared-memory fast path for completed static wallpapers. Dynamic
    /// content and transitions continue to use wgpu; static content can drop
    /// its GPU image and become compositor-owned with no render loop.
    shm_surface: wl_surface::WlSurface,
    /// Server handle for (re)creating the pool. `Shm` itself is not Clone,
    /// but the underlying proxy is; a fresh handle is minted on demand.
    shm_server: wayland_client::protocol::wl_shm::WlShm,
    shm_pool: SlotPool,
    /// Ping-pong shm buffers. The parked buffer is usually the currently
    /// displayed one (legitimately compositor-held), so single-buffer reuse
    /// can never hit in steady use; alternating guarantees the idle buffer
    /// was released when its sibling was presented, bounding the pool.
    shm_buffers: [Option<ShmBuffer>; 2],
    shm_index: usize,
    shm_width: u32,
    shm_height: u32,
    /// A layer surface must not switch from wgpu explicit-sync commits to
    /// bufferless `wl_shm` commits. Some compositors reject that sequence
    /// with a missing acquire timeline. Keep static requests on wgpu after
    /// the first GPU presentation for this surface.
    gpu_surface_used: bool,
    /// Cached video texture for reuse across same-dimension video playback.
    /// Reusing Y/UV planes, conversion resources, and output texture avoids
    /// reallocating GPU memory on every video switch when resolution is unchanged.
    /// Wrapped in Arc so it can be shared with active playback tasks.
    cached_video_texture: Option<std::sync::Arc<crate::renderer::VideoTexture>>,
}

struct GpuState {
    renderer: std::sync::Arc<Renderer>,
    surface: &'static wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    per_output_uniforms: std::sync::Arc<crate::renderer::PerOutputUniforms>,
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
    /// Shared via Arc so the cache can retain it for reuse after playback ends.
    video_texture: Option<std::sync::Arc<crate::renderer::VideoTexture>>,
    /// Playback generation captured at commit time; live playback stops when
    /// it no longer matches `RenderState::playback_gen`.
    generation: u64,
    /// Scaling mode: 0=Fill, 1=Fit, 2=Stretch, 3=Center, 4=Tile.
    scaling_mode: u32,
    /// Optional cap for live video presentation.
    max_fps: Option<u32>,
}

impl RenderState {
    fn ensure_gpu(&mut self) -> anyhow::Result<()> {
        if self.gpu.is_some() {
            return Ok(());
        }

        let renderer = {
            let mut shared = self
                .renderer
                .lock()
                .map_err(|_| anyhow::anyhow!("renderer initialization lock poisoned"))?;
            if let Some(renderer) = shared.as_ref() {
                renderer.clone()
            } else {
                let renderer = tokio::runtime::Handle::current()
                    .block_on(Renderer::new())
                    .map_err(|e| anyhow::anyhow!("GPU init failed: {e}"))?;
                let renderer = std::sync::Arc::new(renderer);
                *shared = Some(renderer.clone());
                renderer
            }
        };

        let window_handle = WaylandWindow {
            display: self.display_ptr.0,
            surface: self.raw_surface.0,
        };
        let wgpu_surface = renderer
            .instance
            .create_surface(&window_handle)
            .map_err(|e| anyhow::anyhow!("wgpu surface creation failed: {e:?}"))?;
        let formats = wgpu_surface.get_capabilities(&renderer.adapter).formats;
        let format = formats
            .iter()
            .copied()
            .find(|format| *format == wgpu::TextureFormat::Bgra8UnormSrgb)
            .or_else(|| formats.into_iter().next())
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);
        wgpu_surface.configure(
            &renderer.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: self.width,
                height: self.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 1,
            },
        );
        // SAFETY: the Wayland display and layer surface are retained by the
        // daemon for the lifetime of this render state.
        let wgpu_surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };
        let surface: &'static wgpu::Surface<'static> = Box::leak(Box::new(wgpu_surface));
        self.gpu = Some(GpuState {
            renderer: renderer.clone(),
            surface,
            format,
            per_output_uniforms: std::sync::Arc::new(renderer.create_per_output_uniforms()),
        });
        Ok(())
    }

    fn set_wallpaper(
        &mut self,
        path: &std::path::Path,
        effect: &crate::animation::Effect,
        duration_ms: u32,
        scaling_mode: u32,
    ) -> anyhow::Result<()> {
        // Setting the already-active source is an idempotent operation. Do
        // not decode it, allocate another GPU texture, or enqueue another
        // transition merely because a watcher/IPC client repeated the same
        // request. This is especially important for bursty wallpaper
        // scripts, where the no-op path should be effectively free.
        if !self.blanked
            && (self.current_bind.is_some() || self.shm_buffers.iter().any(|b| b.is_some()))
            && self.last_wallpaper.as_deref() == Some(path)
            && self.scaling_mode == scaling_mode
            && self.current_effect.as_ref() == Some(effect)
        {
            return Ok(());
        }
        // A zero-duration static commit does not need a shader or a swapchain
        // frame. Hand the pixels directly to the compositor and release the
        // persistent GPU image, matching the low-idle-cost architecture used
        // by dedicated static wallpaper daemons.
        let is_animated_image = matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("gif" | "GIF" | "apng" | "APNG")
        );
        if duration_ms == 0
            && !is_animated_image
            && !crate::video::VideoDecoder::is_video_file(path)
            && !self.gpu_surface_used
            && self.commit_static_shm(path, scaling_mode)?
        {
            self.playback_gen.fetch_add(1, Ordering::SeqCst);
            self.pacer.notify();
            self.video_playback.stop();
            self.current_bind = None;
            self.current_tex = None;
            self.current_width = 0;
            self.current_height = 0;
            self.scaling_mode = scaling_mode;
            self.current_effect = Some(effect.clone());
            self.last_wallpaper = Some(path.to_path_buf());
            self.blanked = false;
            // Dropping the old bind group/texture is enough to release the
            // Rust-side ownership. A blocking device drain here makes every
            // static switch wait for unrelated GPU work even though the new
            // frame is already compositor-owned through wl_shm. The render
            // path polls when it actually needs synchronization.
            return Ok(());
        }
        self.ensure_gpu()?;
        let commit = self.commit_wallpaper(path, scaling_mode)?;
        self.gpu_surface_used = true;
        self.scaling_mode = scaling_mode;
        self.current_effect = Some(effect.clone());
        self.spawn_transition(commit, effect, duration_ms);
        // Update last_wallpaper after successful commit
        self.last_wallpaper = Some(path.to_path_buf());
        Ok(())
    }

    fn commit_static_shm(
        &mut self,
        path: &std::path::Path,
        scaling_mode: u32,
    ) -> anyhow::Result<bool> {
        use image::{ImageDecoder, ImageReader};

        // The wgpu transition task and the wl_shm fast path target the same
        // layer surface. Serialize the protocol commit so a static switch
        // cannot race an in-flight GPU present.
        // A transition can be parked indefinitely in FIFO present while a
        // compositor is suspended or a monitor is disabled. Never make an
        // IPC/static switch wait behind that task: fall back to the normal
        // GPU commit, which promotes the new generation and lets the stale
        // transition exit before it renders again.
        let Some(_render_guard) = self.render_lock.try_lock().ok() else {
            return Ok(false);
        };

        let decoder = ImageReader::open(path)?.into_decoder()?;
        let (source_width, source_height) = decoder.dimensions();
        Renderer::validate_static_decode(source_width, source_height, decoder.total_bytes())?;
        let image = image::DynamicImage::from_decoder(decoder)?;
        let (rgba, width, height) = crate::renderer::prepare_image_rgba(
            &image,
            self.width,
            self.height,
            scaling_mode,
            wgpu::Limits::default().max_texture_dimension_2d,
        )?;
        if width != self.width || height != self.height {
            // wl_shm buffers are displayed at their native size. The GPU path
            // remains responsible for center/tile and other non-fullscreen
            // modes that cannot be represented by one compositor buffer.
            return Ok(false);
        }

        let stride = width
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("static image stride overflow"))?;
        // Ping-pong between two buffers. The parked buffer is usually the
        // currently displayed one (legitimately compositor-held, so waiting
        // on it is futile); alternating guarantees the idle sibling was
        // released when its partner was presented, so steady-state switching
        // never allocates and the pool never grows.
        let mut used_slot = 0usize;
        let mut chosen: Option<ShmBuffer> = None;
        if self.shm_width == width && self.shm_height == height {
            for _ in 0..2 {
                let slot = self.shm_index % 2;
                self.shm_index = self.shm_index.wrapping_add(1);
                if let Some(buffer) = self.shm_buffers[slot].take() {
                    if buffer.canvas(&mut self.shm_pool).is_some() {
                        chosen = Some(buffer);
                        used_slot = slot;
                        break;
                    }
                    self.shm_buffers[slot] = Some(buffer);
                }
            }
            if chosen.is_none() {
                // Both slots busy under sustained flood. Wait briefly for a
                // release on a detached task (never the Wayland event loop).
                'wait: for _ in 0..10 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    for slot in 0..2 {
                        if let Some(buffer) = self.shm_buffers[slot].take() {
                            if buffer.canvas(&mut self.shm_pool).is_some() {
                                chosen = Some(buffer);
                                used_slot = slot;
                                break 'wait;
                            }
                            self.shm_buffers[slot] = Some(buffer);
                        }
                    }
                }
            }
        } else {
            // Output size changed: drop both parked buffers (rare path).
            self.shm_buffers = [None, None];
        }
        let buffer = match chosen {
            Some(buffer) => buffer,
            None => {
                // No free buffer. Never grow the pool beyond the pair:
                // pool growth is process-lifetime high-water billed to
                // post-workload RSS (measured ~390 MiB retained after 90
                // rapid switches with unbounded growth).
                if self.shm_width != width || self.shm_height != height {
                    // Output size changed: reset the pair (rare path).
                    self.shm_buffers = [None, None];
                    let (new_buffer, _) = self.shm_pool.create_buffer(
                        width as i32,
                        height as i32,
                        stride as i32,
                        wayland_client::protocol::wl_shm::Format::Xrgb8888,
                    )?;
                    used_slot = 0;
                    new_buffer
                } else if let Some(slot) = self.shm_buffers.iter().position(|slot| slot.is_none()) {
                    // Priming the pair: still bounded at two buffers.
                    let (new_buffer, _) = self.shm_pool.create_buffer(
                        width as i32,
                        height as i32,
                        stride as i32,
                        wayland_client::protocol::wl_shm::Format::Xrgb8888,
                    )?;
                    used_slot = slot;
                    new_buffer
                } else {
                    // Both slots busy: the compositor is more than a frame
                    // behind under flood. Fall back to the GPU path instead
                    // of growing the pool; it promotes its own texture and
                    // a later commit reuses a released slot.
                    return Ok(false);
                }
            }
        };
        let canvas = buffer
            .canvas(&mut self.shm_pool)
            .ok_or_else(|| anyhow::anyhow!("shared-memory wallpaper buffer is still active"))?;
        for (src, dst) in rgba
            .as_raw()
            .chunks_exact(4)
            .zip(canvas.chunks_exact_mut(4))
        {
            // XRGB8888 is stored as B,G,R,X on little-endian Wayland hosts.
            // Write channels directly so the hot conversion loop does not
            // construct a temporary slice for every pixel.
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = 0xff;
        }
        buffer
            .attach_to(&self.shm_surface)
            .map_err(|e| anyhow::anyhow!(e))?;
        self.shm_surface
            .damage_buffer(0, 0, width as i32, height as i32);
        self.shm_surface.commit();
        self.shm_buffers[used_slot] = Some(buffer);
        self.shm_width = width;
        self.shm_height = height;
        Ok(true)
    }

    /// Loads the new wallpaper and atomically promotes it to the current
    /// frame. The outgoing bind group stays alive for the transition, so the
    /// render task can keep drawing from it after this commit returns.
    fn commit_wallpaper(
        &mut self,
        path: &std::path::Path,
        scaling_mode: u32,
    ) -> anyhow::Result<CommitData> {
        use image::{ImageDecoder, ImageReader};
        let gpu = self
            .gpu
            .as_ref()
            .expect("GPU state required for dynamic content");

        // Check if this is a video file FIRST
        if crate::video::VideoDecoder::is_video_file(path) {
            tracing::info!("Video file detected: {:?}", path);

            let generation = self.playback_gen.load(Ordering::SeqCst).wrapping_add(1);
            let renderer = gpu.renderer.clone();
            // Prepare and validate the new decoder before replacing active
            // playback. A failed video therefore leaves the old wallpaper and
            // decoder untouched.
            let mut prepared = crate::video::VideoPlayback::prepare(
                path,
                self.hw_accel,
                self.preload_frames,
                std::time::Duration::from_millis(1000),
                move |metadata| {
                    renderer
                        .validate_video_texture(metadata.width, metadata.height)
                        .map_err(crate::video::VideoError::GpuResourceCreation)
                },
            )?;
            let metadata = prepared.metadata().clone();
            let first_frame = prepared.take_first_frame();

            let (tex_width, tex_height) = first_frame
                .as_ref()
                .map(|frame| (frame.width, frame.height))
                .unwrap_or((metadata.width, metadata.height));

            // Reuse the cached video texture if dimensions match, avoiding
            // Y/UV/output texture reallocation when switching between same-resolution videos.
            let video_texture = if let Some(cached) = self.cached_video_texture.as_ref()
                && cached.matches_dimensions(tex_width, tex_height)
            {
                tracing::debug!("Reusing video texture for {}x{}", tex_width, tex_height);
                // Keep the Arc in the cache; clone for CommitData so both hold a reference
                cached.clone()
            } else {
                if let Some(previous) = self.cached_video_texture.as_ref() {
                    tracing::debug!(
                        "Video resolution changed, recreating texture: {}x{} -> {}x{}",
                        previous.width(),
                        previous.height(),
                        tex_width,
                        tex_height
                    );
                }
                let new_texture =
                    std::sync::Arc::new(gpu.renderer.create_video_texture(tex_width, tex_height)?);
                // Store in cache for future reuse
                self.cached_video_texture = Some(new_texture.clone());
                new_texture
            };
            let (img_width, img_height) = if let Some(frame) = first_frame {
                gpu.renderer
                    .update_video_texture(&video_texture, &frame.data)?;
                (frame.width, frame.height)
            } else {
                // WebGPU initializes the output to black when no frame arrives.
                tracing::warn!("No first frame available, using black texture");
                (metadata.width, metadata.height)
            };
            self.video_playback.commit(prepared, generation);
            self.playback_gen.store(generation, Ordering::SeqCst);
            self.pacer.notify();
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
                format: gpu.format,
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

        // Stream animated frames (GIF) on demand during playback; the
        // transition's incoming texture is the GIF's first frame.
        let mut animated = crate::animated::AnimatedImage::decode(path)?;
        let (new_tex, new_bind, img_width, img_height) = if let Some(anim) = animated.as_mut() {
            let (w, h) = (anim.width, anim.height);
            let (tex, bind) = gpu.renderer.create_texture(w, h)?;
            let first = anim.first_frame();
            if !first.is_empty() {
                gpu.renderer.update_texture(&tex, first, w, h);
            }
            (tex, bind, w, h)
        } else {
            let decoder = ImageReader::open(path)?.into_decoder()?;
            let (source_width, source_height) = decoder.dimensions();
            Renderer::validate_static_decode(source_width, source_height, decoder.total_bytes())?;
            let new_img = image::DynamicImage::from_decoder(decoder)?;
            let (tex, bind, width, height) =
                gpu.renderer
                    .load_texture(&new_img, self.width, self.height, scaling_mode)?;
            (tex, bind, width, height)
        };

        // Only supersede active video playback after the replacement has
        // decoded and allocated successfully.
        self.video_playback.stop();
        // Release cached video texture when switching away from video.
        // Static and GIF wallpapers don't need NV12 conversion resources.
        self.cached_video_texture = None;

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
            format: gpu.format,
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
        let gpu = self
            .gpu
            .as_ref()
            .expect("GPU state required for transitions");
        let renderer = gpu.renderer.clone();
        let surface: &'static wgpu::Surface<'static> = gpu.surface;
        let render_lock = self.render_lock.clone();
        let playback_gen = self.playback_gen.clone();
        let pacer = self.pacer.clone();
        let video_playback = self.video_playback.clone();
        let per_output_uniforms = std::sync::Arc::clone(&gpu.per_output_uniforms);
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
    let state_root = wallpaper_state_root();
    let Some(path) = read_wallpaper_state(&state_root, "last_wallpaper", name) else {
        return;
    };

    let effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
    // Restoring a cached static wallpaper does not need an entrance
    // transition. Avoid decoding/uploading a temporary GPU frame at daemon
    // startup; static images can settle directly through the zero-duration
    // path, while GIF/video inputs still select their dynamic pipeline.
    if let Err(err) = set_wallpaper_with_retry(render_state, &path, &effect, 0, 0).await {
        tracing::warn!("Failed to restore wallpaper for {name} from {path:?}: {err}");
        let Some(previous) = read_wallpaper_state(&state_root, "previous_wallpaper", name) else {
            return;
        };
        match set_wallpaper_with_retry(render_state, &previous, &effect, 0, 0).await {
            Ok(()) => {
                if let Err(persist_err) =
                    write_wallpaper_state(&state_root, "last_wallpaper", name, &previous)
                {
                    tracing::warn!(
                        "Restored previous wallpaper for {name}, but failed to update state: {persist_err}"
                    );
                } else {
                    tracing::warn!("Restored previous wallpaper for {name} after {path:?} failed");
                }
            }
            Err(previous_err) => tracing::warn!(
                "Failed to restore previous wallpaper for {name} from {previous:?}: {previous_err}"
            ),
        }
    }
}

const WALLPAPER_RETRY_ATTEMPTS: usize = 3;
const WALLPAPER_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(150);
/// `SlotPool` grows automatically when a buffer does not fit. Keep daemon
/// startup cheap instead of reserving a full output-sized shm mapping before
/// the first static wallpaper is requested.
const INITIAL_SHM_POOL_BYTES: usize = 4096;

async fn set_wallpaper_with_retry(
    render_state: &Arc<Mutex<RenderState>>,
    path: &std::path::Path,
    effect: &crate::animation::Effect,
    duration_ms: u32,
    scaling_mode: u32,
) -> anyhow::Result<()> {
    let render_state = Arc::clone(render_state);
    let path = path.to_path_buf();
    let effect = effect.clone();

    tokio::task::spawn_blocking(move || {
        let mut attempt = 1;
        loop {
            let result = {
                let mut state = render_state.blocking_lock();
                state.set_wallpaper(&path, &effect, duration_ms, scaling_mode)
            };
            match result {
                Ok(()) => return Ok(()),
                Err(err)
                    if attempt < WALLPAPER_RETRY_ATTEMPTS
                        && is_transient_wallpaper_error(&err) =>
                {
                    tracing::warn!(
                        "Transient wallpaper error for {path:?} (attempt {attempt}/{WALLPAPER_RETRY_ATTEMPTS}): {err}"
                    );
                    attempt += 1;
                    std::thread::sleep(WALLPAPER_RETRY_DELAY);
                }
                Err(err) => return Err(err),
            }
        }
    })
    .await?
}

fn is_transient_wallpaper_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| {
                matches!(
                    io_err.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                )
            })
            || cause
                .downcast_ref::<crate::video::VideoError>()
                .is_some_and(crate::video::VideoError::is_recoverable)
    })
}

fn wallpaper_state_root() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("wallr")
}

fn wallpaper_state_path(root: &std::path::Path, state_dir: &str, name: &str) -> std::path::PathBuf {
    root.join(state_dir).join(name)
}

fn read_wallpaper_state(
    root: &std::path::Path,
    state_dir: &str,
    name: &str,
) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let path = std::fs::read(wallpaper_state_path(root, state_dir, name)).ok()?;
    let wallpaper = std::path::PathBuf::from(std::ffi::OsString::from_vec(path));
    (!wallpaper.as_os_str().is_empty()).then_some(wallpaper)
}

fn write_wallpaper_state(
    root: &std::path::Path,
    state_dir: &str,
    name: &str,
    wallpaper: &std::path::Path,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let state_path = wallpaper_state_path(root, state_dir, name);
    let parent = state_path
        .parent()
        .ok_or_else(|| std::io::Error::other("wallpaper state path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = state_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("wallpaper");

    loop {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), id));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(mut temporary) => {
                if let Err(err) = temporary
                    .write_all(wallpaper.as_os_str().as_bytes())
                    .and_then(|()| temporary.sync_all())
                    .and_then(|()| std::fs::rename(&temporary_path, &state_path))
                {
                    let _ = std::fs::remove_file(&temporary_path);
                    return Err(err);
                }
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

fn persist_wallpaper(name: &str, wallpaper: &std::path::Path) -> std::io::Result<()> {
    let root = wallpaper_state_root();
    persist_wallpaper_at(&root, name, wallpaper)
}

fn persist_wallpaper_at(
    root: &std::path::Path,
    name: &str,
    wallpaper: &std::path::Path,
) -> std::io::Result<()> {
    let current = read_wallpaper_state(root, "last_wallpaper", name);
    // Hot-path no-op: a repeated set of the already-active wallpaper (bursty
    // scripts, benchmark loops) must not pay two temp-file writes, two
    // fsyncs, and two renames per call. The on-disk state is already correct,
    // and fsync latency spikes are the dominant jitter source on this path.
    if current.as_deref() == Some(wallpaper) {
        return Ok(());
    }
    let previous = current.filter(|current| current.exists() && current != wallpaper);
    write_wallpaper_state(root, "last_wallpaper", name, wallpaper)?;
    if let Some(previous) = previous {
        write_wallpaper_state(root, "previous_wallpaper", name, &previous)?;
    }
    Ok(())
}

/// Presents one frame per vsync until the wall-clock duration elapses. With
/// PresentMode::Fifo, `get_current_texture` blocks until the previous frame
/// is presented, so this loop is paced to the monitor refresh rate, and the
/// transition lasts exactly `duration_ms` on any refresh rate - frame-count
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
    // Do this check before taking the render lock. Rapid IPC updates can
    // otherwise queue several detached tasks, each retaining a complete
    // incoming/outgoing GPU resource pair while waiting for its turn.
    if playback_gen.load(Ordering::Acquire) != commit.generation {
        reclaim_commit_resources(&renderer, commit);
        return;
    }
    let _guard = render_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let duration = std::time::Duration::from_millis(u64::from(duration_ms.max(1)));
    let start = std::time::Instant::now();
    loop {
        // A newer commit owns the output now. Abort before doing any more
        // CPU/GPU work so superseded transitions release their bind groups,
        // textures, and task stack immediately instead of queueing behind the
        // render lock for the full transition duration.
        if playback_gen.load(Ordering::Acquire) != commit.generation {
            reclaim_commit_resources(&renderer, commit);
            return;
        }
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
        if progress >= 1.0 {
            break;
        }
        match status {
            crate::renderer::FrameStatus::Presented => {}
            // A stalled compositor parks inside `get_current_texture`; a
            // timeout just means "try the next vsync" without breaking the
            // wall-clock duration. Yield briefly so a struggling compositor
            // is not hammered with back-to-back acquires.
            crate::renderer::FrameStatus::TimedOut => {
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
            // The swapchain is stale (resize, scale, recreation). Reconfigure
            // once and continue the transition instead of blanking.
            crate::renderer::FrameStatus::Outdated | crate::renderer::FrameStatus::Lost => {
                surface.configure(
                    &renderer.device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: commit.format,
                        width: commit.width.max(1),
                        height: commit.height.max(1),
                        present_mode: wgpu::PresentMode::Fifo,
                        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 1,
                    },
                );
            }
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
    // Ensure resources from a completed static transition, or a live player
    // that just stopped, are handed back to the backend before the detached
    // task disappears. This is intentionally outside the frame hot path.
    reclaim_commit_resources(&renderer, commit);
}

fn reclaim_commit_resources(renderer: &Renderer, commit: CommitData) {
    drop(commit);
    renderer.device.poll(wgpu::Maintain::Wait);
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
    let Ok((tex_a, bind_a)) = renderer.create_texture(animated.width, animated.height) else {
        tracing::warn!(
            "GIF dimensions {}x{} exceed the GPU texture limit",
            animated.width,
            animated.height
        );
        return;
    };
    let Ok((tex_b, bind_b)) = renderer.create_texture(animated.width, animated.height) else {
        tracing::warn!(
            "GIF dimensions {}x{} exceed the GPU texture limit",
            animated.width,
            animated.height
        );
        return;
    };
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
            // The compositor retains the last committed buffer. Re-presenting
            // an identical frame while paused only burns CPU/GPU time and
            // wakes laptops needlessly. Park until resume or a replacement
            // wallpaper notifies the shared pacer.
            let pause_start = std::time::Instant::now();
            pacer.wait_until(std::time::Instant::now() + std::time::Duration::from_secs(86_400));
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
            // A timeout is transient compositor backpressure, not a reason
            // to stop the animation: yield briefly and keep presenting.
            // (Previously any non-present status killed GIF playback.)
            Ok(crate::renderer::FrameStatus::TimedOut) => {
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
            Ok(crate::renderer::FrameStatus::Outdated | crate::renderer::FrameStatus::Lost) => {
                surface.configure(
                    &renderer.device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: commit.format,
                        width: commit.width.max(1),
                        height: commit.height.max(1),
                        present_mode: wgpu::PresentMode::Fifo,
                        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 1,
                    },
                );
            }
            Err(err) => {
                tracing::warn!("GIF present failed ({err}), stopping playback");
                return;
            }
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
    let mut width = commit.img_width;
    let mut height = commit.img_height;

    let Some(initial) = commit.video_texture.as_ref() else {
        tracing::warn!("Video conversion resources unavailable");
        return;
    };
    // Task-local texture handle. Unchanged resolution reuses the commit's
    // GPU resources every frame (upload + NV12 conversion only); only a
    // genuine mid-stream resolution change recreates them.
    let mut owned_replacement: Option<crate::renderer::VideoTexture> = None;
    let mut texture: &crate::renderer::VideoTexture = initial.as_ref();
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());

    let min_frame_interval = commit
        .max_fps
        .filter(|fps| *fps > 0)
        .map(|fps| std::time::Duration::from_secs_f64(1.0 / f64::from(fps)));
    let mut last_present: Option<std::time::Instant> = None;

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
            // Mid-stream resolution change (rare): recreate only the video
            // conversion resources instead of dropping every subsequent frame.
            if frame.width != width || frame.height != height {
                match renderer.create_video_texture(frame.width, frame.height) {
                    Ok(replacement) => {
                        tracing::info!(
                            "Video resolution changed {}x{} -> {}x{}, recreated conversion resources",
                            width,
                            height,
                            frame.width,
                            frame.height
                        );
                        width = frame.width;
                        height = frame.height;
                        // Drop any previous mid-stream replacement before
                        // installing the new one so only one spare texture
                        // is ever alive in this task.
                        owned_replacement.take();
                        owned_replacement = Some(replacement);
                        texture = owned_replacement.as_ref().expect("just stored");
                    }
                    Err(err) => {
                        tracing::warn!("Video resolution change rejected: {err}");
                        pacer.wait_until(
                            std::time::Instant::now() + std::time::Duration::from_millis(2),
                        );
                        continue;
                    }
                }
            }
            if let Err(err) = renderer.update_video_texture(texture, &frame.data) {
                tracing::warn!("Video frame upload failed: {err}");
                return;
            }
            // `frame` owns the only copy of the decoded planes; it drops here
            // after GPU upload instead of lingering in any queue or cache.
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
            Ok(VideoPresentAction::Presented) => {
                last_present = Some(std::time::Instant::now());
            }
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
                            desired_maximum_frame_latency: 1,
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

fn resolve_named_targets(
    render_states: &std::collections::HashMap<
        String,
        std::sync::Arc<tokio::sync::Mutex<RenderState>>,
    >,
    monitor: Option<&str>,
) -> Vec<(String, std::sync::Arc<tokio::sync::Mutex<RenderState>>)> {
    match monitor {
        Some(name) => render_states
            .get(name)
            .map(|state| vec![(name.to_string(), state.clone())])
            .unwrap_or_default(),
        None => render_states
            .iter()
            .map(|(name, state)| (name.clone(), state.clone()))
            .collect(),
    }
}

type RenderStateMap = std::collections::HashMap<String, Arc<Mutex<RenderState>>>;
type SharedRenderStates = std::sync::Arc<tokio::sync::Mutex<RenderStateMap>>;

/// Snapshot target states while holding the map lock briefly, then drop the
/// guard before any decode, GPU, or theme work. Holding the map lock across
/// long operations would block hotplug insert/remove and other IPC commands.
async fn snapshot_targets(
    map: &SharedRenderStates,
    monitor: Option<&str>,
) -> Vec<Arc<Mutex<RenderState>>> {
    let states = map.lock().await;
    resolve_targets(&states, monitor).await
}

async fn snapshot_named_targets(
    map: &SharedRenderStates,
    monitor: Option<&str>,
) -> Vec<(String, Arc<Mutex<RenderState>>)> {
    let states = map.lock().await;
    resolve_named_targets(&states, monitor)
}

async fn snapshot_all_named(map: &SharedRenderStates) -> Vec<(String, Arc<Mutex<RenderState>>)> {
    let states = map.lock().await;
    states
        .iter()
        .map(|(name, state)| (name.clone(), state.clone()))
        .collect()
}

/// Returns settled outputs' shared-memory pools to the OS.
///
/// A committed shm buffer stays mapped as long as its pool lives, which
/// otherwise pins a full output-sized mapping per output forever (plus any
/// flood growth). Once nothing is drawing, the compositor already owns the
/// displayed pixels, so the pool and both ping-pong buffers can go: the
/// next static set recreates a fresh minimal pool. Settled means no set in
/// flight (state guard free) and no live transition, video, or GIF (those
/// hold the render lock for their whole run). One async task, no OS thread.
async fn reap_idle_shm_pools(states: SharedRenderStates) {
    loop {
        // One-second cadence: cheap (a few non-blocking try_locks when
        // idle) and guarantees a quiescent pool is returned well before
        // any multi-second idle sampling window observes it.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        for (_, state) in snapshot_all_named(&states).await {
            let Ok(mut locked) = state.try_lock() else {
                continue;
            };
            if locked.render_lock.try_lock().is_err() {
                continue;
            }
            if locked.shm_pool.len() > INITIAL_SHM_POOL_BYTES {
                locked.shm_buffers = [None, None];
                locked.shm_width = 0;
                locked.shm_height = 0;
                let fresh = SlotPool::new(
                    INITIAL_SHM_POOL_BYTES,
                    &smithay_client_toolkit::shm::Shm::from(locked.shm_server.clone()),
                );
                match fresh {
                    Ok(pool) => locked.shm_pool = pool,
                    Err(_) => continue,
                }
            }
            // Quiescent heap: hand freed top pages back as well. Only free
            // pages move, so live allocations are unaffected.
            // SAFETY: malloc_trim only releases unallocated pages back to
            // the OS; it cannot invalidate live pointers. The return value
            // (whether anything was released) is intentionally ignored.
            unsafe {
                libc::malloc_trim(0);
            }
        }
    }
}

/// Caps for live-applied configuration. Keeps queues bounded and prevents
/// absurd scheduling values from IPC or config reload. Wallpaper playback
/// needs at most ~3 decoded frames in flight (bounded queue + one pending);
/// a larger backlog only retains obsolete frames the display can never show.
pub(crate) fn clamp_preload_frames(requested: usize) -> usize {
    requested.clamp(1, 3)
}

pub(crate) fn clamp_max_fps(requested: Option<u32>) -> Option<u32> {
    match requested {
        Some(0) | None => None,
        Some(fps) => Some(fps.clamp(1, 240)),
    }
}

/// Returns true when `path` looks like a usable wallpaper file: exists, is a
/// file, and has non-zero size. Used by IPC to avoid decoding
/// partially-written files.
fn is_usable_wallpaper_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Validates a freshly loaded config before it replaces the live one.
/// Only checks values applied at runtime; unknown future keys are ignored by
/// serde defaults and never fail reload.
fn validate_live_config(cfg: &WallrConfig) -> Result<(), String> {
    if cfg.video.preload_frames > 64 {
        return Err(format!(
            "video.preload_frames {} exceeds maximum 64",
            cfg.video.preload_frames
        ));
    }
    if let Some(fps) = cfg.daemon.max_fps
        && (fps == 0 || fps > 1000)
    {
        return Err(format!("daemon.max_fps {fps} out of range 1..=1000"));
    }
    Ok(())
}

/// Tune glibc malloc for a wallpaper daemon's bursty allocation pattern.
/// Animated/video decode issues thousands of transient multi-MB buffers
/// (GIF canvases, zstd scratch, NV12 planes). Under that churn glibc's
/// default *dynamic* mmap threshold adapts upward, after which freed blocks
/// are retained in per-thread arenas instead of being munmap'd: measured
/// 78 MiB stuck after GIF→static with defaults vs 25 MiB with a fixed
/// 128 KiB threshold (identical workload, `MALLOC_MMAP_THRESHOLD_` experiment).
/// A fixed threshold keeps transient decode buffers on mmap (returned to the
/// OS on free); a small trim threshold returns sbrk heap promptly.
#[cfg(target_os = "linux")]
fn tune_allocator() {
    // SAFETY: mallopt with valid M_* parameters only adjusts allocator
    // thresholds; it cannot invalidate live pointers. A 0 return merely
    // keeps glibc defaults.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 64 * 1024);
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
        tune_allocator();
        let socket_path = crate::config::expand_path(&self.config.daemon.socket);
        if socket_path.exists() {
            if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                return Err(DaemonError::AlreadyRunning(
                    socket_path.to_string_lossy().to_string(),
                ));
            }
            let _ = std::fs::remove_file(&socket_path);
        }

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

        let renderer: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Renderer>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));

        // Create a LayerSurface, wgpu Surface, and RenderState for every
        // known output. The key is the output's human-readable name (e.g.
        // "DP-1", "eDP-1") so IPC can target a specific monitor.
        let mut render_states_map: std::collections::HashMap<String, Arc<Mutex<RenderState>>> =
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
            render_states_map.insert(name.clone(), rs.clone());
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
        // Return settled outputs' shm pools to the OS (see reap_idle_shm_pools).
        tokio::spawn(reap_idle_shm_pools(render_states.clone()));
        // Generation counter for theme/hook work. Rapid A→B→C switches bump
        // the counter; detached theme tasks with a stale generation exit
        // without invoking external processes for obsolete wallpapers.
        let theme_gen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let theme_gen_clone = theme_gen.clone();

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
            let render_states_map = render_states_clone.clone();
            let theme_gen = theme_gen_clone.clone();
            let stop_socket = ipc_socket_path.clone();
            async move {
                match cmd {
                    IpcCommand::Pause { monitor } => {
                        let targets =
                            snapshot_targets(&render_states_map, monitor.as_deref()).await;
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
                        let targets =
                            snapshot_targets(&render_states_map, monitor.as_deref()).await;
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
                            rs_lock.pacer.notify();
                        }
                        IpcResponse {
                            success: true,
                            message: Some("Resumed".into()),
                        }
                    }
                    IpcCommand::Reload => {
                        // Re-read config from disk, validate, and apply live
                        // values without rebuilding GPU state. Invalid configs
                        // keep the previous valid configuration.
                        let fresh = crate::config::load_config(None)
                            .map_err(|e| e.to_string())
                            .and_then(|cfg| {
                                validate_live_config(&cfg)?;
                                Ok(cfg)
                            });
                        let fresh = match fresh {
                            Ok(cfg) => cfg,
                            Err(reason) => {
                                return IpcResponse {
                                    success: false,
                                    message: Some(format!("Reload rejected: {reason}")),
                                };
                            }
                        };
                        let preload = clamp_preload_frames(fresh.video.preload_frames);
                        let max_fps = clamp_max_fps(fresh.daemon.max_fps);
                        let hw_accel =
                            crate::video::HwAccel::from_config(&fresh.video.hw_decode);
                        {
                            let mut eng = engine.lock().await;
                            eng.config = fresh.clone();
                            if let Err(e) = eng.reload() {
                                return IpcResponse {
                                    success: false,
                                    message: Some(e.to_string()),
                                };
                            }
                        }
                        // Apply video params per output without holding the
                        // map lock across awaits.
                        let targets = snapshot_all_named(&render_states_map).await;
                        for (_, rs) in targets {
                            let mut rs_lock = rs.lock().await;
                            rs_lock.hw_accel = hw_accel;
                            rs_lock.preload_frames = preload;
                            rs_lock.max_fps = max_fps;
                        }
                        let _ = theme_gen;
                        IpcResponse {
                            success: true,
                            message: Some("Reloaded".into()),
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
                        if !is_usable_wallpaper_file(&p) {
                            return IpcResponse {
                                success: false,
                                message: Some(format!("File not found or unreadable: {}", path)),
                            };
                        }

                        // Snapshot targets first, then drop the map lock
                        // before any decode/upload work so hotplug stays
                        // responsive while a switch is in flight.
                        let targets =
                            snapshot_named_targets(&render_states_map, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }

                        let effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        // Live playback only starts after the transition, so for
                        // videos an unrequested 2s fade reads as a long "load".
                        // Default to a short fade unless the user asked for one.
                        let is_video = crate::video::VideoDecoder::is_video_file(&p);
                        let duration = duration_ms
                            .unwrap_or(if is_video { 150 } else { 2000 })
                            .min(crate::ipc::MAX_DURATION_MS);
                        let sm = scaling_mode.unwrap_or(crate::config::ScalingMode::Fill);
                        let scaling_mode_u32 = match sm {
                            crate::config::ScalingMode::Fill => 0u32,
                            crate::config::ScalingMode::Fit => 1,
                            crate::config::ScalingMode::Stretch => 2,
                            crate::config::ScalingMode::Center => 3,
                            crate::config::ScalingMode::Tile => 4,
                        };

                        let mut last_err = None;
                        for (name, rs) in &targets {
                            let result = set_wallpaper_with_retry(
                                rs,
                                &p,
                                &effect,
                                duration,
                                scaling_mode_u32,
                            )
                            .await;

                            match result {
                                Err(e) => last_err = Some(format!("Render failed: {e}")),
                                Ok(()) => {
                                    if let Err(err) = persist_wallpaper(name, &p) {
                                        tracing::warn!(
                                            "Wallpaper changed on {name}, but state persistence failed: {err}"
                                        );
                                    }
                                }
                            }
                        }

                        match last_err {
                            Some(e) => IpcResponse {
                                success: false,
                                message: Some(e),
                            },
                            None => {
                                // Wallpaper is already visible at this point.
                                // Theme/hooks/reload run detached so they never
                                // block the next switch. Rapid A→B→C switches
                                // supersede queued theme work via the
                                // generation counter.
                                let generation =
                                    theme_gen.fetch_add(1, Ordering::SeqCst) + 1;
                                let opts = SetOptions {
                                    no_theme,
                                    theme_provider: theme_override,
                                    monitor,
                                };
                                let eng = engine.clone();
                                let theme_gen_check = theme_gen.clone();
                                let p_clone = p.clone();
                                tokio::spawn(async move {
                                    if theme_gen_check.load(Ordering::SeqCst) != generation {
                                        return;
                                    }
                                    let mut eng = eng.lock().await;
                                    if theme_gen_check.load(Ordering::SeqCst) != generation {
                                        return;
                                    }
                                    if let Err(e) =
                                        eng.set_wallpaper(&p_clone, &opts).await
                                    {
                                        tracing::warn!(
                                            "Post-render hooks/theme failed for {p_clone:?}: {e}"
                                        );
                                    }
                                });
                                IpcResponse {
                                    success: true,
                                    message: None,
                                }
                            }
                        }
                    }
                    IpcCommand::Stop => {
                        let targets = snapshot_all_named(&render_states_map).await;
                        for (_, rs) in targets {
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
                        let targets =
                            snapshot_named_targets(&render_states_map, monitor.as_deref()).await;
                        if targets.is_empty() {
                            return IpcResponse {
                                success: false,
                                message: match &monitor {
                                    Some(name) => Some(format!("Unknown monitor: {name}")),
                                    None => Some("No outputs available".into()),
                                },
                            };
                        }
                        // When monitor is unspecified, seek all outputs.
                        // The map lock was already dropped; work only on the
                        // snapshot so hotplug stays responsive.
                        let mut seek_count = 0u32;
                        let mut errors = Vec::new();
                        for (name, rs) in &targets {
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
                        let targets =
                            snapshot_named_targets(&render_states_map, monitor.as_deref()).await;
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
                            format!("Outputs: {}", targets.len()),
                        ];
                        for (name, _) in &targets {
                            lines.push(format!("  - {name}"));
                        }

                        // Collect target output info
                        for (name, rs) in &targets {
                            let rs_lock = rs.lock().await;
                            // GPU state is lazy: a static-only output may never
                            // have initialized it. Report that instead of
                            // panicking the IPC handler task.
                            let gpu_info = rs_lock.gpu.as_ref().map_or_else(
                                || "GPU: not initialized (static content)".to_string(),
                                |gpu| {
                                    crate::video::gpu::adapter_diagnostics(&gpu.renderer.adapter)
                                },
                            );

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
                        let targets = snapshot_all_named(&render_states_map).await;
                        let mut lines = Vec::new();
                        for (name, rs) in &targets {
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
                        let targets = snapshot_all_named(&render_states_map).await;
                        if let Some((name, rs)) = targets.first() {
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
                        let targets =
                            snapshot_named_targets(&render_states_map, monitor.as_deref()).await;
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
                        let mut errors = Vec::new();
                        let black_effect = effect.unwrap_or_else(|| {
                            crate::animation::Effect::Fade(crate::animation::FadeParams::default())
                        });
                        let duration = duration_ms.unwrap_or(800);
                        let mut blank_file = match tempfile::Builder::new()
                            .prefix("wallr_blank-")
                            .suffix(".png")
                            .tempfile()
                        {
                            Ok(file) => file,
                            Err(e) => {
                                return IpcResponse {
                                    success: false,
                                    message: Some(format!("Failed to create blank image: {e}")),
                                };
                            }
                        };
                        let blank_path = blank_file.path().to_path_buf();
                        let blank = image::DynamicImage::ImageRgba8(
                            image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255])),
                        );
                        if let Err(e) = blank
                            .write_to(blank_file.as_file_mut(), image::ImageFormat::Png)
                        {
                            return IpcResponse {
                                success: false,
                                message: Some(format!("Failed to write blank image: {e}")),
                            };
                        }

                        for (name, rs) in &targets {
                            let rs = Arc::clone(rs);
                            let blank_path = blank_path.clone();
                            let black_effect = black_effect.clone();
                            let blanked = tokio::task::spawn_blocking(move || {
                                let mut lock = rs.blocking_lock();
                                if lock.blanked {
                                    return Ok(false);
                                }
                                let previous = (
                                    lock.last_wallpaper.clone().unwrap_or_default(),
                                    lock.scaling_mode,
                                );
                                lock.set_wallpaper(&blank_path, &black_effect, duration, 0)?;
                                lock.pre_blank = Some(previous);
                                lock.blanked = true;
                                Ok::<bool, anyhow::Error>(true)
                            })
                            .await;
                            match blanked {
                                Ok(Ok(true)) => blanked_count += 1,
                                Ok(Ok(false)) => {}
                                Ok(Err(e)) => errors.push(format!("{name}: blank failed: {e}")),
                                Err(e) => errors.push(format!("{name}: blank task failed: {e}")),
                            }
                        }
                        if errors.is_empty() {
                            IpcResponse {
                                success: true,
                                message: Some(format!("Blanked {blanked_count} output(s)")),
                            }
                        } else {
                            IpcResponse {
                                success: false,
                                message: Some(format!(
                                    "Blanked {blanked_count} output(s), {} error(s): {}",
                                    errors.len(),
                                    errors.join("; ")
                                )),
                            }
                        }
                    }
                    IpcCommand::Restore {
                        monitor,
                        effect,
                        duration_ms,
                    } => {
                        let targets =
                            snapshot_named_targets(&render_states_map, monitor.as_deref()).await;
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
                        let duration = duration_ms.unwrap_or(800).min(crate::ipc::MAX_DURATION_MS);

                        for (name, rs) in &targets {
                            let rs = Arc::clone(rs);
                            let restore_effect = restore_effect.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                let mut lock = rs.blocking_lock();
                                if !lock.blanked {
                                    return None;
                                }
                                let result = match lock.pre_blank.clone() {
                                    Some((path, scaling_mode)) if path.exists() => lock
                                        .set_wallpaper(
                                            &path,
                                            &restore_effect,
                                            duration,
                                            scaling_mode,
                                        )
                                        .map_err(|e| format!("restore failed: {e}")),
                                    Some(_) => Err("wallpaper path no longer exists".to_string()),
                                    None => Err("no previous wallpaper to restore".to_string()),
                                };
                                if result.is_ok() {
                                    lock.blanked = false;
                                    lock.pre_blank = None;
                                }
                                Some(result)
                            })
                            .await;
                            match result {
                                Ok(Some(Ok(()))) => restored_count += 1,
                                Ok(Some(Err(e))) => errors.push(format!("{name}: {e}")),
                                Ok(None) => {}
                                Err(e) => errors.push(format!("{name}: restore task failed: {e}")),
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

    /// Creates a LayerSurface, wgpu Surface, and RenderState for a single
    /// Wayland output.
    #[allow(clippy::too_many_arguments)]
    async fn create_render_state_for_output(
        renderer: &std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Renderer>>>>,
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
        let shm_surface = layer_surface.wl_surface().clone();
        wayland_state.surfaces.push((output_id, layer_surface));

        Ok(RenderState {
            gpu: None,
            renderer: renderer.clone(),
            display_ptr: SendDisplayPtr(display_ptr),
            raw_surface: SendDisplayPtr(raw_surface),
            render_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            playback_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pacer: std::sync::Arc::new(LivePacer::new()),
            current_bind: None,
            current_tex: None,
            width,
            height,
            current_width: 0,
            current_height: 0,
            video_playback: std::sync::Arc::new(crate::video::VideoPlayback::new()),
            hw_accel: crate::video::HwAccel::from_config(&config.video.hw_decode),
            preload_frames: clamp_preload_frames(config.video.preload_frames),
            max_fps: clamp_max_fps(config.daemon.max_fps),
            scaling_mode: 0,
            current_effect: None,
            last_wallpaper: None,
            pre_blank: None,
            blanked: false,
            gif_paused: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shm_surface,
            shm_server: wayland_state.shm.wl_shm().clone(),
            shm_pool: SlotPool::new(INITIAL_SHM_POOL_BYTES, &wayland_state.shm).map_err(|e| {
                DaemonError::StartError(format!("shared-memory pool creation failed: {e}"))
            })?,
            shm_buffers: [None, None],
            shm_index: 0,
            shm_width: 0,
            shm_height: 0,
            gpu_surface_used: false,
            cached_video_texture: None,
        })
    }
}

/// Synchronous version of `Daemon::create_render_state_for_output` for hotplug.
/// Reuses the existing adapter from the renderer instead of requesting a
/// new one, avoiding the async requirement.
#[allow(clippy::too_many_arguments)]
fn create_render_state_for_output_sync(
    renderer: &std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Renderer>>>>,
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
    let shm_surface = layer_surface.wl_surface().clone();
    wayland_state.surfaces.push((output_id, layer_surface));

    Ok(RenderState {
        gpu: None,
        renderer: renderer.clone(),
        display_ptr: SendDisplayPtr(display_ptr),
        raw_surface: SendDisplayPtr(raw_surface),
        render_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        playback_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        pacer: std::sync::Arc::new(LivePacer::new()),
        current_bind: None,
        current_tex: None,
        width,
        height,
        current_width: 0,
        current_height: 0,
        video_playback: std::sync::Arc::new(crate::video::VideoPlayback::new()),
        hw_accel: crate::video::HwAccel::from_config(&config.video.hw_decode),
        preload_frames: clamp_preload_frames(config.video.preload_frames),
        max_fps: clamp_max_fps(config.daemon.max_fps),
        scaling_mode: 0,
        current_effect: None,
        last_wallpaper: None,
        pre_blank: None,
        blanked: false,
        gif_paused: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        shm_surface,
        shm_server: wayland_state.shm.wl_shm().clone(),
        shm_pool: SlotPool::new(INITIAL_SHM_POOL_BYTES, &wayland_state.shm).map_err(|e| {
            DaemonError::StartError(format!("shared-memory pool creation failed: {e}"))
        })?,
        shm_buffers: [None, None],
        shm_index: 0,
        shm_width: 0,
        shm_height: 0,
        gpu_surface_used: false,
        cached_video_texture: None,
    })
}
