//! Video decoder with hardware acceleration support.
//!
//! Implements FFmpeg-based video decoding with automatic hardware acceleration
//! detection (VAAPI, NVDEC, VideoToolbox) and graceful fallback to software
//! decoding. Decoding runs on a dedicated thread and pushes frames through a
//! bounded channel so memory stays flat no matter how long the video is.
//!
//! The decode thread also listens on a control channel, so pause, resume and
//! seek take effect immediately instead of waiting for the consumer.

use crate::video::error::{VideoError, VideoResult};
use crate::video::scheduler::ScheduledFrame;
use crossbeam_channel::{Receiver, SendTimeoutError, Sender};
use ffmpeg_next as ffmpeg;
use std::ffi::{CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;

/// Trait for video sources - allows different format implementations.
pub trait VideoSource: Send {
    /// Get video metadata.
    fn metadata(&self) -> &VideoMetadata;

    /// Get decoder information.
    fn decoder_info(&self) -> DecoderInfo;

    /// Get the next decoded frame (non-blocking).
    fn next_frame(&self) -> Option<VideoFrame>;

    /// Check if this is a video file.
    fn is_video_file(path: &Path) -> bool;
}

/// Video metadata extracted from the file.
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration: Duration,
    pub fps: f64,
    pub codec: String,
    pub format: String,
    pub total_frames: u64,
}

/// Information about the active decoder.
#[derive(Debug, Clone)]
pub struct DecoderInfo {
    pub codec_name: String,
    pub hardware_accel: Option<String>,
    pub pixel_format: String,
}

/// Video frame ready for texture upload.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// RGBA8 pixel data.
    pub data: Vec<u8>,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Presentation timestamp.
    pub pts: Duration,
    /// Frame index.
    pub index: u64,
}

impl VideoFrame {
    /// Convert to a scheduled frame.
    pub fn into_scheduled(self) -> ScheduledFrame {
        ScheduledFrame::new(self.data, self.width, self.height, self.pts, self.index)
    }
}

/// Hardware acceleration backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwAccel {
    /// VAAPI (Intel, AMD on Linux).
    Vaapi,
    /// NVDEC (NVIDIA on Linux/Windows).
    Nvdec,
    /// Video Toolbox (macOS).
    VideoToolbox,
    /// Software decoding.
    Software,
}

impl HwAccel {
    /// Get a human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            HwAccel::Vaapi => "VAAPI",
            HwAccel::Nvdec => "NVDEC",
            HwAccel::VideoToolbox => "VideoToolbox",
            HwAccel::Software => "Software",
        }
    }

    /// Map an `HwAccel` to a stable code for shared-state reporting.
    const fn code(self) -> u8 {
        match self {
            HwAccel::Software => 1,
            HwAccel::Vaapi => 2,
            HwAccel::Nvdec => 3,
            HwAccel::VideoToolbox => 4,
        }
    }

    /// Inverse of [`HwAccel::code`].
    const fn from_code(code: u8) -> HwAccel {
        match code {
            2 => HwAccel::Vaapi,
            3 => HwAccel::Nvdec,
            4 => HwAccel::VideoToolbox,
            _ => HwAccel::Software,
        }
    }
    /// Resolve the `video.hw_decode` config string ("auto", "vaapi", "nvdec",
    /// "software") into a concrete backend, auto-detecting on "auto".
    pub fn from_config(value: &str) -> HwAccel {
        match value.trim().to_ascii_lowercase().as_str() {
            "vaapi" => HwAccel::Vaapi,
            "nvdec" | "nvidia" | "cuda" => HwAccel::Nvdec,
            "software" | "none" | "off" => HwAccel::Software,
            _ => HwAccel::detect_available(),
        }
    }

    /// Detect available hardware acceleration.
    pub fn detect_available() -> HwAccel {
        #[cfg(target_os = "linux")]
        {
            if Path::new("/dev/dri/renderD128").exists() {
                return HwAccel::Vaapi;
            }
            if Path::new("/dev/nvidia0").exists() {
                return HwAccel::Nvdec;
            }
        }

        #[cfg(target_os = "macos")]
        {
            return HwAccel::VideoToolbox;
        }

        HwAccel::Software
    }
}

/// Commands the consumer (daemon) sends to the decode thread.
#[derive(Debug, Clone, Copy)]
enum DecoderControl {
    Pause,
    Resume,
    Seek(Duration),
}

/// Video decoder with hardware acceleration support.
pub struct VideoDecoder {
    metadata: VideoMetadata,
    frame_rx: Receiver<VideoFrame>,
    control_tx: Sender<DecoderControl>,
    stop_flag: Arc<AtomicBool>,
    /// Which decoder is actually in use (may downgrade from the requested one).
    hw_in_use: Arc<AtomicU8>,
    decode_thread: Option<thread::JoinHandle<()>>,
}

impl VideoDecoder {
    /// Create a new video decoder and start decoding.
    ///
    /// Automatically attempts hardware acceleration and falls back to software
    /// with a diagnostic warning if the hardware path cannot be set up.
    pub fn new<P: AsRef<Path>>(path: P, hw_accel: HwAccel) -> VideoResult<Self> {
        let path = path.as_ref().to_path_buf();

        // Initialize FFmpeg
        ffmpeg::init().map_err(|e| {
            VideoError::SoftwareDecoderInit(anyhow::anyhow!("FFmpeg init failed: {}", e))
        })?;

        // Open the input file and extract metadata
        let metadata = Self::extract_metadata(&path)?;

        tracing::info!(
            "Opened video: {}x{} @ {:.2} fps, duration: {:?}, codec: {}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration,
            metadata.codec
        );

        // Create frame channel with bounded capacity (keep only a few frames
        // in memory, so long videos never balloon RAM usage).
        let (frame_tx, frame_rx) = crossbeam_channel::bounded(3);
        // Control messages are rare and must never block the sender.
        let (control_tx, control_rx) = crossbeam_channel::unbounded();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = stop_flag.clone();
        let hw_in_use = Arc::new(AtomicU8::new(0));
        let hw_in_use_clone = hw_in_use.clone();

        // Spawn the decode thread
        let decode_thread = thread::Builder::new()
            .name("wallr-video-decoder".to_string())
            .spawn(move || {
                let hw_report = hw_in_use_clone.clone();
                let used = std::panic::catch_unwind(|| {
                    Self::decode_loop(
                        path,
                        hw_accel,
                        frame_tx,
                        control_rx,
                        stop_flag_clone,
                        hw_report,
                    )
                });
                let used = match used {
                    Ok(Ok(used)) => used,
                    Ok(Err(e)) => {
                        tracing::error!("Video decode loop error: {}", e);
                        HwAccel::Software
                    }
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".to_string());
                        tracing::error!("Video decode thread panicked: {}", msg);
                        HwAccel::Software
                    }
                };
                tracing::info!("Video decode thread exited (backend: {})", used.name());
                hw_in_use_clone.store(used.code(), Ordering::Relaxed);
            })
            .map_err(|e| VideoError::SoftwareDecoderInit(e.into()))?;

        Ok(Self {
            metadata,
            frame_rx,
            control_tx,
            stop_flag,
            hw_in_use,
            decode_thread: Some(decode_thread),
        })
    }

    /// Extract metadata from a video file.
    fn extract_metadata(path: &Path) -> VideoResult<VideoMetadata> {
        let ictx = ffmpeg::format::input(&path).map_err(|e| VideoError::FileOpen {
            path: path.to_path_buf(),
            source: std::io::Error::other(e.to_string()),
        })?;

        let stream = ictx
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| VideoError::NoVideoStream(path.to_path_buf()))?;

        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .and_then(|ctx| ctx.decoder().video())
            .map_err(|e| VideoError::SoftwareDecoderInit(e.into()))?;

        let width = decoder.width();
        let height = decoder.height();
        let codec = decoder
            .codec()
            .map(|c| c.name().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Calculate FPS
        let fps = {
            let frame_rate = stream.avg_frame_rate();
            if frame_rate.numerator() > 0 {
                frame_rate.numerator() as f64 / frame_rate.denominator() as f64
            } else {
                30.0 // Default fallback
            }
        };

        // Calculate duration
        let duration = {
            let duration_ts = stream.duration();
            let time_base = stream.time_base();
            if duration_ts > 0 {
                Duration::from_secs_f64(
                    duration_ts as f64 * time_base.numerator() as f64
                        / time_base.denominator() as f64,
                )
            } else {
                // Try container duration
                let container_duration = ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;
                Duration::from_secs_f64(container_duration)
            }
        };

        let total_frames = if fps > 0.0 {
            (duration.as_secs_f64() * fps) as u64
        } else {
            0
        };

        let format = ictx.format().name().to_string();

        Ok(VideoMetadata {
            width,
            height,
            duration,
            fps,
            codec,
            format,
            total_frames,
        })
    }

    /// Create an FFmpeg hardware device context for the given backend.
    ///
    /// Returns the raw `AVBufferRef` (owned by the `AVCodecContext` once
    /// attached — FFmpeg unrefs it on `avcodec_free_context`) plus the device
    /// type name for diagnostics. `None` means the backend is unavailable.
    fn init_hw_device(hw_accel: HwAccel) -> Option<(*mut ffmpeg::ffi::AVBufferRef, &'static str)> {
        let (type_name, device) = match hw_accel {
            HwAccel::Vaapi => ("vaapi", Some(c"/dev/dri/renderD128")),
            HwAccel::Nvdec => ("cuda", Some(c"0")),
            HwAccel::VideoToolbox => ("videotoolbox", None),
            HwAccel::Software => return None,
        };

        let type_name_c = match CString::new(type_name) {
            Ok(c) => c,
            Err(_) => return None,
        };

        // SAFETY: We pass correctly formed C strings to FFmpeg and treat the
        // returned device context as owned by the codec context afterwards.
        unsafe {
            let hw_type = ffmpeg::ffi::av_hwdevice_find_type_by_name(type_name_c.as_ptr());
            if hw_type == ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                tracing::warn!("{} hardware type unavailable", hw_accel.name());
                return None;
            }

            let mut device_ctx: *mut ffmpeg::ffi::AVBufferRef = std::ptr::null_mut();
            let device_ptr: *const c_char = device.map(|d| d.as_ptr()).unwrap_or(std::ptr::null());
            let ret = ffmpeg::ffi::av_hwdevice_ctx_create(
                &mut device_ctx,
                hw_type,
                device_ptr,
                std::ptr::null_mut(),
                0,
            );
            if ret < 0 || device_ctx.is_null() {
                tracing::warn!(
                    "{} device init failed (error {}), falling back to software",
                    hw_accel.name(),
                    ret
                );
                return None;
            }
            Some((device_ctx, type_name))
        }
    }

    /// Build a decoder context, attaching a hardware device context when the
    /// backend is available. Falls back to software with a warning on failure.
    ///
    /// Returns the decoder plus the backend that ended up being used.
    fn build_decoder(
        stream: &ffmpeg::format::stream::Stream,
        hw_accel: HwAccel,
    ) -> (ffmpeg::codec::decoder::Video, HwAccel) {
        if hw_accel != HwAccel::Software
            && let Some((device_ctx, _type_name)) = Self::init_hw_device(hw_accel)
        {
            // SAFETY: `device_ctx` is a valid heap allocation owned by FFmpeg.
            // Attaching it before `avcodec_open2` is the documented flow
            // (doc/examples/hw_decode.c); the codec context unrefs it when it
            // is freed, so we must not free it ourselves.
            let mut context =
                match ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        tracing::warn!(
                            "codec context creation failed ({}), falling back to software",
                            e
                        );
                        let decoder =
                            ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                                .and_then(|ctx| ctx.decoder().video())
                                .expect("software decoder must be available");
                        return (decoder, HwAccel::Software);
                    }
                };
            unsafe {
                (*context.as_mut_ptr()).hw_device_ctx = device_ctx;
            }
            match context.decoder().video() {
                Ok(decoder) => {
                    tracing::info!("Hardware decode active: {}", hw_accel.name());
                    return (decoder, hw_accel);
                }
                Err(e) => {
                    tracing::warn!(
                        "{} decode init failed ({}), falling back to software",
                        hw_accel.name(),
                        e
                    );
                    // `context` is dropped here, which frees the codec context
                    // and unrefs `device_ctx`.
                }
            }
        }

        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .and_then(|ctx| ctx.decoder().video())
            .expect("software decoder must be available");
        (decoder, HwAccel::Software)
    }

    /// Main decode loop (runs in a separate thread).
    ///
    /// Returns the hardware backend actually used.
    #[allow(clippy::too_many_arguments)]
    fn decode_loop(
        path: PathBuf,
        hw_accel: HwAccel,
        frame_tx: Sender<VideoFrame>,
        control_rx: Receiver<DecoderControl>,
        stop_flag: Arc<AtomicBool>,
        hw_in_use: Arc<AtomicU8>,
    ) -> VideoResult<HwAccel> {
        let mut ictx = ffmpeg::format::input(&path).map_err(|e| VideoError::FileOpen {
            path: path.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;

        let stream = ictx
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| VideoError::NoVideoStream(path.clone()))?;

        let video_stream_index = stream.index();
        let time_base = stream.time_base();

        let (mut decoder, used_hw) = Self::build_decoder(&stream, hw_accel);
        hw_in_use.store(used_hw.code(), Ordering::Relaxed);
        tracing::info!("Decoder in use: {}", used_hw.name());

        // The scaler is created lazily from the first frame's *software* pixel
        // format: with hardware decode the codec reports the hardware format,
        // and only the transferred frame exposes the real one.
        let mut scaler: Option<ffmpeg::software::scaling::Context> = None;
        let mut scaler_src: Option<ffmpeg::format::Pixel> = None;

        let mut paused = false;
        let mut pending_seek: Option<Duration> = None;
        let mut frame_index = 0u64;
        let mut loop_count = 0u64;
        let mut decoded_frame = ffmpeg::frame::Video::empty();
        let mut sw_frame = ffmpeg::frame::Video::empty();
        let mut rgb_frame = ffmpeg::frame::Video::empty();

        'outer: loop {
            loop_count += 1;
            tracing::debug!("Decode loop iteration {}", loop_count);
            if stop_flag.load(Ordering::Relaxed) {
                tracing::info!("Decode loop exiting (stop flag)");
                break;
            }

            // Drain control messages. This never touches `ictx`, so it can
            // also run inside the packet loop below.
            while let Ok(control) = control_rx.try_recv() {
                match control {
                    DecoderControl::Pause => paused = true,
                    DecoderControl::Resume => paused = false,
                    DecoderControl::Seek(ts) => pending_seek = Some(ts),
                }
            }

            // Apply a pending seek (needs the input context, so it runs here
            // between packet loops).
            if let Some(ts) = pending_seek.take() {
                Self::apply_seek(&mut ictx, &mut decoder, time_base, ts);
            }

            // While paused, don't decode: keep the CPU near idle.
            if paused {
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            // Read packets from the stream
            for (stream, packet) in ictx.packets() {
                if stop_flag.load(Ordering::Relaxed) {
                    break 'outer;
                }

                // Stay responsive to pause/seek even while demuxing.
                while let Ok(control) = control_rx.try_recv() {
                    match control {
                        DecoderControl::Pause => paused = true,
                        DecoderControl::Resume => paused = false,
                        DecoderControl::Seek(ts) => pending_seek = Some(ts),
                    }
                }
                if paused || pending_seek.is_some() {
                    // Restart the 'outer loop, which applies the seek and
                    // honors the pause. Exiting the thread here would end
                    // playback on any seek or pause.
                    break;
                }

                if stream.index() != video_stream_index {
                    continue;
                }

                decoder.send_packet(&packet).map_err(|e| {
                    VideoError::DecodeFailed(anyhow::anyhow!("send_packet failed: {}", e))
                })?;

                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    if stop_flag.load(Ordering::Relaxed) {
                        break 'outer;
                    }

                    // Hardware frames carry no CPU-visible pixel data; copy
                    // them into a software frame first.
                    let is_hw_frame = unsafe { (*decoded_frame.as_ptr()).data[0].is_null() };

                    let src_frame = if is_hw_frame {
                        // SAFETY: `sw_frame` is a valid allocated AVFrame and
                        // `decoded_frame` is a valid hardware frame; FFmpeg
                        // allocates the destination buffers itself.
                        let ret = unsafe {
                            ffmpeg::ffi::av_hwframe_transfer_data(
                                sw_frame.as_mut_ptr(),
                                decoded_frame.as_ptr(),
                                0,
                            )
                        };
                        if ret < 0 {
                            tracing::warn!("hwframe transfer failed: {ret}");
                            continue;
                        }
                        &sw_frame
                    } else {
                        &decoded_frame
                    };

                    let src_format = src_frame.format();
                    if scaler_src != Some(src_format) {
                        scaler = Some(
                            ffmpeg::software::scaling::context::Context::get(
                                src_format,
                                src_frame.width(),
                                src_frame.height(),
                                ffmpeg::format::Pixel::RGBA,
                                src_frame.width(),
                                src_frame.height(),
                                ffmpeg::software::scaling::Flags::BILINEAR,
                            )
                            .map_err(|e| VideoError::FormatConversionFailed(e.into()))?,
                        );
                        scaler_src = Some(src_format);
                        tracing::debug!("Scaler initialized for format {:?}", src_format);
                    }

                    scaler
                        .as_mut()
                        .expect("scaler initialized above")
                        .run(src_frame, &mut rgb_frame)
                        .map_err(|e| VideoError::FormatConversionFailed(e.into()))?;

                    // Calculate PTS in Duration
                    let pts_duration = if let Some(pts) = decoded_frame.timestamp() {
                        Duration::from_secs_f64(
                            pts as f64 * time_base.numerator() as f64
                                / time_base.denominator() as f64,
                        )
                    } else {
                        Duration::from_secs_f64(frame_index as f64 / 30.0) // Fallback
                    };

                    // Copy frame data
                    let data = rgb_frame.data(0).to_vec();

                    let video_frame = VideoFrame {
                        data,
                        width: rgb_frame.width(),
                        height: rgb_frame.height(),
                        pts: pts_duration,
                        index: frame_index,
                    };
                    frame_index = frame_index.wrapping_add(1);

                    // Push the frame, but never block indefinitely: bounded
                    // waits let pause/seek/stop reach the thread promptly.
                    match frame_tx.send_timeout(video_frame, Duration::from_millis(20)) {
                        Ok(()) => {}
                        Err(SendTimeoutError::Timeout(_)) => {
                            // Consumer is behind (e.g. during a transition);
                            // drop this frame and restart the packet loop so
                            // stop/pause/seek checks run. `break` (not
                            // `break 'outer`): the 'outer loop must survive.
                            break;
                        }
                        Err(SendTimeoutError::Disconnected(_)) => {
                            // Consumer dropped the queue (daemon stopped).
                            tracing::warn!("Frame queue disconnected, ending decode loop");
                            return Ok(used_hw);
                        }
                    }
                }
            }

            // End of stream: drain any frames still held by the decoder, then
            // seek back to the start for seamless looping.
            tracing::debug!(
                "End of stream after {} frames, seeking back to 0",
                frame_index
            );
            while decoder.receive_frame(&mut decoded_frame).is_ok() {
                if stop_flag.load(Ordering::Relaxed) {
                    break 'outer;
                }
            }
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            if !paused {
                ictx.seek(0, ..)
                    .map_err(|e| VideoError::SeekFailed(Duration::ZERO, e.into()))?;
                decoder.flush();
            }
        }

        Ok(used_hw)
    }

    /// Reposition the demuxer and decoder to a timestamp.
    fn apply_seek(
        ictx: &mut ffmpeg::format::context::Input,
        decoder: &mut ffmpeg::codec::decoder::Video,
        time_base: ffmpeg::Rational,
        ts: Duration,
    ) {
        let tb_sec = time_base.numerator() as f64 / time_base.denominator() as f64;
        let ts_tb = if tb_sec > 0.0 {
            (ts.as_secs_f64() / tb_sec) as i64
        } else {
            0
        };
        match ictx.seek(ts_tb, ..) {
            Ok(()) => {
                decoder.flush();
                // Drop frames still buffered inside the decoder so we restart
                // cleanly at the seek target.
                let mut drain = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut drain).is_ok() {}
                tracing::info!("Decoder seeked to {:?}", ts);
            }
            Err(e) => tracing::warn!("Decoder seek to {:?} failed: {}", ts, e),
        }
    }

    /// Get the next decoded frame (non-blocking).
    pub fn next_frame(&self) -> Option<VideoFrame> {
        self.frame_rx.try_recv().ok()
    }

    /// Get video metadata.
    pub fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    /// Get decoder information for diagnostics (reflects the backend actually
    /// in use).
    pub fn decoder_info(&self) -> DecoderInfo {
        DecoderInfo {
            codec_name: self.metadata.codec.clone(),
            hardware_accel: if self.hw_accel_in_use() != HwAccel::Software {
                Some(self.hw_accel_in_use().name().to_string())
            } else {
                None
            },
            pixel_format: "RGBA".to_string(),
        }
    }

    /// The hardware backend currently in use by the decode thread. The decode
    /// thread reports the real backend shortly after startup (it may fall back
    /// from a requested hw backend to software), so this polls briefly until
    /// the report lands.
    pub fn hw_accel_in_use(&self) -> HwAccel {
        // Value 0 means "not yet reported".
        for _ in 0..50 {
            let code = self.hw_in_use.load(Ordering::Relaxed);
            if code != 0 {
                return HwAccel::from_code(code);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        HwAccel::Software
    }

    /// Pause decoding (the decode thread stops producing frames).
    pub fn pause(&self) {
        let _ = self.control_tx.send(DecoderControl::Pause);
    }

    /// Resume decoding.
    pub fn resume(&self) {
        let _ = self.control_tx.send(DecoderControl::Resume);
    }

    /// Seek the underlying stream to a timestamp. Frames already queued are
    /// stale after the seek, so call [`VideoDecoder::drain`] afterwards.
    pub fn seek(&self, timestamp: Duration) {
        let _ = self.control_tx.send(DecoderControl::Seek(timestamp));
    }

    /// Drop all frames still queued for consumption (used after a seek).
    pub fn drain(&mut self) {
        while self.frame_rx.try_recv().is_ok() {}
    }

    /// Check if this file is a supported video format.
    pub fn is_video_file<P: AsRef<Path>>(path: P) -> bool {
        let path = path.as_ref();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            matches!(
                ext.to_lowercase().as_str(),
                "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v"
            )
        } else {
            false
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // Signal the decode thread to stop
        self.stop_flag.store(true, Ordering::Relaxed);

        // Wait for the thread to finish
        if let Some(thread) = self.decode_thread.take() {
            let _ = thread.join();
        }
    }
}

impl VideoSource for VideoDecoder {
    fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    fn decoder_info(&self) -> DecoderInfo {
        self.decoder_info()
    }

    fn next_frame(&self) -> Option<VideoFrame> {
        self.next_frame()
    }

    fn is_video_file(path: &Path) -> bool {
        VideoDecoder::is_video_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_video_file() {
        assert!(VideoDecoder::is_video_file("test.mp4"));
        assert!(VideoDecoder::is_video_file("test.MP4"));
        assert!(VideoDecoder::is_video_file("test.webm"));
        assert!(VideoDecoder::is_video_file("test.mkv"));
        assert!(VideoDecoder::is_video_file("test.mov"));
        assert!(!VideoDecoder::is_video_file("test.jpg"));
        assert!(!VideoDecoder::is_video_file("test.gif"));
        assert!(!VideoDecoder::is_video_file("test.png"));
    }

    #[test]
    fn test_hwaccel_names() {
        assert_eq!(HwAccel::Vaapi.name(), "VAAPI");
        assert_eq!(HwAccel::Nvdec.name(), "NVDEC");
        assert_eq!(HwAccel::Software.name(), "Software");
    }

    #[test]
    fn test_hwaccel_codes_roundtrip() {
        for accel in [
            HwAccel::Software,
            HwAccel::Vaapi,
            HwAccel::Nvdec,
            HwAccel::VideoToolbox,
        ] {
            assert_eq!(HwAccel::from_code(accel.code()), accel);
        }
    }

    #[test]
    fn test_hwaccel_from_config() {
        assert_eq!(HwAccel::from_config("vaapi"), HwAccel::Vaapi);
        assert_eq!(HwAccel::from_config("nvdec"), HwAccel::Nvdec);
        assert_eq!(HwAccel::from_config("software"), HwAccel::Software);
        // "auto" resolves to whatever is available on this machine
        assert!(matches!(
            HwAccel::from_config("auto"),
            HwAccel::Vaapi | HwAccel::Nvdec | HwAccel::VideoToolbox | HwAccel::Software
        ));
    }
}
