use crate::video::error::{VideoError, VideoResult};
use crate::video::scheduler::ScheduledFrame;
use crossbeam_channel::{Receiver, Sender};
use ffmpeg_next as ffmpeg;
use std::collections::VecDeque;
use std::ffi::{CString, c_char};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_VIDEO_DIMENSION: u32 = 16_384;
const MAX_DECODED_FRAME_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwAccel {
    /// Try all hardware backends in priority order, then software
    Auto,
    Vaapi,
    Nvdec,
    VideoToolbox,
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderState {
    Initializing,
    HardwareNegotiating,
    HardwareActive,
    SoftwareActive,
    SoftwareFallback,
    Failed,
}

impl DecoderState {
    const fn code(self) -> u8 {
        match self {
            Self::Initializing => 0,
            Self::HardwareNegotiating => 1,
            Self::HardwareActive => 2,
            Self::SoftwareActive => 3,
            Self::SoftwareFallback => 4,
            Self::Failed => 5,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::HardwareNegotiating,
            2 => Self::HardwareActive,
            3 => Self::SoftwareActive,
            4 => Self::SoftwareFallback,
            5 => Self::Failed,
            _ => Self::Initializing,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::HardwareNegotiating => "hardware negotiation",
            Self::HardwareActive => "hardware active",
            Self::SoftwareActive => "software active",
            Self::SoftwareFallback => "software fallback",
            Self::Failed => "failed",
        }
    }
}

/// Minimum frames a decode probe must produce before it counts as usable.
pub const PROBE_MIN_FRAMES: u64 = 30;

/// Whether a decode probe run should be reported as successful.
///
/// A run passes only when it produced more than [`PROBE_MIN_FRAMES`] frames
/// *and* the decoder never reported failure. Checking the frame count alone
/// would let a decoder that failed after decoding part of the stream pass.
pub const fn probe_succeeded(frames: u64, state: DecoderState) -> bool {
    frames > PROBE_MIN_FRAMES && !matches!(state, DecoderState::Failed)
}

impl HwAccel {
    pub fn name(&self) -> &'static str {
        match self {
            HwAccel::Auto => "Auto",
            HwAccel::Vaapi => "VAAPI",
            HwAccel::Nvdec => "NVDEC",
            HwAccel::VideoToolbox => "VideoToolbox",
            HwAccel::Software => "Software",
        }
    }

    const fn code(self) -> u8 {
        match self {
            HwAccel::Auto => 0,
            HwAccel::Software => 1,
            HwAccel::Vaapi => 2,
            HwAccel::Nvdec => 3,
            HwAccel::VideoToolbox => 4,
        }
    }

    pub fn from_config(value: &str) -> HwAccel {
        match value.trim().to_ascii_lowercase().as_str() {
            "vaapi" => HwAccel::Vaapi,
            "nvdec" | "nvidia" | "cuda" => HwAccel::Nvdec,
            "software" | "none" | "off" => HwAccel::Software,
            _ => HwAccel::Auto,
        }
    }

    /// Hardware backends in priority order for auto-detection, matched to
    /// the GPU actually present. Probing NVDEC on an AMD or iGPU machine
    /// makes FFmpeg attempt to load `libcuda.so.1` first, fail, and print a
    /// scary CUDA warning before ever reaching VAAPI; only NVIDIA hardware
    /// should ever touch the NVDEC path.
    #[cfg(target_os = "linux")]
    fn auto_hardware_order() -> Vec<HwAccel> {
        let mut order = Vec::new();
        if nvidia_gpu_present() {
            order.push(HwAccel::Nvdec);
        }
        if vaapi_available() {
            order.push(HwAccel::Vaapi);
        }
        order
    }
}

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

#[derive(Debug)]
pub enum VideoFrameData {
    Rgba(Vec<u8>),
    Nv12 {
        y_plane: Vec<u8>,
        uv_plane: Vec<u8>,
        color: YuvColorInfo,
    },
}

impl VideoFrameData {
    pub fn len(&self) -> usize {
        match self {
            Self::Rgba(data) => data.len(),
            Self::Nv12 {
                y_plane, uv_plane, ..
            } => y_plane.len() + uv_plane.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YuvColorInfo {
    pub matrix: YuvMatrix,
    pub range: YuvRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvRange {
    Limited,
    Full,
}

#[derive(Debug)]
pub struct VideoFrame {
    pub data: VideoFrameData,
    pub width: u32,
    pub height: u32,
    pub pts: Duration,
    pub index: u64,
}

impl VideoFrame {
    /// Move-only conversion into the scheduler representation. No frame
    /// bytes are cloned: ownership of the decoded planes transfers.
    pub fn into_scheduled(self) -> ScheduledFrame {
        ScheduledFrame::new(self.data, self.width, self.height, self.pts, self.index)
    }
}

#[derive(Debug, Clone)]
pub struct DecoderInfo {
    pub codec_name: String,
    pub hardware_accel: Option<String>,
    pub pixel_format: String,
    pub dropped_frames: u64,
    pub state: DecoderState,
}

type SeekAck = (u64, Option<VideoError>);
type SeekRequest = (Duration, u64, std::sync::mpsc::Sender<SeekAck>);

enum DecoderControl {
    Seek(SeekRequest),
}

pub struct VideoDecoder {
    metadata: VideoMetadata,
    frame_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
    dropped_frames: Arc<AtomicU64>,
    control_tx: Sender<DecoderControl>,
    stop_flag: Arc<AtomicBool>,
    pause_requested: Arc<AtomicBool>,
    seek_epoch: Arc<AtomicU64>,
    hw_in_use: Arc<AtomicU8>,
    decoder_state: Arc<AtomicU8>,
    fallback_occurred: Arc<AtomicBool>,
    max_output_fps: Arc<AtomicU32>,
    decode_thread: Option<thread::JoinHandle<()>>,
}

/// Maximum total bytes retained by a decoder's frame queue. This bounds
/// memory use even when only a few very large frames are queued.
pub(crate) const MAX_VIDEO_FRAME_QUEUE_BYTES: usize = 256 * 1024 * 1024;

pub(crate) fn queued_video_frame_bytes(queue: &std::collections::VecDeque<VideoFrame>) -> usize {
    queue
        .iter()
        .map(|frame| frame.data.len())
        .fold(0usize, usize::saturating_add)
}

/// Push a frame while enforcing both frame-count and byte budgets.
///
/// Returns the number of older frames dropped to make room, or an error if
/// the frame itself exceeds the byte budget.
pub(crate) fn push_bounded_video_frame(
    queue: &mut std::collections::VecDeque<VideoFrame>,
    frame: VideoFrame,
    max_frames: usize,
    max_bytes: usize,
) -> VideoResult<usize> {
    if max_frames == 0 || frame.data.len() > max_bytes {
        return Err(VideoError::QueueFull);
    }
    let frame_bytes = frame.data.len();
    let mut dropped = 0usize;
    while queue.len() >= max_frames
        || queued_video_frame_bytes(queue).saturating_add(frame_bytes) > max_bytes
    {
        if queue.pop_front().is_some() {
            dropped += 1;
        } else {
            break;
        }
    }
    queue.push_back(frame);
    Ok(dropped)
}

pub(crate) fn ffmpeg_seek_timestamp(timestamp: Duration) -> VideoResult<i64> {
    let micros = timestamp.as_micros();
    if micros > i64::MAX as u128 {
        return Err(VideoError::SeekFailed(
            timestamp,
            anyhow::anyhow!("seek timestamp exceeds representable FFmpeg time"),
        ));
    }
    Ok(micros as i64)
}

impl VideoDecoder {
    pub fn new<P: AsRef<Path>>(path: P, hw_accel: HwAccel) -> VideoResult<Self> {
        Self::with_preload(path, hw_accel, 2, true)
    }

    pub fn with_preload<P: AsRef<Path>>(
        path: P,
        hw_accel: HwAccel,
        preload_frames: usize,
        loop_video: bool,
    ) -> VideoResult<Self> {
        let path = path.as_ref().to_path_buf();

        ensure_ffmpeg_init()?;

        let metadata = Self::extract_metadata(&path)?;

        tracing::info!(
            "Opened video: {}x{} @ {:.2} fps, duration: {:?}, codec: {}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration,
            metadata.codec
        );

        // Aggressively bounded decoded-frame queue: wallpaper playback never
        // needs a backlog. At most `preload` frames are in flight, plus one
        // pending frame in `VideoPlayback`. Stale frames are dropped by the
        // scheduler rather than preserved. All transfers are moves.
        let preload = preload_frames.clamp(1, 3);
        let frame_queue = Arc::new(Mutex::new(VecDeque::with_capacity(preload)));
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let (control_tx, control_rx) = crossbeam_channel::unbounded();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = stop_flag.clone();
        let pause_requested = Arc::new(AtomicBool::new(false));
        let pause_requested_clone = pause_requested.clone();
        let seek_epoch = Arc::new(AtomicU64::new(0));
        let hw_in_use = Arc::new(AtomicU8::new(0));
        let hw_in_use_clone = hw_in_use.clone();
        let decoder_state = Arc::new(AtomicU8::new(DecoderState::Initializing.code()));
        let decoder_state_clone = decoder_state.clone();
        let fallback_occurred = Arc::new(AtomicBool::new(false));
        let fallback_clone = fallback_occurred.clone();
        let max_output_fps = Arc::new(AtomicU32::new(0));
        let max_output_fps_clone = max_output_fps.clone();
        let frame_queue_clone = frame_queue.clone();
        let dropped_frames_clone = dropped_frames.clone();

        let decode_thread = thread::Builder::new()
            .name("wallr-video-decoder".to_string())
            .spawn(move || {
                let used = Self::decode_loop(
                    path,
                    hw_accel,
                    frame_queue_clone,
                    preload,
                    dropped_frames_clone,
                    control_rx,
                    stop_flag_clone,
                    pause_requested_clone,
                    hw_in_use_clone.clone(),
                    decoder_state_clone.clone(),
                    fallback_clone,
                    max_output_fps_clone,
                    loop_video,
                );
                let used = match used {
                    Ok(used) => used,
                    Err(e) => {
                        tracing::error!("Video decode loop: {}", e);
                        decoder_state_clone.store(DecoderState::Failed.code(), Ordering::Release);
                        HwAccel::Software
                    }
                };
                tracing::info!("Decode thread exited (backend: {})", used.name());
                // Update again on exit in case it wasn't set during init
                hw_in_use_clone.store(used.code(), Ordering::Relaxed);
            })
            .map_err(|e| VideoError::SoftwareDecoderInit(e.into()))?;

        Ok(Self {
            metadata,
            frame_queue,
            dropped_frames,
            control_tx,
            stop_flag,
            pause_requested,
            seek_epoch,
            hw_in_use,
            decoder_state,
            fallback_occurred,
            max_output_fps,
            decode_thread: Some(decode_thread),
        })
    }

    pub fn set_max_output_fps(&self, fps: Option<u32>) {
        self.max_output_fps
            .store(fps.filter(|fps| *fps > 0).unwrap_or(0), Ordering::Release);
    }

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

        let frame_rate = stream.avg_frame_rate();
        let fps = if frame_rate.numerator() > 0 && frame_rate.denominator() > 0 {
            let fps = frame_rate.numerator() as f64 / frame_rate.denominator() as f64;
            if fps.is_finite() && fps > 0.0 {
                fps
            } else {
                30.0
            }
        } else {
            30.0
        };

        let duration = {
            let duration_ts = stream.duration();
            let time_base = stream.time_base();
            let seconds = if duration_ts > 0 && time_base.denominator() > 0 {
                duration_ts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64
            } else {
                ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64
            };
            duration_from_seconds(seconds)?
        };

        if width == 0 || height == 0 {
            return Err(VideoError::UnsupportedFormat(
                "video has empty dimensions".to_string(),
            ));
        }
        validate_video_dimensions(width, height)?;
        let total_frames = (duration.as_secs_f64() * fps).min(u64::MAX as f64) as u64;

        Ok(VideoMetadata {
            width,
            height,
            duration,
            fps,
            codec,
            format: ictx.format().name().to_string(),
            total_frames,
        })
    }

    fn init_hw_device(hw_accel: HwAccel) -> Option<(*mut ffmpeg::ffi::AVBufferRef, &'static str)> {
        // VA-API render nodes are enumerated instead of hardcoding
        // renderD128 so multi-GPU systems use the first working node and
        // stay on the same physical GPU as the compositor where possible.
        // NVDEC keeps its device index; VideoToolbox needs no device string.
        if hw_accel == HwAccel::Vaapi {
            return Self::init_vaapi_device();
        }
        let (type_name, device) = match hw_accel {
            HwAccel::Nvdec => ("cuda", Some(c"0")),
            HwAccel::VideoToolbox => ("videotoolbox", None),
            HwAccel::Software | HwAccel::Auto | HwAccel::Vaapi => return None,
        };

        let type_name_c = CString::new(type_name).ok()?;

        // SAFETY: C strings passed to FFmpeg; returned device ctx owned by codec context.
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
                tracing::warn!("{} device init failed ({})", hw_accel.name(), ret);
                return None;
            }
            Some((device_ctx, type_name))
        }
    }

    fn init_vaapi_device() -> Option<(*mut ffmpeg::ffi::AVBufferRef, &'static str)> {
        let type_name_c = CString::new("vaapi").ok()?;
        // SAFETY: read-only FFmpeg type lookup with a valid C string.
        let hw_type = unsafe { ffmpeg::ffi::av_hwdevice_find_type_by_name(type_name_c.as_ptr()) };
        if hw_type == ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
            tracing::warn!("VAAPI hardware type unavailable");
            return None;
        }
        for candidate in vaapi_device_candidates() {
            // SAFETY: candidate is a valid NUL-terminated path; the returned
            // device ctx is owned by the codec context on success.
            let result = unsafe {
                let mut device_ctx: *mut ffmpeg::ffi::AVBufferRef = std::ptr::null_mut();
                let ret = ffmpeg::ffi::av_hwdevice_ctx_create(
                    &mut device_ctx,
                    hw_type,
                    candidate.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                );
                if ret < 0 || device_ctx.is_null() {
                    None
                } else {
                    Some((device_ctx, "vaapi"))
                }
            };
            if result.is_some() {
                return result;
            }
        }
        tracing::warn!("VAAPI device init failed on all render nodes");
        None
    }

    /// Try to build a decoder with a hardware device context attached.
    fn try_hw_decoder(
        stream: &ffmpeg::format::stream::Stream,
        hw_accel: HwAccel,
    ) -> Option<(ffmpeg::codec::decoder::Video, HwAccel)> {
        let (device_ctx, _) = Self::init_hw_device(hw_accel)?;

        let mut context =
            ffmpeg::codec::context::Context::from_parameters(stream.parameters()).ok()?;

        let codec = hardware_decoder(stream.parameters().id(), hw_accel)?;

        // SAFETY: device_ctx is a valid FFmpeg allocation; the codec context
        // unrefs it when dropped.
        unsafe {
            (*context.as_mut_ptr()).hw_device_ctx = device_ctx;
            (*context.as_mut_ptr()).get_format = Some(select_hw_pixel_format);
        }

        match context
            .decoder()
            .open_as(codec)
            .and_then(|opened| opened.video())
        {
            Ok(decoder) => {
                tracing::info!(
                    "Hardware decoder initialized: {} using {}; awaiting negotiated hardware frame",
                    hw_accel.name(),
                    codec.name()
                );
                Some((decoder, hw_accel))
            }
            Err(e) => {
                tracing::warn!("{} decode init failed: {}", hw_accel.name(), e);
                None
            }
        }
    }

    /// Build a decoder with appropriate hardware acceleration fallback.
    ///
    /// - Auto: Try all hardware backends in priority order, then software
    /// - Explicit backend (Vaapi, Nvdec, VideoToolbox): Try that backend, then software
    /// - Software: Use software decoder only (no hardware attempts)
    fn build_decoder(
        stream: &ffmpeg::format::stream::Stream,
        hw_accel: HwAccel,
    ) -> VideoResult<(ffmpeg::codec::decoder::Video, HwAccel, DecoderState)> {
        let mut hardware_attempted = false;
        match hw_accel {
            HwAccel::Auto => {
                // Probe only what this machine can actually use, in
                // vendor-matched priority: NVDEC for NVIDIA hardware, VAAPI
                // for everything else with a render node. Never probe NVDEC
                // on AMD/iGPU machines (that makes FFmpeg try to load
                // libcuda.so.1, print a CUDA error, then fall through).
                let mut tried = 0;
                for backend in HwAccel::auto_hardware_order() {
                    tried += 1;
                    hardware_attempted = true;
                    if let Some(result) = Self::try_hw_decoder(stream, backend) {
                        return Ok((result.0, result.1, DecoderState::HardwareNegotiating));
                    }
                }
                if tried == 0 {
                    tracing::info!("No hardware decoder present, using software decoder");
                } else {
                    tracing::info!("Hardware decoders failed, using software decoder");
                }
            }
            HwAccel::Software => {
                // Explicit software request: skip hardware entirely
                tracing::info!("Software decoder explicitly requested");
            }
            specific => {
                // Try the requested hardware backend first
                hardware_attempted = true;
                if let Some(result) = Self::try_hw_decoder(stream, specific) {
                    return Ok((result.0, result.1, DecoderState::HardwareNegotiating));
                }
                // Fall back to software
                tracing::info!(
                    "{} hardware decoder failed, falling back to software",
                    specific.name()
                );
            }
        }

        // Software fallback. An unavailable software decoder (unsupported
        // codec) is a normal error, not a reason to panic the decode thread.
        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .and_then(|ctx| ctx.decoder().video())
            .map_err(|e| VideoError::UnsupportedCodec(e.to_string()))?;
        let state = software_decoder_state(hardware_attempted);
        Ok((decoder, HwAccel::Software, state))
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_loop(
        path: std::path::PathBuf,
        hw_accel: HwAccel,
        frame_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
        frame_capacity: usize,
        dropped_frames: Arc<AtomicU64>,
        control_rx: Receiver<DecoderControl>,
        stop_flag: Arc<AtomicBool>,
        pause_requested: Arc<AtomicBool>,
        hw_in_use: Arc<AtomicU8>,
        decoder_state: Arc<AtomicU8>,
        fallback_occurred: Arc<AtomicBool>,
        max_output_fps: Arc<AtomicU32>,
        loop_video: bool,
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
        let stream_start = {
            let start = stream.start_time();
            if start != i64::MIN && time_base.numerator() > 0 && time_base.denominator() > 0 {
                let seconds =
                    start as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
                seconds.is_finite().then_some(seconds)
            } else {
                None
            }
        };

        let (mut decoder, used_hw, initial_state) = Self::build_decoder(&stream, hw_accel)?;
        tracing::info!("Decoder in use: {}", used_hw.name());
        decoder_state.store(initial_state.code(), Ordering::Release);
        fallback_occurred.store(
            initial_state == DecoderState::SoftwareFallback,
            Ordering::Release,
        );

        let mut scaler: Option<ffmpeg::software::scaling::Context> = None;
        let mut scaler_src: Option<ffmpeg::format::Pixel> = None;

        let mut paused: bool;
        let mut pending_seek: Option<SeekRequest> = None;
        let mut frame_index = 0u64;
        let mut pts_origin: Option<f64> = stream_start;
        let mut last_output_pts = None;
        let mut last_good_position = Duration::ZERO;
        let mut hardware_frame_seen = false;
        let mut decoded_frame = ffmpeg::frame::Video::empty();
        let mut sw_frame = ffmpeg::frame::Video::empty();
        let mut converted_frame = ffmpeg::frame::Video::empty();

        'outer: loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            while let Ok(control) = control_rx.try_recv() {
                match control {
                    DecoderControl::Seek((ts, epoch, ack)) => {
                        if let Some(previous) = pending_seek.take() {
                            let _ = previous.2.send((
                                previous.1,
                                Some(VideoError::SeekFailed(
                                    previous.0,
                                    anyhow::anyhow!("seek superseded"),
                                )),
                            ));
                        }
                        pending_seek = Some((ts, epoch, ack));
                    }
                }
            }
            paused = pause_requested.load(Ordering::Acquire);

            if let Some((ts, epoch, ack)) = pending_seek.take() {
                match Self::apply_seek(&mut ictx, &mut decoder, ts) {
                    Ok(()) => {
                        last_good_position = ts;
                        last_output_pts = None;
                        if let Ok(mut queue) = frame_queue.lock() {
                            queue.clear();
                        }
                        let _ = ack.send((epoch, None));
                    }
                    Err(err) => {
                        // A failed demuxer seek may leave FFmpeg's internal
                        // position undefined. Recover to the last decoded
                        // position; if that also fails the decoder is no
                        // longer trustworthy.
                        if Self::apply_seek(&mut ictx, &mut decoder, last_good_position).is_ok() {
                            last_output_pts = None;
                            if let Ok(mut queue) = frame_queue.lock() {
                                queue.clear();
                            }
                            let _ = ack.send((epoch, Some(err)));
                        } else {
                            decoder_state.store(DecoderState::Failed.code(), Ordering::Release);
                            let _ = ack.send((epoch, Some(err)));
                            break 'outer;
                        }
                    }
                }
            }

            if paused {
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            for (stream, packet) in ictx.packets() {
                if stop_flag.load(Ordering::Relaxed) {
                    break 'outer;
                }

                while let Ok(control) = control_rx.try_recv() {
                    match control {
                        DecoderControl::Seek((ts, epoch, ack)) => {
                            if let Some(previous) = pending_seek.take() {
                                let _ = previous.2.send((
                                    previous.1,
                                    Some(VideoError::SeekFailed(
                                        previous.0,
                                        anyhow::anyhow!("seek superseded"),
                                    )),
                                ));
                            }
                            pending_seek = Some((ts, epoch, ack));
                        }
                    }
                }
                paused = pause_requested.load(Ordering::Acquire);
                if paused || pending_seek.is_some() {
                    break;
                }

                if stream.index() != video_stream_index {
                    continue;
                }

                // A corrupt or transiently unreadable packet must not end
                // wallpaper playback: skip it and keep decoding. Previously a
                // single `send_packet` error terminated the thread, silently
                // freezing the video forever.
                if let Err(e) = decoder.send_packet(&packet) {
                    tracing::warn!("video packet skipped: {e}");
                    continue;
                }

                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    if stop_flag.load(Ordering::Relaxed) {
                        break 'outer;
                    }

                    // A full queue is backpressure, not a decoder failure. Do
                    // not block forever and do not terminate the decoder. The
                    // consumer owns presentation timing, so replace the
                    // oldest queued frame when necessary and keep the newest
                    // frame available.
                    let mut interrupted = false;
                    loop {
                        if stop_flag.load(Ordering::Relaxed) {
                            break 'outer;
                        }
                        while let Ok(control) = control_rx.try_recv() {
                            match control {
                                DecoderControl::Seek((ts, epoch, ack)) => {
                                    if let Some(previous) = pending_seek.take() {
                                        let _ = previous.2.send((
                                            previous.1,
                                            Some(VideoError::SeekFailed(
                                                previous.0,
                                                anyhow::anyhow!("seek superseded"),
                                            )),
                                        ));
                                    }
                                    pending_seek = Some((ts, epoch, ack));
                                }
                            }
                        }
                        paused = pause_requested.load(Ordering::Acquire);
                        if pending_seek.is_some() {
                            interrupted = true;
                            break;
                        }
                        let queue_full = frame_queue
                            .lock()
                            .map(|queue| queue.len() >= frame_capacity)
                            .unwrap_or(false);
                        if !paused && !queue_full {
                            break;
                        }
                        // The queue is intentionally bounded. Avoid a 1 ms
                        // busy-poll when the consumer/GPU is behind or when
                        // playback is paused; 2 ms still keeps seek/pause
                        // control responsive while cutting wakeups in half.
                        thread::sleep(Duration::from_millis(2));
                    }
                    if interrupted {
                        break;
                    }

                    let pts_duration = match pts_to_duration(
                        decoded_frame.timestamp(),
                        time_base,
                        frame_index,
                        &mut pts_origin,
                    ) {
                        Some(duration) => duration,
                        None => {
                            tracing::warn!("video frame dropped: invalid presentation timestamp");
                            continue;
                        }
                    };
                    if !frame_due_for_upload(
                        pts_duration,
                        max_output_fps.load(Ordering::Acquire),
                        last_output_pts,
                    ) {
                        frame_index = frame_index.wrapping_add(1);
                        dropped_frames.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }

                    let is_hw_frame = unsafe { !(*decoded_frame.as_ptr()).hw_frames_ctx.is_null() };

                    let src_frame = if is_hw_frame {
                        let ret = unsafe {
                            ffmpeg::ffi::av_frame_unref(sw_frame.as_mut_ptr());
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
                        hardware_frame_seen = true;
                        hw_in_use.store(used_hw.code(), Ordering::Release);
                        decoder_state
                            .store(frame_decoder_state(used_hw, true).code(), Ordering::Release);
                        &sw_frame
                    } else {
                        hw_in_use.store(HwAccel::Software.code(), Ordering::Release);
                        // Preserve initialization fallback and remember any software
                        // frames even if hardware decoding resumes later.
                        decoder_state.store(
                            observe_software_frame(used_hw, &fallback_occurred).code(),
                            Ordering::Release,
                        );
                        &decoded_frame
                    };

                    let width = src_frame.width();
                    let height = src_frame.height();
                    // Conversion failures (unexpected pixel format, hardware
                    // readback hiccup) skip the frame instead of killing the
                    // decode thread. Previous behavior froze playback forever
                    // on the first hiccup.
                    let data = match convert_frame(
                        src_frame,
                        &decoded_frame,
                        &mut scaler,
                        &mut scaler_src,
                        &mut converted_frame,
                        width,
                        height,
                        &decoder,
                    ) {
                        Ok(data) => data,
                        Err(e) => {
                            tracing::warn!("video frame conversion skipped: {e}");
                            continue;
                        }
                    };

                    let video_frame = VideoFrame {
                        data,
                        width,
                        height,
                        pts: pts_duration,
                        index: frame_index,
                    };
                    last_output_pts = Some(pts_duration);
                    frame_index = frame_index.wrapping_add(1);

                    if let Ok(mut queue) = frame_queue.lock() {
                        match push_bounded_video_frame(
                            &mut queue,
                            video_frame,
                            frame_capacity,
                            MAX_VIDEO_FRAME_QUEUE_BYTES,
                        ) {
                            Ok(dropped) => {
                                dropped_frames.fetch_add(dropped as u64, Ordering::Relaxed);
                                last_good_position = pts_duration;
                            }
                            Err(_) => {
                                dropped_frames.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }

            tracing::debug!("End of stream after {} frames, looping", frame_index);
            while decoder.receive_frame(&mut decoded_frame).is_ok() {
                if stop_flag.load(Ordering::Relaxed) {
                    break 'outer;
                }
            }
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            if loop_video && !paused {
                ictx.seek(0, ..)
                    .map_err(|e| VideoError::SeekFailed(Duration::ZERO, e.into()))?;
                decoder.flush();
                last_output_pts = None;
            } else if !paused {
                // Reached the end without looping (single play-through).
                // Stay parked so pause/seek/stop still work, and a later
                // seek restarts the stream from the requested position.
                thread::sleep(Duration::from_millis(50));
            }
        }

        if hardware_frame_seen {
            Ok(used_hw)
        } else {
            if used_hw != HwAccel::Software {
                decoder_state.store(DecoderState::SoftwareFallback.code(), Ordering::Release);
            }
            Ok(HwAccel::Software)
        }
    }

    fn apply_seek(
        ictx: &mut ffmpeg::format::context::Input,
        decoder: &mut ffmpeg::codec::decoder::Video,
        ts: Duration,
    ) -> VideoResult<()> {
        let ts_micros = ffmpeg_seek_timestamp(ts)?;
        match ictx.seek(ts_micros, ..) {
            Ok(()) => {
                decoder.flush();
                let mut drain = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut drain).is_ok() {}
                tracing::info!("Seeked to {:?}", ts);
                Ok(())
            }
            Err(e) => {
                tracing::warn!("Seek to {:?} failed: {}", ts, e);
                Err(VideoError::SeekFailed(ts, anyhow::anyhow!(e)))
            }
        }
    }

    pub fn next_frame(&self) -> Option<VideoFrame> {
        self.frame_queue.lock().ok()?.pop_front()
    }

    pub fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    pub fn decoder_info(&self) -> DecoderInfo {
        let hw_accel = self.hw_accel_in_use();
        DecoderInfo {
            codec_name: self.metadata.codec.clone(),
            hardware_accel: if hw_accel != HwAccel::Software {
                Some(hw_accel.name().to_string())
            } else {
                None
            },
            pixel_format: "NV12/RGBA".to_string(),
            dropped_frames: self.dropped_frames(),
            state: self.decoder_state(),
        }
    }

    pub fn hw_accel_in_use(&self) -> HwAccel {
        match self.hw_in_use.load(Ordering::Acquire) {
            2 => HwAccel::Vaapi,
            3 => HwAccel::Nvdec,
            4 => HwAccel::VideoToolbox,
            _ => HwAccel::Software,
        }
    }

    pub fn pause(&self) {
        self.pause_requested.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.pause_requested.store(false, Ordering::Release);
    }

    pub fn seek(&self, timestamp: Duration) -> VideoResult<()> {
        let epoch = self.seek_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.control_tx
            .send(DecoderControl::Seek((timestamp, epoch, ack_tx)))
            .map_err(|_| VideoError::ChannelDisconnected)?;
        // Wait for the decoder to finish or reject this exact request. A
        // caller-side timeout would allow the request to execute later and
        // move playback to an abandoned position, so completion is awaited.
        match ack_rx.recv() {
            Ok((ack_epoch, None)) if ack_epoch == epoch => Ok(()),
            Ok((ack_epoch, Some(err))) if ack_epoch == epoch => Err(err),
            Ok(_) => Err(VideoError::SeekFailed(
                timestamp,
                anyhow::anyhow!("stale seek acknowledgement"),
            )),
            Err(std::sync::mpsc::RecvError) => Err(VideoError::ChannelDisconnected),
        }
    }

    pub fn drain(&mut self) {
        if let Ok(mut queue) = self.frame_queue.lock() {
            queue.clear();
        }
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    pub fn decoder_state(&self) -> DecoderState {
        DecoderState::from_code(self.decoder_state.load(Ordering::Acquire))
    }

    /// Whether this decoder has ever fallen back, including before recovery.
    pub fn fallback_occurred(&self) -> bool {
        self.fallback_occurred.load(Ordering::Acquire)
    }

    pub fn is_video_file<P: AsRef<Path>>(path: P) -> bool {
        path.as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_lowercase().as_str(),
                    "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v"
                )
            })
    }
}

/// Find a decoder for this codec that advertises support for the requested
/// hardware device context. FFmpeg's default decoder can be a software-only
/// implementation (for example libdav1d), even when a hardware decoder for
/// the same codec is available.
fn hardware_decoder(id: ffmpeg::codec::Id, hw_accel: HwAccel) -> Option<ffmpeg::Codec> {
    let device_type = match hw_accel {
        HwAccel::Nvdec => ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
        HwAccel::Vaapi => ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
        HwAccel::VideoToolbox => ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
        HwAccel::Auto | HwAccel::Software => return None,
    };

    // SAFETY: FFmpeg owns the codec registry and returned codec/config pointers;
    // they remain valid for the process lifetime. The iterator is local.
    unsafe {
        let mut iterator = std::ptr::null_mut();
        loop {
            let codec = ffmpeg::ffi::av_codec_iterate(&mut iterator);
            if codec.is_null() {
                return None;
            }
            if ffmpeg::ffi::av_codec_is_decoder(codec) == 0 || (*codec).id != id.into() {
                continue;
            }

            let mut index = 0;
            loop {
                let config = ffmpeg::ffi::avcodec_get_hw_config(codec, index);
                if config.is_null() {
                    break;
                }
                if (*config).device_type == device_type
                    && (*config).methods
                        & ffmpeg::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32
                        != 0
                {
                    return Some(ffmpeg::Codec::wrap(codec));
                }
                index += 1;
            }
        }
    }
}

fn validate_video_dimensions(width: u32, height: u32) -> VideoResult<()> {
    if width > MAX_VIDEO_DIMENSION || height > MAX_VIDEO_DIMENSION {
        return Err(VideoError::UnsupportedFormat(format!(
            "video dimensions {width}x{height} exceed the {MAX_VIDEO_DIMENSION}x{MAX_VIDEO_DIMENSION} safety limit"
        )));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| VideoError::UnsupportedFormat("video dimensions overflow".to_string()))?;
    let estimated_bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| VideoError::UnsupportedFormat("video frame size overflow".to_string()))?;
    if estimated_bytes > MAX_DECODED_FRAME_BYTES {
        return Err(VideoError::UnsupportedFormat(format!(
            "decoded video frame {width}x{height} requires approximately {:.1} MiB, exceeding the {:.0} MiB safety limit",
            estimated_bytes as f64 / (1024.0 * 1024.0),
            MAX_DECODED_FRAME_BYTES as f64 / (1024.0 * 1024.0)
        )));
    }
    Ok(())
}

/// Select the hardware pixel format advertised by FFmpeg for the attached
/// device. Merely attaching `hw_device_ctx` does not make a decoder produce
/// hardware frames; FFmpeg calls this callback during codec negotiation.
unsafe extern "C" fn select_hw_pixel_format(
    context: *mut ffmpeg::ffi::AVCodecContext,
    formats: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    if context.is_null() || formats.is_null() {
        return ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }

    let device_type = unsafe {
        (*context).hw_device_ctx.as_ref().and_then(|device| {
            (!device.data.is_null())
                .then(|| (*(device.data as *const ffmpeg::ffi::AVHWDeviceContext)).type_)
        })
    };
    choose_hw_pixel_format(device_type, formats)
}

fn choose_hw_pixel_format(
    device_type: Option<ffmpeg::ffi::AVHWDeviceType>,
    formats: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    let preferred = match device_type {
        Some(ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA) => {
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_CUDA
        }
        Some(ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI) => {
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI
        }
        Some(ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX) => {
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
        }
        _ => ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE,
    };

    if preferred != ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        let mut current = formats;
        loop {
            let format = unsafe { *current };
            if format == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                break;
            }
            if format == preferred {
                return format;
            }
            current = unsafe { current.add(1) };
        }
    }

    ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

fn ensure_ffmpeg_init() -> VideoResult<()> {
    use std::sync::OnceLock;
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let result = INIT.get_or_init(|| {
        ffmpeg::init().map_err(|e| format!("FFmpeg init: {e}"))?;
        // Metadata extraction opens the input once on the calling thread so
        // a failed video never replaces active playback; the decode thread
        // then opens it a second time for the actual stream. The two opens
        // are intentional: the first validates cheaply before committing.
        Ok(())
    });
    result
        .as_ref()
        .map_err(|e| VideoError::SoftwareDecoderInit(anyhow::anyhow!(e.clone())))
        .map(|_| ())
}

/// Converts untrusted container timing to a `Duration` without panicking on
/// NaN, negative, infinite, or beyond-`Duration::MAX` values.
fn duration_from_seconds(seconds: f64) -> VideoResult<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(VideoError::InvalidDuration(format!(
            "container reported {seconds} seconds"
        )));
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| VideoError::InvalidDuration(format!("{seconds} seconds is out of range")))
}

/// Converts an untrusted presentation timestamp to a `Duration`.
///
/// Container PTS values are arbitrary user-controlled data: streams with edit
/// lists start before zero, and `AV_NOPTS_VALUE` can surface as an extreme
/// negative value. `Duration::from_secs_f64` panics on negative, NaN, and
/// out-of-range input - and the release profile builds with
/// `panic = "abort"`, so a malformed PTS would abort the daemon. Returns
/// `None` for every timestamp that cannot be represented; the decode loop
/// drops the frame and keeps playing.
fn pts_to_duration(
    timestamp: Option<i64>,
    time_base: ffmpeg::Rational,
    frame_index: u64,
    pts_origin: &mut Option<f64>,
) -> Option<Duration> {
    let pts = match timestamp {
        Some(pts) => pts,
        None => {
            // Missing PTS: fall back to the nominal position of this frame in
            // the stream. Bounds-checked against 30fps decode leftovers.
            return Duration::try_from_secs_f64(frame_index as f64 / 30.0).ok();
        }
    };
    if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
        return None;
    }
    let seconds = pts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
    if !seconds.is_finite() {
        return None;
    }
    // Some valid containers use an edit list or codec delay that makes the
    // first PTS negative. Normalize the stream against its first timestamp
    // instead of dropping those frames and appearing frozen at startup.
    let origin = *pts_origin.get_or_insert(seconds);
    let normalized = seconds - origin;
    if !normalized.is_finite() || normalized < 0.0 {
        return None;
    }
    // Frames that decode but cannot schedule anywhere (u64::MAX ms and
    // beyond) are dropped; every reasonable container publishes times far
    // below this.
    let deadline = Duration::from_secs(24 * 3600 * 3650);
    Duration::try_from_secs_f64(normalized)
        .ok()
        .filter(|duration| *duration <= deadline)
}

fn frame_due_for_upload(pts: Duration, max_fps: u32, last_pts: Option<Duration>) -> bool {
    if max_fps == 0 {
        return true;
    }
    let interval = Duration::from_secs_f64(1.0 / f64::from(max_fps));
    last_pts.is_none_or(|last| pts.saturating_sub(last) >= interval)
}

const fn software_decoder_state(hardware_attempted: bool) -> DecoderState {
    if hardware_attempted {
        DecoderState::SoftwareFallback
    } else {
        DecoderState::SoftwareActive
    }
}

const fn frame_decoder_state(selected_backend: HwAccel, hardware_frame: bool) -> DecoderState {
    if hardware_frame {
        DecoderState::HardwareActive
    } else if matches!(selected_backend, HwAccel::Software) {
        DecoderState::SoftwareActive
    } else {
        DecoderState::SoftwareFallback
    }
}

fn observe_software_frame(selected_backend: HwAccel, fallback: &AtomicBool) -> DecoderState {
    if selected_backend != HwAccel::Software {
        fallback.store(true, Ordering::Release);
    }
    software_decoder_state(fallback.load(Ordering::Acquire))
}

/// True when any `/sys/class/drm/card*` sibling reports NVIDIA vendor 0x10de.
fn nvidia_gpu_present() -> bool {
    any_nvidia_card(std::path::Path::new("/sys/class/drm"))
}

fn any_nvidia_card(root: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") {
            return false;
        }
        std::fs::read_to_string(entry.path().join("device/vendor"))
            .is_ok_and(|vendor| vendor.trim().eq_ignore_ascii_case("0x10de"))
    })
}

/// True when a usable DRM render node exists (`/dev/dri/renderD*`).
fn vaapi_available() -> bool {
    any_render_node(std::path::Path::new("/dev/dri"))
}

fn any_render_node(root: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries
        .filter_map(|e| e.ok())
        .any(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
}

/// VA-API render-node candidates in probe order. Enumerates
/// `/dev/dri/renderD*` so hybrid/multi-GPU systems try every node instead
/// of assuming renderD128; falls back to renderD128 when enumeration fails.
fn vaapi_device_candidates() -> Vec<CString> {
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/dri") {
        let mut nodes: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("renderD"))
            .collect();
        nodes.sort();
        for node in nodes {
            let path = format!("/dev/dri/{node}");
            if let Ok(c) = CString::new(path) {
                candidates.push(c);
            }
        }
    }
    if candidates.is_empty()
        && let Ok(fallback) = CString::new("/dev/dri/renderD128")
    {
        candidates.push(fallback);
    }
    candidates
}

fn copy_packed_rows(
    source: &[u8],
    stride: usize,
    row_bytes: usize,
    height: usize,
) -> VideoResult<Vec<u8>> {
    let invalid = |message: String| VideoError::FormatConversionFailed(anyhow::anyhow!(message));
    if height == 0 {
        return Err(invalid("frame has zero height".to_string()));
    }
    if stride < row_bytes {
        return Err(invalid(format!(
            "frame stride {stride} is smaller than row width {row_bytes}"
        )));
    }
    let needed = stride
        .checked_mul(height)
        .ok_or_else(|| invalid("frame stride overflow".to_string()))?;
    if source.len() < needed {
        return Err(invalid(format!(
            "frame buffer {} is smaller than required {needed}",
            source.len()
        )));
    }
    let capacity = row_bytes
        .checked_mul(height)
        .ok_or_else(|| invalid("frame size overflow".to_string()))?;
    let mut packed = Vec::with_capacity(capacity);
    for row in source.chunks(stride).take(height) {
        packed.extend_from_slice(&row[..row_bytes]);
    }
    Ok(packed)
}

/// Converts decoded frames to NV12, avoiding a full-resolution CPU RGB
/// conversion and reducing queued/uploaded frame bytes versus RGBA.
#[allow(clippy::too_many_arguments)]
fn convert_frame(
    src_frame: &ffmpeg::frame::Video,
    decoded_frame: &ffmpeg::frame::Video,
    scaler: &mut Option<ffmpeg::software::scaling::Context>,
    scaler_src: &mut Option<ffmpeg::format::Pixel>,
    converted_frame: &mut ffmpeg::frame::Video,
    width: u32,
    height: u32,
    decoder: &ffmpeg::codec::decoder::Video,
) -> VideoResult<VideoFrameData> {
    if src_frame.format() == ffmpeg::format::Pixel::NV12 {
        let color_space = match decoded_frame.color_space() {
            ffmpeg::color::Space::Unspecified => decoder.color_space(),
            value => value,
        };
        let color_range = match decoded_frame.color_range() {
            ffmpeg::color::Range::Unspecified => decoder.color_range(),
            value => value,
        };
        let color = select_yuv_color(color_space, color_range, width, height);
        let (y_plane, uv_plane) = copy_nv12_planes(src_frame)?;
        Ok(VideoFrameData::Nv12 {
            y_plane,
            uv_plane,
            color,
        })
    } else {
        let src_format = src_frame.format();
        if *scaler_src != Some(src_format) {
            *scaler = Some(
                ffmpeg::software::scaling::context::Context::get(
                    src_format,
                    width,
                    height,
                    ffmpeg::format::Pixel::NV12,
                    width,
                    height,
                    ffmpeg::software::scaling::Flags::FAST_BILINEAR,
                )
                .map_err(|e| VideoError::FormatConversionFailed(e.into()))?,
            );
            *scaler_src = Some(src_format);
        }

        scaler
            .as_mut()
            .expect("scaler initialized above")
            .run(src_frame, converted_frame)
            .map_err(|e| VideoError::FormatConversionFailed(e.into()))?;
        let color_space = match decoded_frame.color_space() {
            ffmpeg::color::Space::Unspecified => decoder.color_space(),
            value => value,
        };
        let color_range = match decoded_frame.color_range() {
            ffmpeg::color::Range::Unspecified => decoder.color_range(),
            value => value,
        };
        let color = select_yuv_color(color_space, color_range, width, height);
        let (y_plane, uv_plane) = copy_nv12_planes(converted_frame)?;
        Ok(VideoFrameData::Nv12 {
            y_plane,
            uv_plane,
            color,
        })
    }
}

fn copy_nv12_planes(frame: &ffmpeg::frame::Video) -> VideoResult<(Vec<u8>, Vec<u8>)> {
    copy_nv12_data(
        frame.data(0),
        frame.stride(0),
        frame.data(1),
        frame.stride(1),
        frame.width() as usize,
        frame.height() as usize,
    )
}

fn copy_nv12_data(
    y_source: &[u8],
    y_stride: usize,
    uv_source: &[u8],
    uv_stride: usize,
    width: usize,
    height: usize,
) -> VideoResult<(Vec<u8>, Vec<u8>)> {
    if width == 0 || height == 0 {
        return Err(VideoError::FormatConversionFailed(anyhow::anyhow!(
            "NV12 frame is empty"
        )));
    }
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let uv_row = chroma_width.checked_mul(2).ok_or_else(|| {
        VideoError::FormatConversionFailed(anyhow::anyhow!("NV12 chroma row overflow"))
    })?;
    Ok((
        copy_packed_rows(y_source, y_stride, width, height)?,
        copy_packed_rows(uv_source, uv_stride, uv_row, chroma_height)?,
    ))
}

fn select_yuv_color(
    space: ffmpeg::color::Space,
    range: ffmpeg::color::Range,
    width: u32,
    height: u32,
) -> YuvColorInfo {
    let matrix = match space {
        ffmpeg::color::Space::BT470BG | ffmpeg::color::Space::SMPTE170M => YuvMatrix::Bt601,
        ffmpeg::color::Space::BT709 => YuvMatrix::Bt709,
        ffmpeg::color::Space::BT2020NCL => YuvMatrix::Bt2020,
        _ if width >= 1280 || height > 576 => YuvMatrix::Bt709,
        _ => YuvMatrix::Bt601,
    };
    let range = match range {
        ffmpeg::color::Range::JPEG => YuvRange::Full,
        ffmpeg::color::Range::MPEG | ffmpeg::color::Range::Unspecified => YuvRange::Limited,
    };
    YuvColorInfo { matrix, range }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(thread) = self.decode_thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_fps_cap_skips_expensive_frame_preparation() {
        let last = Duration::ZERO;
        assert!(!frame_due_for_upload(
            Duration::from_millis(16),
            30,
            Some(last)
        ));
        assert!(frame_due_for_upload(
            Duration::from_millis(34),
            30,
            Some(last)
        ));
        assert!(frame_due_for_upload(
            Duration::from_millis(16),
            0,
            Some(last)
        ));
        assert!(frame_due_for_upload(Duration::ZERO, 30, None));
    }

    #[test]
    fn nvidia_probe_detects_vendor_via_sysfs_layout() {
        let temp = tempfile::tempdir().expect("tempdir");
        let card0 = temp.path().join("card0");
        let card1 = temp.path().join("card1");

        std::fs::create_dir_all(card0.join("device")).expect("dirs");
        std::fs::create_dir_all(card1.join("device")).expect("dirs");
        assert!(!any_nvidia_card(temp.path()));

        std::fs::write(card1.join("device/vendor"), "0x10de\n").expect("vendor");
        assert!(any_nvidia_card(temp.path()));

        std::fs::write(card1.join("device/vendor"), "0x1002\n").expect("vendor");
        assert!(!any_nvidia_card(temp.path()));
    }

    #[test]
    fn render_node_probe_detects_render_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(!any_render_node(temp.path()));
        std::fs::create_dir_all(temp.path().join("renderD128")).expect("create");
        assert!(any_render_node(temp.path()));
    }

    #[test]
    fn ffmpeg_seek_timestamp_uses_microseconds() {
        assert_eq!(ffmpeg_seek_timestamp(Duration::ZERO).unwrap(), 0);
        assert_eq!(
            ffmpeg_seek_timestamp(Duration::from_secs(1)).unwrap(),
            1_000_000
        );
        assert_eq!(
            ffmpeg_seek_timestamp(Duration::from_millis(1500)).unwrap(),
            1_500_000
        );
    }

    #[test]
    fn ffmpeg_seek_timestamp_rejects_overflow() {
        let huge = Duration::from_secs(u64::MAX);
        assert!(ffmpeg_seek_timestamp(huge).is_err());
    }

    #[test]
    fn bounded_video_frame_queue_enforces_count_and_bytes() {
        let mut queue = std::collections::VecDeque::new();
        let frame = |size: usize| VideoFrame {
            data: VideoFrameData::Rgba(vec![0; size]),
            width: 1,
            height: 1,
            pts: Duration::ZERO,
            index: 0,
        };

        assert_eq!(
            push_bounded_video_frame(&mut queue, frame(4), 2, 10).unwrap(),
            0
        );
        assert_eq!(
            push_bounded_video_frame(&mut queue, frame(4), 2, 10).unwrap(),
            0
        );
        assert_eq!(
            push_bounded_video_frame(&mut queue, frame(4), 2, 10).unwrap(),
            1
        );
        assert_eq!(queue.len(), 2);
        assert_eq!(queued_video_frame_bytes(&queue), 8);

        assert!(push_bounded_video_frame(&mut queue, frame(11), 2, 10).is_err());
    }

    #[test]
    fn real_video_seek_lands_near_requested_timestamp() {
        let Some((_temp, path)) = make_test_video() else {
            return;
        };
        let decoder = VideoDecoder::new(&path, HwAccel::Software).expect("decoder");
        assert!(wait_for_frame(&decoder, Duration::from_secs(3)).is_some());

        decoder
            .seek(Duration::from_millis(1000))
            .expect("seek middle");
        let frame = wait_for_frame(&decoder, Duration::from_secs(3)).expect("frame after seek");
        assert!(
            frame.pts >= Duration::from_millis(900) && frame.pts <= Duration::from_millis(1400),
            "frame pts {:?} was not near 1s",
            frame.pts
        );

        // A failed seek must not poison later seeks.
        assert!(decoder.seek(Duration::from_secs(u64::MAX)).is_err());
        decoder
            .seek(Duration::from_millis(1500))
            .expect("seek after failure");
        let frame = wait_for_frame(&decoder, Duration::from_secs(3)).expect("frame after failure");
        assert!(
            frame.pts >= Duration::from_millis(1400),
            "frame pts {:?} was not near 1.5s",
            frame.pts
        );

        // A second seek must not return frames from the first position.
        decoder.seek(Duration::ZERO).expect("seek beginning");
        let frame = wait_for_frame(&decoder, Duration::from_secs(3)).expect("frame after rewind");
        assert!(
            frame.pts <= Duration::from_millis(300),
            "frame pts {:?} was not near the beginning",
            frame.pts
        );
    }

    fn make_test_video() -> Option<(tempfile::TempDir, std::path::PathBuf)> {
        let temp = tempfile::tempdir().ok()?;
        let path = temp.path().join("seek-test.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=64x64:rate=10",
                "-c:v",
                "mpeg4",
                "-g",
                "1",
                "-q:v",
                "2",
                "-y",
            ])
            .arg(&path)
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
        Some((temp, path))
    }

    fn wait_for_frame(decoder: &VideoDecoder, timeout: Duration) -> Option<VideoFrame> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(frame) = decoder.next_frame() {
                return Some(frame);
            }
            if decoder.decoder_state() == DecoderState::Failed {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

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
    fn decoder_states_have_stable_diagnostic_names() {
        assert_eq!(
            DecoderState::HardwareNegotiating.name(),
            "hardware negotiation"
        );
        assert_eq!(DecoderState::HardwareActive.name(), "hardware active");
        assert_eq!(DecoderState::SoftwareFallback.name(), "software fallback");
        assert_eq!(
            DecoderState::from_code(DecoderState::Failed.code()),
            DecoderState::Failed
        );
        assert_eq!(software_decoder_state(true), DecoderState::SoftwareFallback);
        assert_eq!(software_decoder_state(false), DecoderState::SoftwareActive);
    }

    #[test]
    fn software_frame_from_hardware_decoder_reports_fallback() {
        assert_eq!(
            frame_decoder_state(HwAccel::Nvdec, false),
            DecoderState::SoftwareFallback
        );
        assert_eq!(
            frame_decoder_state(HwAccel::Nvdec, true),
            DecoderState::HardwareActive
        );
        assert_eq!(
            frame_decoder_state(HwAccel::Software, false),
            DecoderState::SoftwareActive
        );
    }

    #[test]
    fn fallback_history_survives_recovery_and_software_frames() {
        let fallback = AtomicBool::new(false);
        assert_eq!(
            observe_software_frame(HwAccel::Software, &fallback),
            DecoderState::SoftwareActive
        );
        assert!(!fallback.load(Ordering::Acquire));
        assert_eq!(
            observe_software_frame(HwAccel::Nvdec, &fallback),
            DecoderState::SoftwareFallback
        );
        assert_eq!(
            frame_decoder_state(HwAccel::Nvdec, true),
            DecoderState::HardwareActive
        );
        assert!(fallback.load(Ordering::Acquire));
        assert_eq!(
            observe_software_frame(HwAccel::Software, &fallback),
            DecoderState::SoftwareFallback
        );
    }

    #[test]
    fn failed_decoder_cannot_pass_after_producing_frames() {
        assert!(!probe_succeeded(300, DecoderState::Failed));
        assert!(!probe_succeeded(30, DecoderState::SoftwareActive));
        assert!(probe_succeeded(31, DecoderState::SoftwareActive));
        assert!(probe_succeeded(300, DecoderState::HardwareActive));
    }

    #[test]
    fn probe_fails_on_low_frames_even_when_hardware_active() {
        assert!(!probe_succeeded(0, DecoderState::HardwareActive));
        assert!(!probe_succeeded(
            PROBE_MIN_FRAMES,
            DecoderState::HardwareActive
        ));
        assert!(probe_succeeded(
            PROBE_MIN_FRAMES + 1,
            DecoderState::HardwareActive
        ));
    }

    #[test]
    fn test_hwaccel_from_config() {
        assert_eq!(HwAccel::from_config("vaapi"), HwAccel::Vaapi);
        assert_eq!(HwAccel::from_config("nvdec"), HwAccel::Nvdec);
        assert_eq!(HwAccel::from_config("software"), HwAccel::Software);
        assert_eq!(HwAccel::from_config("auto"), HwAccel::Auto);
        assert_eq!(HwAccel::from_config("unknown"), HwAccel::Auto);
    }

    #[test]
    fn packs_strided_rows() {
        let y = [1, 2, 3, 99, 99, 4, 5, 6, 99, 99, 7, 8, 9, 99, 99];
        let uv = [10, 11, 12, 13, 99, 99, 14, 15, 16, 17, 99, 99];
        let (packed_y, packed_uv) = copy_nv12_data(&y, 5, &uv, 6, 3, 3).unwrap();
        assert_eq!(packed_y, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(packed_uv, [10, 11, 12, 13, 14, 15, 16, 17]);
    }

    #[test]
    fn selects_yuv_matrix_and_range() {
        assert_eq!(
            select_yuv_color(
                ffmpeg::color::Space::BT2020NCL,
                ffmpeg::color::Range::JPEG,
                3840,
                2160,
            ),
            YuvColorInfo {
                matrix: YuvMatrix::Bt2020,
                range: YuvRange::Full,
            }
        );
        assert_eq!(
            select_yuv_color(
                ffmpeg::color::Space::Unspecified,
                ffmpeg::color::Range::Unspecified,
                1920,
                1080,
            ),
            YuvColorInfo {
                matrix: YuvMatrix::Bt709,
                range: YuvRange::Limited,
            }
        );
        assert_eq!(
            select_yuv_color(
                ffmpeg::color::Space::Unspecified,
                ffmpeg::color::Range::MPEG,
                720,
                576,
            ),
            YuvColorInfo {
                matrix: YuvMatrix::Bt601,
                range: YuvRange::Limited,
            }
        );
    }

    #[test]
    fn hardware_format_negotiation_selects_only_the_matching_backend() {
        let formats = [
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P,
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_CUDA,
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE,
        ];
        assert_eq!(
            choose_hw_pixel_format(
                Some(ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA),
                formats.as_ptr()
            ),
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_CUDA
        );

        let software_only = [
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P,
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE,
        ];
        assert_eq!(
            choose_hw_pixel_format(
                Some(ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA),
                software_only.as_ptr()
            ),
            ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE
        );
    }
    #[test]
    fn rejects_truncated_or_inverted_frame_rows() {
        assert!(copy_packed_rows(&[0; 10], 5, 6, 2).is_err());
        assert!(copy_packed_rows(&[0; 10], 5, 5, 3).is_err());
        assert!(copy_packed_rows(&[0; 10], 4, 5, 2).is_err());
        assert!(copy_packed_rows(&[0; 15], 5, 3, 3).is_ok());
    }
    #[test]
    fn untrusted_duration_values_never_panic() {
        for value in [f64::NAN, f64::INFINITY, -1.0, f64::MAX] {
            assert!(duration_from_seconds(value).is_err(), "{value}");
        }
        assert_eq!(
            duration_from_seconds(1.5).unwrap(),
            Duration::from_millis(1500)
        );
        assert_eq!(duration_from_seconds(0.0).unwrap(), Duration::ZERO);
    }
}
#[test]
fn normalizes_negative_start_pts_without_dropping_frames() {
    let time_base = ffmpeg::Rational(1, 1000);
    let mut origin = None;
    assert_eq!(
        pts_to_duration(Some(-80), time_base, 0, &mut origin),
        Some(Duration::ZERO)
    );
    assert_eq!(
        pts_to_duration(Some(-64), time_base, 1, &mut origin),
        Some(Duration::from_millis(16))
    );
}

#[test]
fn accepts_8k_frames_but_rejects_unbounded_allocations() {
    assert!(validate_video_dimensions(7680, 4320).is_ok());
    assert!(validate_video_dimensions(16_384, 16_384).is_err());
    assert!(validate_video_dimensions(20_000, 1).is_err());
}
