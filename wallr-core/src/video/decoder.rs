use crate::video::error::{VideoError, VideoResult};
use crate::video::scheduler::ScheduledFrame;
use crossbeam_channel::{Receiver, SendTimeoutError, Sender};
use ffmpeg_next as ffmpeg;
use std::ffi::{CString, c_char};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwAccel {
    /// Try all hardware backends in priority order, then software
    Auto,
    Vaapi,
    Nvdec,
    VideoToolbox,
    Software,
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

    const fn from_code(code: u8) -> HwAccel {
        match code {
            2 => HwAccel::Vaapi,
            3 => HwAccel::Nvdec,
            4 => HwAccel::VideoToolbox,
            1 => HwAccel::Software,
            _ => HwAccel::Auto,
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

    /// All hardware backends in priority order for auto-detection fallback.
    /// NVDEC preferred on Linux (common primary GPU on hybrid systems),
    /// then VAAPI, then VideoToolbox on macOS.
    fn all_hardware() -> &'static [HwAccel] {
        &[HwAccel::Nvdec, HwAccel::Vaapi, HwAccel::VideoToolbox]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub data: VideoFrameData,
    pub width: u32,
    pub height: u32,
    pub pts: Duration,
    pub index: u64,
}

impl VideoFrame {
    pub fn into_scheduled(self) -> ScheduledFrame {
        ScheduledFrame::new(self.data, self.width, self.height, self.pts, self.index)
    }
}

#[derive(Debug, Clone)]
pub struct DecoderInfo {
    pub codec_name: String,
    pub hardware_accel: Option<String>,
    pub pixel_format: String,
}

#[derive(Debug, Clone, Copy)]
enum DecoderControl {
    Pause,
    Resume,
    Seek(Duration, u64),
}

pub struct VideoDecoder {
    metadata: VideoMetadata,
    frame_rx: Receiver<VideoFrame>,
    control_tx: Sender<DecoderControl>,
    stop_flag: Arc<AtomicBool>,
    seek_epoch: Arc<AtomicU64>,
    hw_in_use: Arc<AtomicU8>,
    decode_thread: Option<thread::JoinHandle<()>>,
}

impl VideoDecoder {
    pub fn new<P: AsRef<Path>>(path: P, hw_accel: HwAccel) -> VideoResult<Self> {
        Self::with_preload(path, hw_accel, 2)
    }

    pub fn with_preload<P: AsRef<Path>>(
        path: P,
        hw_accel: HwAccel,
        preload_frames: usize,
    ) -> VideoResult<Self> {
        let path = path.as_ref().to_path_buf();

        ffmpeg::init()
            .map_err(|e| VideoError::SoftwareDecoderInit(anyhow::anyhow!("FFmpeg init: {}", e)))?;

        let metadata = Self::extract_metadata(&path)?;

        tracing::info!(
            "Opened video: {}x{} @ {:.2} fps, duration: {:?}, codec: {}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration,
            metadata.codec
        );

        let (frame_tx, frame_rx) = crossbeam_channel::bounded(preload_frames.max(1));
        let (control_tx, control_rx) = crossbeam_channel::unbounded();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = stop_flag.clone();
        let seek_epoch = Arc::new(AtomicU64::new(0));
        let seek_epoch_clone = seek_epoch.clone();
        let hw_in_use = Arc::new(AtomicU8::new(0));
        let hw_in_use_clone = hw_in_use.clone();

        let decode_thread = thread::Builder::new()
            .name("wallr-video-decoder".to_string())
            .spawn(move || {
                let used = Self::decode_loop(
                    path,
                    hw_accel,
                    frame_tx,
                    control_rx,
                    stop_flag_clone,
                    seek_epoch_clone,
                    hw_in_use_clone.clone(),
                );
                let used = match used {
                    Ok(used) => used,
                    Err(e) => {
                        tracing::error!("Video decode loop: {}", e);
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
            frame_rx,
            control_tx,
            stop_flag,
            seek_epoch,
            hw_in_use,
            decode_thread: Some(decode_thread),
        })
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
        let fps = if frame_rate.numerator() > 0 {
            frame_rate.numerator() as f64 / frame_rate.denominator() as f64
        } else {
            30.0
        };

        let duration = {
            let duration_ts = stream.duration();
            let time_base = stream.time_base();
            if duration_ts > 0 {
                Duration::from_secs_f64(
                    duration_ts as f64 * time_base.numerator() as f64
                        / time_base.denominator() as f64,
                )
            } else {
                let container_duration = ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;
                Duration::from_secs_f64(container_duration)
            }
        };

        let total_frames = if fps > 0.0 {
            (duration.as_secs_f64() * fps) as u64
        } else {
            0
        };

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
        let (type_name, device) = match hw_accel {
            HwAccel::Vaapi => ("vaapi", Some(c"/dev/dri/renderD128")),
            HwAccel::Nvdec => ("cuda", Some(c"0")),
            HwAccel::VideoToolbox => ("videotoolbox", None),
            HwAccel::Software | HwAccel::Auto => return None,
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

    /// Try to build a decoder with a hardware device context attached.
    fn try_hw_decoder(
        stream: &ffmpeg::format::stream::Stream,
        hw_accel: HwAccel,
    ) -> Option<(ffmpeg::codec::decoder::Video, HwAccel)> {
        let (device_ctx, _) = Self::init_hw_device(hw_accel)?;

        let mut context =
            ffmpeg::codec::context::Context::from_parameters(stream.parameters()).ok()?;

        // SAFETY: device_ctx is a valid FFmpeg allocation; the codec context
        // unrefs it when dropped.
        unsafe {
            (*context.as_mut_ptr()).hw_device_ctx = device_ctx;
        }

        match context.decoder().video() {
            Ok(decoder) => {
                tracing::info!("Hardware decode active: {}", hw_accel.name());
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
    ) -> (ffmpeg::codec::decoder::Video, HwAccel) {
        match hw_accel {
            HwAccel::Auto => {
                // Try all hardware backends in priority order
                for &backend in HwAccel::all_hardware() {
                    if let Some(result) = Self::try_hw_decoder(stream, backend) {
                        return result;
                    }
                }
                // Fall back to software
                tracing::info!("All hardware backends failed, using software decoder");
            }
            HwAccel::Software => {
                // Explicit software request: skip hardware entirely
                tracing::info!("Software decoder explicitly requested");
            }
            specific => {
                // Try the requested hardware backend first
                if let Some(result) = Self::try_hw_decoder(stream, specific) {
                    return result;
                }
                // Fall back to software
                tracing::info!(
                    "{} hardware decoder failed, falling back to software",
                    specific.name()
                );
            }
        }

        // Software fallback.
        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .and_then(|ctx| ctx.decoder().video())
            .expect("software decoder must be available");
        (decoder, HwAccel::Software)
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_loop(
        path: std::path::PathBuf,
        hw_accel: HwAccel,
        frame_tx: Sender<VideoFrame>,
        control_rx: Receiver<DecoderControl>,
        stop_flag: Arc<AtomicBool>,
        seek_epoch: Arc<AtomicU64>,
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
        tracing::info!("Decoder in use: {}", used_hw.name());

        // Report the active backend immediately after successful initialization
        hw_in_use.store(used_hw.code(), Ordering::Relaxed);

        let mut scaler: Option<ffmpeg::software::scaling::Context> = None;
        let mut scaler_src: Option<ffmpeg::format::Pixel> = None;

        let mut paused = false;
        let mut pending_seek: Option<(Duration, u64)> = None;
        let mut applied_seek_epoch = 0;
        let mut frame_index = 0u64;
        let mut decoded_frame = ffmpeg::frame::Video::empty();
        let mut sw_frame = ffmpeg::frame::Video::empty();
        let mut rgb_frame = ffmpeg::frame::Video::empty();

        'outer: loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            while let Ok(control) = control_rx.try_recv() {
                match control {
                    DecoderControl::Pause => paused = true,
                    DecoderControl::Resume => paused = false,
                    DecoderControl::Seek(ts, epoch) => pending_seek = Some((ts, epoch)),
                }
            }

            if let Some((ts, epoch)) = pending_seek.take() {
                Self::apply_seek(&mut ictx, &mut decoder, time_base, ts);
                applied_seek_epoch = epoch;
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
                        DecoderControl::Pause => paused = true,
                        DecoderControl::Resume => paused = false,
                        DecoderControl::Seek(ts, epoch) => pending_seek = Some((ts, epoch)),
                    }
                }
                if paused || pending_seek.is_some() {
                    break;
                }

                if stream.index() != video_stream_index {
                    continue;
                }

                decoder
                    .send_packet(&packet)
                    .map_err(|e| VideoError::DecodeFailed(anyhow::anyhow!("send_packet: {}", e)))?;

                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    if stop_flag.load(Ordering::Relaxed) {
                        break 'outer;
                    }

                    while let Ok(control) = control_rx.try_recv() {
                        match control {
                            DecoderControl::Pause => paused = true,
                            DecoderControl::Resume => paused = false,
                            DecoderControl::Seek(ts, epoch) => pending_seek = Some((ts, epoch)),
                        }
                    }
                    if paused
                        || pending_seek.is_some()
                        || seek_epoch.load(Ordering::Acquire) != applied_seek_epoch
                    {
                        break;
                    }

                    // Apply backpressure before GPU readback and color
                    // conversion. Dropping here would let the decoder race
                    // through the file at hundreds of FPS while the renderer
                    // is paced to presentation.
                    let mut interrupted = false;
                    while frame_tx.is_full() {
                        if stop_flag.load(Ordering::Relaxed) {
                            break 'outer;
                        }
                        while let Ok(control) = control_rx.try_recv() {
                            match control {
                                DecoderControl::Pause => paused = true,
                                DecoderControl::Resume => paused = false,
                                DecoderControl::Seek(ts, epoch) => {
                                    pending_seek = Some((ts, epoch));
                                }
                            }
                        }
                        if paused || pending_seek.is_some() {
                            interrupted = true;
                            break;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    if interrupted {
                        break;
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
                        &sw_frame
                    } else {
                        &decoded_frame
                    };

                    let pts_duration = if let Some(pts) = decoded_frame.timestamp() {
                        Duration::from_secs_f64(
                            pts as f64 * time_base.numerator() as f64
                                / time_base.denominator() as f64,
                        )
                    } else {
                        Duration::from_secs_f64(frame_index as f64 / 30.0)
                    };

                    let width = src_frame.width();
                    let height = src_frame.height();
                    let data = if src_frame.format() == ffmpeg::format::Pixel::NV12 {
                        let color_space = match decoded_frame.color_space() {
                            ffmpeg::color::Space::Unspecified => decoder.color_space(),
                            value => value,
                        };
                        let color_range = match decoded_frame.color_range() {
                            ffmpeg::color::Range::Unspecified => decoder.color_range(),
                            value => value,
                        };
                        let color = select_yuv_color(color_space, color_range, width, height);
                        let (y_plane, uv_plane) = copy_nv12_planes(src_frame);
                        VideoFrameData::Nv12 {
                            y_plane,
                            uv_plane,
                            color,
                        }
                    } else {
                        let src_format = src_frame.format();
                        if scaler_src != Some(src_format) {
                            scaler = Some(
                                ffmpeg::software::scaling::context::Context::get(
                                    src_format,
                                    width,
                                    height,
                                    ffmpeg::format::Pixel::RGBA,
                                    width,
                                    height,
                                    ffmpeg::software::scaling::Flags::BILINEAR,
                                )
                                .map_err(|e| VideoError::FormatConversionFailed(e.into()))?,
                            );
                            scaler_src = Some(src_format);
                        }

                        scaler
                            .as_mut()
                            .expect("scaler initialized above")
                            .run(src_frame, &mut rgb_frame)
                            .map_err(|e| VideoError::FormatConversionFailed(e.into()))?;
                        VideoFrameData::Rgba(copy_packed_rows(
                            rgb_frame.data(0),
                            rgb_frame.stride(0),
                            rgb_frame.width() as usize * 4,
                            rgb_frame.height() as usize,
                        ))
                    };

                    let video_frame = VideoFrame {
                        data,
                        width,
                        height,
                        pts: pts_duration,
                        index: frame_index,
                    };
                    frame_index = frame_index.wrapping_add(1);

                    match frame_tx.send_timeout(video_frame, Duration::from_millis(20)) {
                        Ok(()) => {}
                        Err(SendTimeoutError::Timeout(_)) => break,
                        Err(SendTimeoutError::Disconnected(_)) => {
                            tracing::warn!("Frame queue disconnected, ending decode loop");
                            return Ok(used_hw);
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
            if !paused {
                ictx.seek(0, ..)
                    .map_err(|e| VideoError::SeekFailed(Duration::ZERO, e.into()))?;
                decoder.flush();
            }
        }

        Ok(used_hw)
    }

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
                let mut drain = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut drain).is_ok() {}
                tracing::info!("Seeked to {:?}", ts);
            }
            Err(e) => tracing::warn!("Seek to {:?} failed: {}", ts, e),
        }
    }

    pub fn next_frame(&self) -> Option<VideoFrame> {
        self.frame_rx.try_recv().ok()
    }

    pub fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    pub fn decoder_info(&self) -> DecoderInfo {
        DecoderInfo {
            codec_name: self.metadata.codec.clone(),
            hardware_accel: if self.hw_accel_in_use() != HwAccel::Software {
                Some(self.hw_accel_in_use().name().to_string())
            } else {
                None
            },
            pixel_format: "NV12/RGBA".to_string(),
        }
    }

    pub fn hw_accel_in_use(&self) -> HwAccel {
        for _ in 0..50 {
            let code = self.hw_in_use.load(Ordering::Relaxed);
            if code != 0 {
                return HwAccel::from_code(code);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        HwAccel::Software
    }

    pub fn pause(&self) {
        let _ = self.control_tx.send(DecoderControl::Pause);
    }

    pub fn resume(&self) {
        let _ = self.control_tx.send(DecoderControl::Resume);
    }

    pub fn seek(&self, timestamp: Duration) {
        let epoch = self.seek_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.control_tx.send(DecoderControl::Seek(timestamp, epoch));
    }

    pub fn drain(&mut self) {
        while self.frame_rx.try_recv().is_ok() {}
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

fn copy_packed_rows(source: &[u8], stride: usize, row_bytes: usize, height: usize) -> Vec<u8> {
    let mut packed = Vec::with_capacity(row_bytes * height);
    for row in source.chunks(stride).take(height) {
        packed.extend_from_slice(&row[..row_bytes]);
    }
    packed
}

fn copy_nv12_planes(frame: &ffmpeg::frame::Video) -> (Vec<u8>, Vec<u8>) {
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
) -> (Vec<u8>, Vec<u8>) {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    (
        copy_packed_rows(y_source, y_stride, width, height),
        copy_packed_rows(uv_source, uv_stride, chroma_width * 2, chroma_height),
    )
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
        assert_eq!(HwAccel::from_config("auto"), HwAccel::Auto);
        assert_eq!(HwAccel::from_config("unknown"), HwAccel::Auto);
    }

    #[test]
    fn packs_strided_rows() {
        let y = [1, 2, 3, 99, 99, 4, 5, 6, 99, 99, 7, 8, 9, 99, 99];
        let uv = [10, 11, 12, 13, 99, 99, 14, 15, 16, 17, 99, 99];
        let (packed_y, packed_uv) = copy_nv12_data(&y, 5, &uv, 6, 3, 3);
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
}
