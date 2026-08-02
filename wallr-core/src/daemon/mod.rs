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
    protocol::{wl_compositor, wl_output, wl_surface},
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
        // Note: Input region is set once at creation and persists across
        // configure events. The empty region ensures clicks pass through.
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
            let metadata = self.video_playback.start(path, self.hw_accel)?;

            // Wait for the first frame so the transition's incoming image is
            // the real first frame, not a black placeholder.
            let first_frame = self
                .video_playback
                .wait_first_frame(std::time::Duration::from_millis(1000));

            let (new_tex, new_bind, img_width, img_height) = if let Some(frame) = first_frame {
                let (tex, bind) = self.renderer.create_texture(frame.width, frame.height);
                self.renderer
                    .update_texture(&tex, &frame.data, frame.width, frame.height);
                (tex, bind, frame.width, frame.height)
            } else {
                // Fallback: create black texture
                tracing::warn!("No first frame available, using black texture");
                let (tex, bind) = self
                    .renderer
                    .create_texture(metadata.width, metadata.height);
                let black = vec![0u8; (metadata.width * metadata.height * 4) as usize];
                self.renderer
                    .update_texture(&tex, &black, metadata.width, metadata.height);
                (tex, bind, metadata.width, metadata.height)
            };

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
                generation,
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
        let pacer = self.pacer.clone();
        let video_playback = self.video_playback.clone();
        let effect = effect.clone();
        drop(tokio::task::spawn_blocking(move || {
            render_transition(
                renderer,
                surface,
                render_lock,
                playback_gen,
                pacer,
                video_playback,
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
#[allow(clippy::too_many_arguments)]
fn render_transition(
    renderer: std::sync::Arc<Renderer>,
    surface: &'static wgpu::Surface<'static>,
    render_lock: std::sync::Arc<std::sync::Mutex<()>>,
    playback_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pacer: std::sync::Arc<LivePacer>,
    video_playback: std::sync::Arc<crate::video::VideoPlayback>,
    mut commit: CommitData,
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
    let mut animated = commit.animated.take();
    if let Some(animated) = animated.as_mut()
        && playback_gen.load(Ordering::SeqCst) == commit.generation
    {
        play_live(&renderer, surface, &commit, animated, &playback_gen, &pacer);
    } else if commit.is_video && playback_gen.load(Ordering::SeqCst) == commit.generation {
        play_video(&renderer, surface, &commit, &video_playback, &playback_gen);
    }
}

/// Presents live wallpaper frames until the next commit. One frame is
/// presented per GIF frame boundary instead of at the monitor refresh rate.
/// Two textures are double-buffered and frames are decompressed directly
/// into a mapped staging ring (no intermediate copy), so the wake path only
/// presents and the pacing sleep hides the decode/upload entirely.
fn play_live(
    renderer: &Renderer,
    surface: &'static wgpu::Surface<'static>,
    commit: &CommitData,
    animated: &mut crate::animated::AnimatedImage,
    playback_gen: &std::sync::atomic::AtomicU64,
    pacer: &LivePacer,
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
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
    loop {
        if playback_gen.load(Ordering::SeqCst) != commit.generation {
            return;
        }
        let index = animated.frame_index_at(start.elapsed());
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
        let status = renderer.render_frame(crate::renderer::FrameRequest {
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
        });
        match status {
            Ok(crate::renderer::FrameStatus::Presented) => {}
            // A stalled present parks inside the acquire; a Timeout or error
            // means the surface is unusable, so give up and let the next
            // transition take over.
            _ => return,
        }

        // Pace to the next GIF frame boundary instead of presenting at the
        // monitor refresh rate: an animated wallpaper only needs a present
        // when its frame changes. A commit wakes us via the pacer. The
        // boundary is computed in absolute time (frame_start is loop-relative,
        // so add the completed loops) to stay correct after the animation
        // wraps. While waiting, warm the idle texture with the next frame so
        // the wake path stays on the hot critical section.
        let elapsed = start.elapsed();
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

/// Live video playback loop: continuously updates texture with decoded frames.
fn play_video(
    renderer: &Renderer,
    surface: &'static wgpu::Surface<'static>,
    commit: &CommitData,
    video_playback: &std::sync::Arc<crate::video::VideoPlayback>,
    playback_gen: &std::sync::atomic::AtomicU64,
) {
    // Get initial frame dimensions
    let (width, height) = match video_playback.metadata() {
        Some(meta) => (meta.width, meta.height),
        None => {
            tracing::warn!("No video metadata available");
            return;
        }
    };

    let (texture, bind) = renderer.create_texture(width, height);
    let static_effect = crate::animation::Effect::Fade(crate::animation::FadeParams::default());

    // The texture starts empty; present a real frame before the first
    // vsync so the surface never flashes black.
    let mut uploaded = false;

    loop {
        // A newer commit superseded us. Do NOT touch the shared
        // `video_playback` here: the successor commit already replaced the
        // decoder (video) or stopped it (static image), and stopping it now
        // would kill the successor's playback too.
        if playback_gen.load(Ordering::SeqCst) != commit.generation {
            return;
        }

        // Pull the next displayable frame. The decoder queue is bounded, so
        // this never blocks; `None` means "present the current texture".
        if let Some(frame) = video_playback.next_frame() {
            // The shared decoder can be replaced between commits; never
            // upload a frame whose size does not match this task's texture.
            if frame.width != width || frame.height != height {
                continue;
            }
            renderer.update_texture(&texture, &frame.data, frame.width, frame.height);
            uploaded = true;
        }

        if !uploaded {
            // No frame yet; wait briefly and try again instead of presenting
            // an uninitialized texture.
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
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
            img_width: width,
            img_height: height,
            old_img_width: width,
            old_img_height: height,
        });

        match status {
            Ok(crate::renderer::FrameStatus::Presented) => {}
            // A stalled present parks inside the acquire; a Timeout or error
            // means the surface is unusable, so give up.
            other => {
                tracing::warn!("Video present failed ({:?}), stopping playback", other);
                video_playback.stop();
                return;
            }
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
        layer_surface.set_exclusive_zone(-1); // Don't reserve space
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None); // No keyboard

        // CRITICAL: Empty input region so ALL clicks pass through to desktop
        // Without this, the wallpaper blocks desktop interaction on KDE/Plasma
        let compositor = globals
            .bind::<wl_compositor::WlCompositor, WaylandState, smithay_client_toolkit::globals::GlobalData>(
                &qh,
                1..=4,
                smithay_client_toolkit::globals::GlobalData,
            )
            .map_err(|e| DaemonError::StartError(format!("compositor bind failed: {e:?}")))?;
        let empty_region = compositor.create_region(&qh, ());
        layer_surface
            .wl_surface()
            .set_input_region(Some(&empty_region));
        layer_surface.commit();
        empty_region.destroy();

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
            pacer: std::sync::Arc::new(LivePacer::new()),
            current_bind: None,
            current_tex: None,
            width,
            height,
            current_width: 0,
            current_height: 0,
            format: surf_format,
            video_playback: std::sync::Arc::new(crate::video::VideoPlayback::new()),
            hw_accel: crate::video::HwAccel::from_config(&self.config.video.hw_decode),
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

        // Graceful shutdown on POSIX signals: stop video decoding, remove the
        // IPC socket, and exit. The compositor releases the layer-shell
        // surface automatically when the process exits.
        {
            let rs = render_state.clone();
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
                if let Ok(state) = rs.try_lock() {
                    state.video_playback.stop();
                }
                let _ = std::fs::remove_file(&socket_path);
                std::process::exit(0);
            });
        }

        let ipc_socket_path = socket_path.clone();
        start_ipc_server(&socket_path, move |cmd| {
            let paused = paused_clone.clone();
            let engine = engine_clone.clone();
            let rs = render_state_clone.clone();
            let stop_socket = ipc_socket_path.clone();
            async move {
                match cmd {
                    IpcCommand::Pause => {
                        paused.store(true, Ordering::SeqCst);
                        let rs_lock = rs.lock().await;
                        rs_lock.video_playback.pause();
                        IpcResponse {
                            success: true,
                            message: Some("Paused".into()),
                        }
                    }
                    IpcCommand::Resume => {
                        paused.store(false, Ordering::SeqCst);
                        let rs_lock = rs.lock().await;
                        rs_lock.video_playback.resume();
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
                        // Live playback only starts after the transition, so for
                        // videos an unrequested 2s fade reads as a long "load".
                        // Default to a short fade unless the user asked for one.
                        let is_video = crate::video::VideoDecoder::is_video_file(&p);
                        let duration = duration_ms.unwrap_or(if is_video { 150 } else { 2000 });

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
                    IpcCommand::Seek { timestamp_ms } => {
                        let rs_lock = rs.lock().await;
                        match rs_lock
                            .video_playback
                            .seek(std::time::Duration::from_millis(timestamp_ms))
                        {
                            Ok(()) => IpcResponse {
                                success: true,
                                message: Some(format!("Seeked to {}ms", timestamp_ms)),
                            },
                            Err(e) => IpcResponse {
                                success: false,
                                message: Some(format!("Seek failed: {}", e)),
                            },
                        }
                    }
                    IpcCommand::Info => {
                        let rs_lock = rs.lock().await;

                        // Get GPU info from renderer
                        let gpu_info =
                            crate::video::gpu::adapter_diagnostics(&rs_lock.renderer.adapter);

                        let mut lines = vec![
                            format!("wallr v{}", env!("CARGO_PKG_VERSION")),
                            String::new(),
                            gpu_info,
                        ];

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
                                lines.push(format!("  Resolution: {}x{}", meta.width, meta.height));
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

                        IpcResponse {
                            success: true,
                            message: Some(lines.join("\n")),
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
