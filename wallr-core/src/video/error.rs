//! Error types for video wallpaper operations.

use std::path::PathBuf;

pub type VideoResult<T> = Result<T, VideoError>;

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("failed to open video file: {path}")]
    FileOpen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unsupported video format: {0}")]
    UnsupportedFormat(String),

    #[error("no video stream found in {0}")]
    NoVideoStream(PathBuf),

    #[error("video codec not supported: {0}")]
    UnsupportedCodec(String),

    #[error("hardware decoder initialization failed: {backend}")]
    HardwareDecoderInit {
        backend: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("all hardware decoders failed, falling back to software")]
    HardwareDecoderFallback,

    #[error("software decoder initialization failed")]
    SoftwareDecoderInit(#[source] anyhow::Error),

    #[error("failed to decode video frame")]
    DecodeFailed(#[source] anyhow::Error),

    #[error("failed to convert frame format")]
    FormatConversionFailed(#[source] anyhow::Error),

    #[error("failed to upload frame to GPU texture")]
    TextureUploadFailed(#[source] anyhow::Error),

    #[error("invalid presentation timestamp")]
    InvalidPts,

    #[error("video duration could not be determined")]
    UnknownDuration,

    #[error("failed to seek to timestamp {0:?}")]
    SeekFailed(std::time::Duration, #[source] anyhow::Error),

    #[error("GPU adapter not found: {0}")]
    AdapterNotFound(String),

    #[error("failed to create GPU resources")]
    GpuResourceCreation(#[source] anyhow::Error),

    #[error("video playback channel disconnected")]
    ChannelDisconnected,

    #[error("video frame queue is full")]
    QueueFull,

    #[error("end of stream reached")]
    EndOfStream,
}

impl VideoError {
    /// Returns true if this error is recoverable and playback can continue.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            VideoError::DecodeFailed(_) | VideoError::QueueFull | VideoError::InvalidPts
        )
    }

    /// Returns true if this error indicates hardware acceleration is unavailable.
    pub fn is_hardware_unavailable(&self) -> bool {
        matches!(
            self,
            VideoError::HardwareDecoderInit { .. } | VideoError::HardwareDecoderFallback
        )
    }
}
