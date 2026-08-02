//! Video playback manager for integrating the decoder with the renderer.
//!
//! All methods are synchronous: the playback state is guarded by a plain
//! `std::sync::Mutex`, so the same manager can be driven from the blocking
//! render task (which presents every vsync) and from the async IPC handlers.
//! Locks are only ever held for short, allocation-free critical sections.

use crate::video::{
    DecoderInfo, FrameScheduler, HwAccel, VideoDecoder, VideoError, VideoFrame, VideoMetadata,
    VideoResult,
};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Video playback state manager.
pub struct VideoPlayback {
    decoder: Mutex<Option<VideoDecoder>>,
    scheduler: Mutex<Option<FrameScheduler>>,
    current_frame: Mutex<Option<VideoFrame>>,
}

impl VideoPlayback {
    /// Create a new video playback manager.
    pub fn new() -> Self {
        Self {
            decoder: Mutex::new(None),
            scheduler: Mutex::new(None),
            current_frame: Mutex::new(None),
        }
    }

    /// Start video playback from a file, replacing any active playback.
    pub fn start(&self, path: &Path, hw_accel: HwAccel) -> VideoResult<VideoMetadata> {
        // Stop the previous playback first: this drops the old decoder and
        // joins its thread, releasing decoder buffers immediately.
        self.stop();

        let decoder = VideoDecoder::new(path, hw_accel)?;
        let metadata = decoder.metadata().clone();
        let scheduler = FrameScheduler::new(metadata.duration);

        *self.lock_decoder() = Some(decoder);
        *self.lock_scheduler() = Some(scheduler);

        tracing::info!(
            "Video playback started: {}x{} @ {:.2} fps, duration: {:?}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration
        );

        Ok(metadata)
    }

    /// Stop video playback and release all resources immediately.
    pub fn stop(&self) {
        *self.lock_decoder() = None;
        *self.lock_scheduler() = None;
        *self.lock_current() = None;
    }

    /// Pause playback: the display loop keeps presenting the last frame, the
    /// scheduler freezes its clock, and the decode thread stops decoding so
    /// the CPU stays idle.
    pub fn pause(&self) {
        if let Some(scheduler) = self.lock_scheduler().as_mut() {
            scheduler.pause();
        }
        if let Some(decoder) = self.lock_decoder().as_ref() {
            decoder.pause();
        }
    }

    /// Resume playback after a pause.
    pub fn resume(&self) {
        if let Some(scheduler) = self.lock_scheduler().as_mut() {
            scheduler.resume();
        }
        if let Some(decoder) = self.lock_decoder().as_ref() {
            decoder.resume();
        }
    }

    /// Seek to a specific timestamp. Both the scheduler clock and the decoder
    /// stream are repositioned, and stale queued frames are discarded.
    pub fn seek(&self, timestamp: Duration) -> VideoResult<()> {
        {
            let mut decoder = self.lock_decoder();
            let mut scheduler = self.lock_scheduler();

            let scheduler = scheduler.as_mut().ok_or_else(|| {
                VideoError::SeekFailed(timestamp, anyhow::anyhow!("no video is playing"))
            })?;
            scheduler.seek(timestamp)?;

            if let Some(decoder) = decoder.as_mut() {
                decoder.seek(timestamp);
                decoder.drain();
            }
        }
        tracing::info!("Video playback seeked to {:?}", timestamp);
        Ok(())
    }

    /// Pull the next frame that should be presented.
    ///
    /// Returns `Some(frame)` when a new frame must be uploaded to the GPU, and
    /// `None` when the currently shown texture should be presented unchanged
    /// (duplicate frame, decoder behind, or playback paused).
    pub fn next_frame(&self) -> Option<VideoFrame> {
        let mut decoder = self.lock_decoder();
        let mut scheduler = self.lock_scheduler();

        let (Some(decoder), Some(scheduler)) = (decoder.as_mut(), scheduler.as_mut()) else {
            return None;
        };

        let mut newest: Option<VideoFrame> = None;
        while let Some(frame) = decoder.next_frame() {
            if scheduler.should_display(frame.pts) {
                if scheduler.should_upload(frame.pts) {
                    *self.lock_current() = Some(frame.clone());
                    return Some(frame);
                }
                // Duplicate of the last uploaded frame: keep the newest one
                // around in case the consumer only wants the current frame.
                newest = Some(frame);
            }
        }
        let _ = newest;
        None
    }

    /// Block until the first frame is available or the timeout elapses.
    ///
    /// Used at commit time so the transition's incoming image is the video's
    /// actual first frame instead of a black placeholder.
    pub fn wait_first_frame(&self, timeout: Duration) -> Option<VideoFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.next_frame() {
                return Some(frame);
            }
            if Instant::now() >= deadline {
                tracing::warn!("Timed out waiting for the first video frame");
                return None;
            }
            thread_sleep(5);
        }
    }

    /// Get the last presented frame without advancing playback.
    pub fn current_frame(&self) -> Option<VideoFrame> {
        self.lock_current().clone()
    }

    /// Get video metadata of the active playback.
    pub fn metadata(&self) -> Option<VideoMetadata> {
        self.lock_decoder().as_ref().map(|d| d.metadata().clone())
    }

    /// Get decoder diagnostics for the active playback.
    pub fn decoder_info(&self) -> Option<DecoderInfo> {
        self.lock_decoder().as_ref().map(|d| d.decoder_info())
    }

    /// The hardware backend actually used by the active decoder.
    pub fn hw_accel_in_use(&self) -> HwAccel {
        self.lock_decoder()
            .as_ref()
            .map(VideoDecoder::hw_accel_in_use)
            .unwrap_or(HwAccel::Software)
    }

    /// Get the current playback position.
    pub fn position(&self) -> Option<Duration> {
        self.lock_scheduler().as_ref().map(|s| s.current_position())
    }

    /// Check if playback is paused.
    pub fn is_paused(&self) -> bool {
        self.lock_scheduler()
            .as_ref()
            .map(|s| s.is_paused())
            .unwrap_or(false)
    }

    /// Check if a video is currently active.
    pub fn is_playing(&self) -> bool {
        self.lock_decoder().is_some()
    }

    fn lock_decoder(&self) -> std::sync::MutexGuard<'_, Option<VideoDecoder>> {
        self.decoder.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_scheduler(&self) -> std::sync::MutexGuard<'_, Option<FrameScheduler>> {
        self.scheduler.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_current(&self) -> std::sync::MutexGuard<'_, Option<VideoFrame>> {
        self.current_frame.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Small sleep helper (kept behind a name so the intent is obvious in
/// `wait_first_frame`).
fn thread_sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

impl Default for VideoPlayback {
    fn default() -> Self {
        Self::new()
    }
}
