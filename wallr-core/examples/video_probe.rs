//! Standalone video decode probe.
//!
//! Usage: `cargo run --release --example video_probe -- <video> [vaapi|nvdec|software]`
//!
//! Decodes the file for a few seconds and reports frame rate, timing, and the
//! backend actually used - no Wayland or GPU required.

use std::process::ExitCode;
use std::time::{Duration, Instant};
use wallr_core::video::{DecoderState, HwAccel, VideoDecoder};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: video_probe <video> [backend]");
    if path == "--capabilities" {
        print_capabilities();
        return ExitCode::SUCCESS;
    }
    let backend = match args.next().as_deref() {
        Some("vaapi") => HwAccel::Vaapi,
        Some("nvdec") => HwAccel::Nvdec,
        Some("software") | Some("sw") => HwAccel::Software,
        _ => HwAccel::Software,
    };

    let decoder = match VideoDecoder::new(&path, backend) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to init decoder: {e}");
            return ExitCode::FAILURE;
        }
    };

    let meta = decoder.metadata().clone();
    println!(
        "file: {path}\nresolution: {}x{}\nfps: {:.2}\nduration: {:?}\ncodec: {}\ncontainer: {}",
        meta.width, meta.height, meta.fps, meta.duration, meta.codec, meta.format
    );
    println!("requested backend: {}", backend.name());

    let mut count = 0u64;
    let mut dropped = 0u64;
    let start = Instant::now();
    let deadline = Duration::from_secs(5);

    while start.elapsed() < deadline && count < 300 {
        match decoder.next_frame() {
            Some(frame) => {
                count += 1;
                if count % 25 == 1 {
                    println!(
                        "  frame {count}: pts={:?} {}x{} bytes={}",
                        frame.pts,
                        frame.width,
                        frame.height,
                        frame.data.len()
                    );
                }
            }
            None => {
                dropped += 1;
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    let elapsed = start.elapsed();
    let fps = count as f64 / elapsed.as_secs_f64();
    println!(
        "decoded {count} frames in {elapsed:?} ({fps:.1} fps, {} idle polls)",
        dropped
    );
    let state = decoder.decoder_state();
    println!(
        "active backend: {} (state: {}, dropped frames: {})",
        decoder.hw_accel_in_use().name(),
        state.name(),
        decoder.dropped_frames()
    );
    println!("fallback occurred: {}", decoder.fallback_occurred());
    let passed = probe_passed(count, state);
    println!("result: {}", if passed { "PASS" } else { "FAIL" });
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn probe_passed(count: u64, state: DecoderState) -> bool {
    count > 30 && state != DecoderState::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_decoder_cannot_pass_after_producing_frames() {
        assert!(!probe_passed(300, DecoderState::Failed));
        assert!(!probe_passed(30, DecoderState::SoftwareActive));
        assert!(probe_passed(31, DecoderState::SoftwareActive));
        assert!(probe_passed(300, DecoderState::HardwareActive));
    }
}

fn print_capabilities() {
    println!(
        "cuda: {}",
        if h264_supports_pixel_format(ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_CUDA) {
            "enabled"
        } else {
            "disabled"
        }
    );
    for (name, backend) in [("vaapi", "vaapi"), ("videotoolbox", "videotoolbox")] {
        let backend = std::ffi::CString::new(backend).expect("static backend name");
        let available = unsafe {
            ffmpeg_next::ffi::av_hwdevice_find_type_by_name(backend.as_ptr())
                != ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE
        };
        println!("{name}: {}", if available { "enabled" } else { "disabled" });
    }
}

fn h264_supports_pixel_format(pixel_format: ffmpeg_next::ffi::AVPixelFormat) -> bool {
    unsafe {
        let codec =
            ffmpeg_next::ffi::avcodec_find_decoder(ffmpeg_next::ffi::AVCodecID::AV_CODEC_ID_H264);
        if codec.is_null() {
            return false;
        }
        let mut index = 0;
        loop {
            let config = ffmpeg_next::ffi::avcodec_get_hw_config(codec, index);
            if config.is_null() {
                return false;
            }
            if (*config).pix_fmt == pixel_format {
                return true;
            }
            index += 1;
        }
    }
}
