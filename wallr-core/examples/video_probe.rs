//! Standalone video decode probe.
//!
//! Usage: `cargo run --release --example video_probe -- <video> [vaapi|nvdec|software]`
//!
//! Decodes the file for a few seconds and reports frame rate, timing, and the
//! backend actually used — no Wayland or GPU required.

use std::time::{Duration, Instant};
use wallr_core::video::{HwAccel, VideoDecoder};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: video_probe <video> [backend]");
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
            std::process::exit(1);
        }
    };

    let meta = decoder.metadata().clone();
    println!(
        "file: {path}\nresolution: {}x{}\nfps: {:.2}\nduration: {:?}\ncodec: {}\ncontainer: {}",
        meta.width, meta.height, meta.fps, meta.duration, meta.codec, meta.format
    );
    println!(
        "requested backend: {} (active: {})",
        backend.name(),
        decoder.hw_accel_in_use().name()
    );

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
    println!("result: {}", if count > 30 { "PASS" } else { "FAIL" });
}
