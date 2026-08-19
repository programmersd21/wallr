use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use image::GenericImageView;
use image::ImageDecoder;
use tracing::info;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::animation::{Effect, compute_effect_uniforms};
use crate::renderer::Renderer;

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("failed to open preview window: {0}")]
    Window(String),
}

pub struct PreviewWindow {
    pub target_path: PathBuf,
    pub duration: std::time::Duration,
    pub effect: Effect,
}

impl PreviewWindow {
    pub fn new(target_path: PathBuf) -> Self {
        Self {
            target_path,
            duration: std::time::Duration::from_millis(2000),
            effect: Effect::Grow(crate::animation::GrowParams::default()),
        }
    }

    pub async fn run(&self) -> Result<(), PreviewError> {
        info!("Initializing preview mode for: {:?}", self.target_path);

        let event_loop = EventLoop::new().map_err(|e| PreviewError::Window(e.to_string()))?;
        event_loop.set_control_flow(ControlFlow::Poll);

        let mut app = PreviewApp::new(self.target_path.clone(), self.duration, self.effect.clone());
        event_loop
            .run_app(&mut app)
            .map_err(|e| PreviewError::Window(e.to_string()))?;
        Ok(())
    }
}

/// The daemon persists the path of the last applied wallpaper here.
fn last_wallpaper_state() -> Option<PathBuf> {
    let state_path = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("wallr/last_wallpaper");
    std::fs::read_to_string(state_path)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
}

/// Load the last applied wallpaper for use as the outgoing frame. Returns
/// `None` when nothing was ever applied or when it is the same file as the
/// incoming image (a same-to-same transition would show nothing).
fn load_last_wallpaper(target: &std::path::Path) -> Option<image::DynamicImage> {
    let last = last_wallpaper_state().filter(|p| p.exists())?;
    let same = last.canonicalize().unwrap_or_else(|_| last.clone())
        == target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());
    if same {
        return None;
    }
    image::ImageReader::open(&last).ok()?.decode().ok()
}

struct PreviewApp {
    target_path: PathBuf,
    duration: std::time::Duration,
    effect: Effect,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: Option<wgpu::TextureFormat>,
    bg_bind: Option<wgpu::BindGroup>,
    new_bind: Option<wgpu::BindGroup>,
    _bg_tex: Option<wgpu::Texture>,
    _new_tex: Option<wgpu::Texture>,
    start: Option<Instant>,
    img_size: (u32, u32),
    old_img_size: (u32, u32),
    animated: Option<crate::animated::AnimatedImage>,
    play_tex: Option<wgpu::Texture>,
    play_bind: Option<wgpu::BindGroup>,
    shown_frame: usize,
    video: Option<crate::video::VideoPlayback>,
    video_texture: Option<crate::renderer::VideoTexture>,
    video_size: (u32, u32),
    per_output_uniforms: Option<crate::renderer::PerOutputUniforms>,
}

impl PreviewApp {
    fn new(target_path: PathBuf, duration: std::time::Duration, effect: Effect) -> Self {
        Self {
            target_path,
            duration,
            effect,
            window: None,
            renderer: None,
            surface: None,
            surface_format: None,
            bg_bind: None,
            new_bind: None,
            _bg_tex: None,
            _new_tex: None,
            start: None,
            img_size: (1, 1),
            old_img_size: (1, 1),
            animated: None,
            play_tex: None,
            play_bind: None,
            shown_frame: usize::MAX,
            video: None,
            video_texture: None,
            video_size: (1, 1),
            per_output_uniforms: None,
        }
    }

    fn configure_surface(
        &self,
        renderer: &Renderer,
        surface: &wgpu::Surface<'static>,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        surface.configure(
            &renderer.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: width.max(1),
                height: height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
    }
}

impl ApplicationHandler for PreviewApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("wallr preview")
                .with_inner_size(LogicalSize::new(1280.0, 720.0)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("failed to create preview window: {e}");
                event_loop.exit();
                return;
            }
        };

        let renderer = match pollster::block_on(Renderer::new()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("renderer init failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let surface = match renderer.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("surface creation failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let caps = surface.get_capabilities(&renderer.adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let size = window.inner_size();
        self.configure_surface(&renderer, &surface, format, size.width, size.height);

        // Video preview: stream decoded frames directly into the preview
        // window. Images and GIFs keep the transition-based preview path.
        if crate::video::VideoDecoder::is_video_file(&self.target_path) {
            let playback = crate::video::VideoPlayback::new();
            match crate::video::VideoPlayback::prepare(
                &self.target_path,
                crate::video::HwAccel::from_config("auto"),
                crate::config::VideoConfig::default().preload_frames,
                std::time::Duration::from_millis(2000),
                |metadata| {
                    renderer
                        .validate_video_texture(metadata.width, metadata.height)
                        .map_err(crate::video::VideoError::GpuResourceCreation)
                },
            ) {
                Ok(mut prepared) => {
                    let meta = prepared.metadata().clone();
                    let first = prepared.take_first_frame();
                    let (w, h) = (meta.width, meta.height);
                    let texture = match renderer.create_video_texture(w, h) {
                        Ok(texture) => texture,
                        Err(e) => {
                            eprintln!("failed to create video texture: {e}");
                            event_loop.exit();
                            return;
                        }
                    };
                    if let Some(frame) = first {
                        if let Err(e) = renderer.update_video_texture(&texture, &frame.data) {
                            eprintln!("failed to upload first video frame: {e}");
                            event_loop.exit();
                            return;
                        }
                    }
                    playback.commit(prepared, 0);
                    self.video = Some(playback);
                    self.video_texture = Some(texture);
                    self.video_size = (w, h);
                    self.window = Some(window);
                    self.per_output_uniforms = Some(renderer.create_per_output_uniforms());
                    self.renderer = Some(renderer);
                    self.surface = Some(surface);
                    self.surface_format = Some(format);
                    self.start = Some(Instant::now());
                    return;
                }
                Err(e) => {
                    eprintln!("failed to start video preview: {e}");
                    event_loop.exit();
                    return;
                }
            }
        }

        let img = match image::ImageReader::open(&self.target_path) {
            Ok(reader) => match reader.into_decoder() {
                Ok(decoder) => {
                    let (width, height) = decoder.dimensions();
                    if let Err(e) =
                        Renderer::validate_static_decode(width, height, decoder.total_bytes())
                    {
                        eprintln!("failed to decode image: {e}");
                        event_loop.exit();
                        return;
                    }
                    match image::DynamicImage::from_decoder(decoder) {
                        Ok(img) => img,
                        Err(e) => {
                            eprintln!("failed to decode image: {e}");
                            event_loop.exit();
                            return;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("failed to decode image: {e}");
                    event_loop.exit();
                    return;
                }
            },
            Err(e) => {
                eprintln!("failed to open image: {e}");
                event_loop.exit();
                return;
            }
        };

        let (new_tex, new_bind, w, h) =
            match renderer.load_texture(&img, size.width, size.height, 0) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("failed to upload image: {e}");
                    event_loop.exit();
                    return;
                }
            };

        // Fade in over the last applied wallpaper (persisted by the daemon),
        // so the preview shows a real transition like on the desktop. Fall
        // back to a solid black texture when nothing was ever applied, when
        // the outgoing image cannot be loaded, or when it is the same file
        // as the incoming one (a same-to-same loop would show nothing).
        let (bg_tex, bg_bind, old_size) =
            match load_last_wallpaper(&self.target_path).and_then(|bg| {
                renderer
                    .load_texture(&bg, size.width, size.height, 0)
                    .ok()
                    .map(|(tex, bind, width, height)| (tex, bind, (width, height)))
            }) {
                Some((tex, bind, size)) => (tex, bind, size),
                None => {
                    let black = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
                        w.max(1),
                        h.max(1),
                        image::Rgba([0, 0, 0, 255]),
                    ));
                    match renderer.load_texture(&black, size.width, size.height, 0) {
                        Ok((tex, bind, width, height)) => (tex, bind, (width, height)),
                        Err(e) => {
                            eprintln!("failed to upload background: {e}");
                            event_loop.exit();
                            return;
                        }
                    }
                }
            };

        // When the target is an animated GIF, the transition runs once over
        // the first frame and then the preview switches to live playback,
        // just like the daemon does on the desktop.
        self.animated = crate::animated::AnimatedImage::decode(&self.target_path)
            .ok()
            .flatten();
        if let Some(anim) = self.animated.as_mut() {
            let (w, h) = (anim.width, anim.height);
            let (tex, bind) = match renderer.create_texture(w, h) {
                Ok(texture) => texture,
                Err(e) => {
                    eprintln!("failed to create GIF texture: {e}");
                    event_loop.exit();
                    return;
                }
            };
            let first = anim.first_frame();
            if !first.is_empty() {
                renderer.update_texture(&tex, first, w, h);
            }
            self.play_tex = Some(tex);
            self.play_bind = Some(bind);
        }

        self.window = Some(window);
        self.per_output_uniforms = Some(renderer.create_per_output_uniforms());
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.surface_format = Some(format);
        self.bg_bind = Some(bg_bind);
        self.new_bind = Some(new_bind);
        self._bg_tex = Some(bg_tex);
        self._new_tex = Some(new_tex);
        self.img_size = img.dimensions();
        self.old_img_size = old_size;
        self.start = Some(Instant::now());
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(renderer), Some(surface), Some(format)) =
                    (&self.renderer, &self.surface, self.surface_format)
                {
                    self.configure_surface(renderer, surface, format, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.render_frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl PreviewApp {
    fn render_frame(&mut self, event_loop: &ActiveEventLoop) {
        // Video targets skip the transition and stream frames live.
        if self.video.is_some() {
            self.render_video_frame(event_loop);
            return;
        }

        let Some(start) = self.start else { return };

        let elapsed = start.elapsed().as_millis() as f32;
        let total = self.duration.as_millis() as f32;
        let progress = if self.animated.is_some() {
            // Animated targets: the transition runs exactly once, then the
            // preview switches to live playback of the GIF frames.
            (elapsed / total).min(1.0)
        } else {
            // Loop the transition forever so the effect can be judged
            // repeatedly. The final frame is held for 25% of the duration so
            // the loop restart reads as a deliberate pause, not a hard snap
            // back to the outgoing wallpaper.
            let hold = total * 0.25;
            let cycle = elapsed % (total + hold);
            (cycle / total).min(1.0)
        };

        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1, 1));

        if self.animated.is_some() && progress >= 1.0 {
            self.render_live_frame(event_loop, size, elapsed, total);
            return;
        }

        let Some(renderer) = &self.renderer else {
            return;
        };
        let Some(surface) = &self.surface else { return };
        let Some(format) = self.surface_format else {
            return;
        };
        let Some(bg) = &self.bg_bind else { return };
        let Some(new_bind) = &self.new_bind else {
            return;
        };

        let (img_w, img_h) = self.img_size;
        let (old_w, old_h) = self.old_img_size;

        let uniforms = compute_effect_uniforms(&self.effect, progress);

        match renderer.render_frame(
            crate::renderer::FrameRequest {
                surface,
                format,
                bg_bind: bg,
                new_bind,
                effect: &uniforms,
                width: size.width,
                height: size.height,
                img_width: img_w,
                img_height: img_h,
                old_img_width: old_w,
                old_img_height: old_h,
                scaling_mode: 0,
            },
            self.per_output_uniforms.as_ref().unwrap(),
        ) {
            Ok(crate::renderer::FrameStatus::TimedOut) => {
                // The surface is not presenting right now (e.g. the window is
                // hidden or the monitor is off); keep polling so the preview
                // resumes cleanly once frames flow again.
            }
            Err(e) => {
                eprintln!("render error: {e}");
                event_loop.exit();
            }
            _ => {}
        }
    }

    /// Mirrors the daemon's `play_video` loop: pull the next displayable frame
    /// per vsync and present it, never blocking on the bounded decoder queue.
    fn render_video_frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let Some(surface) = &self.surface else { return };
        let Some(format) = self.surface_format else {
            return;
        };
        let Some(playback) = &self.video else { return };
        let Some(texture) = &self.video_texture else {
            return;
        };

        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1, 1));

        let (w, h) = self.video_size;
        if let Some(frame) = playback.next_frame()
            && frame.width == w
            && frame.height == h
        {
            if let Err(e) = renderer.update_video_texture(texture, &frame.data) {
                eprintln!("video upload error: {e}");
                event_loop.exit();
                return;
            }
        }

        let uniforms = compute_effect_uniforms(&self.effect, 1.0);
        match renderer.render_frame(
            crate::renderer::FrameRequest {
                surface,
                format,
                bg_bind: texture.bind_group(),
                new_bind: texture.bind_group(),
                effect: &uniforms,
                width: size.width.max(1),
                height: size.height.max(1),
                img_width: w,
                img_height: h,
                old_img_width: w,
                old_img_height: h,
                scaling_mode: 0,
            },
            self.per_output_uniforms.as_ref().unwrap(),
        ) {
            Ok(crate::renderer::FrameStatus::TimedOut) => {}
            Err(e) => {
                eprintln!("render error: {e}");
                event_loop.exit();
            }
            _ => {}
        }
    }

    /// Present GIF frames live, one per vsync, after the initial transition
    /// has completed. Texture uploads only happen when the playhead crosses
    /// into a new frame, so playback is smooth without re-uploading frames
    /// that are already showing.
    fn render_live_frame(
        &mut self,
        event_loop: &ActiveEventLoop,
        size: winit::dpi::PhysicalSize<u32>,
        elapsed: f32,
        total: f32,
    ) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let Some(surface) = &self.surface else { return };
        let Some(format) = self.surface_format else {
            return;
        };
        let Some(anim) = self.animated.as_mut() else {
            return;
        };
        let Some(tex) = &self.play_tex else { return };
        let Some(bind) = &self.play_bind else { return };

        let (anim_w, anim_h) = (anim.width, anim.height);
        let live_ms = (elapsed - total).max(0.0) as u64;
        let index = anim.frame_index_at(std::time::Duration::from_millis(live_ms));
        if index != self.shown_frame {
            let frame = anim.frame_at(index);
            if !frame.is_empty() {
                renderer.update_texture(tex, frame, anim_w, anim_h);
                self.shown_frame = index;
            }
        }

        let uniforms = compute_effect_uniforms(&self.effect, 1.0);
        match renderer.render_frame(
            crate::renderer::FrameRequest {
                surface,
                format,
                bg_bind: bind,
                new_bind: bind,
                effect: &uniforms,
                width: size.width,
                height: size.height,
                img_width: anim.width,
                img_height: anim.height,
                old_img_width: anim.width,
                old_img_height: anim.height,
                scaling_mode: 0,
            },
            self.per_output_uniforms.as_ref().unwrap(),
        ) {
            Ok(crate::renderer::FrameStatus::TimedOut) => {}
            Err(e) => {
                eprintln!("render error: {e}");
                event_loop.exit();
            }
            _ => {}
        }
    }
}
