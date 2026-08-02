//! # Video Wallpaper Support
//!
//! Native video wallpaper playback with hardware-accelerated decoding.
//!
//! ## Architecture
//!
//! - **decoder**: FFmpeg-based video demuxing and decoding with hardware acceleration
//! - **scheduler**: PTS-based frame timing and presentation scheduling
//! - **gpu**: GPU adapter detection and selection
//! - **error**: Comprehensive error types for video operations
//!
//! ## Design Principles
//!
//! - Hardware-accelerated decoding (VAAPI, NVDEC) with software fallback
//! - Streaming architecture: bounded queues, no full video buffering
//! - Zero-copy or minimal-copy texture uploads
//! - PTS-based timing for accurate playback
//! - Seamless looping without restart
//! - Integration with existing transition system

pub mod decoder;
pub mod error;
pub mod gpu;
pub mod playback;
pub mod scheduler;

pub use decoder::{DecoderInfo, HwAccel, VideoDecoder, VideoFrame, VideoMetadata, VideoSource};
pub use error::{VideoError, VideoResult};
pub use gpu::{GpuSelection, detect_adapters, select_adapter};
pub use playback::VideoPlayback;
pub use scheduler::{FrameScheduler, ScheduledFrame};
