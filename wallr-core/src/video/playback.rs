use crate::video::{
    DecoderInfo, FrameScheduler, HwAccel, VideoDecoder, VideoError, VideoFrame, VideoMetadata,
};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct VideoPlayback {
    decoder: Mutex<Option<VideoDecoder>>,
    scheduler: Mutex<Option<FrameScheduler>>,
    pending_frame: Mutex<Option<VideoFrame>>,
    generation: AtomicU64,
}

pub struct PreparedVideoPlayback {
    decoder: VideoDecoder,
    metadata: VideoMetadata,
    first_frame: Option<VideoFrame>,
}

impl PreparedVideoPlayback {
    pub fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    pub fn take_first_frame(&mut self) -> Option<VideoFrame> {
        self.first_frame.take()
    }
}

impl VideoPlayback {
    pub fn new() -> Self {
        Self {
            decoder: Mutex::new(None),
            scheduler: Mutex::new(None),
            pending_frame: Mutex::new(None),
            generation: AtomicU64::new(u64::MAX),
        }
    }

    pub fn start(
        &self,
        path: &Path,
        hw_accel: HwAccel,
        preload_frames: usize,
        generation: u64,
    ) -> Result<VideoMetadata, crate::video::error::VideoError> {
        let prepared = Self::prepare(
            path,
            hw_accel,
            preload_frames,
            Duration::from_millis(0),
            |_| Ok(()),
        )?;
        Ok(self.commit(prepared, generation))
    }

    pub fn prepare<F>(
        path: &Path,
        hw_accel: HwAccel,
        preload_frames: usize,
        first_frame_timeout: Duration,
        validate: F,
    ) -> Result<PreparedVideoPlayback, crate::video::error::VideoError>
    where
        F: FnOnce(&VideoMetadata) -> Result<(), crate::video::error::VideoError>,
    {
        let decoder = VideoDecoder::with_preload(path, hw_accel, preload_frames)?;
        let metadata = decoder.metadata().clone();
        validate(&metadata)?;
        let deadline = Instant::now() + first_frame_timeout;
        let first_frame = loop {
            if let Some(frame) = decoder.next_frame() {
                break Some(frame);
            }
            if Instant::now() >= deadline {
                tracing::warn!("Timed out waiting for first video frame");
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };

        Ok(PreparedVideoPlayback {
            decoder,
            metadata,
            first_frame,
        })
    }

    pub fn commit(&self, prepared: PreparedVideoPlayback, generation: u64) -> VideoMetadata {
        let PreparedVideoPlayback {
            decoder,
            metadata,
            first_frame,
        } = prepared;
        let scheduler = FrameScheduler::new(metadata.duration);
        self.stop();
        self.generation.store(generation, Ordering::Release);
        *self.lock_decoder() = Some(decoder);
        *self.lock_scheduler() = Some(scheduler);
        *self.lock_pending() = first_frame;
        tracing::info!(
            "Video started: {}x{} @ {:.2} fps, {:?}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration
        );
        metadata
    }

    pub fn stop(&self) {
        self.generation.store(u64::MAX, Ordering::Release);
        *self.lock_decoder() = None;
        *self.lock_scheduler() = None;
        *self.lock_pending() = None;
    }

    /// Stops playback only when `generation` is still active.
    /// Superseded callers cannot stop their successor's playback.
    pub fn stop_generation(&self, generation: u64) {
        let mut decoder = self.lock_decoder();
        let mut scheduler = self.lock_scheduler();
        let mut pending = self.lock_pending();
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        self.generation.store(u64::MAX, Ordering::Release);
        *decoder = None;
        *scheduler = None;
        *pending = None;
    }

    pub fn pause(&self) {
        if let Some(s) = self.lock_scheduler().as_mut() {
            s.pause();
        }
        if let Some(d) = self.lock_decoder().as_ref() {
            d.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(s) = self.lock_scheduler().as_mut() {
            s.resume();
        }
        if let Some(d) = self.lock_decoder().as_ref() {
            d.resume();
        }
    }

    pub fn seek(&self, timestamp: Duration) -> Result<(), crate::video::error::VideoError> {
        let mut decoder = self.lock_decoder();
        let mut scheduler = self.lock_scheduler();
        let mut pending = self.lock_pending();
        let scheduler = scheduler.as_mut().ok_or_else(|| {
            VideoError::SeekFailed(timestamp, anyhow::anyhow!("no video is playing"))
        })?;
        scheduler.seek(timestamp)?;
        if let Some(d) = decoder.as_mut() {
            d.seek(timestamp);
            d.drain();
        }
        *pending = None;
        tracing::info!("Video seeked to {:?}", timestamp);
        Ok(())
    }

    pub fn next_frame(&self) -> Option<VideoFrame> {
        self.next_frame_for_generation(None)
    }

    /// Returns a frame only when `generation` is still active.
    pub fn next_frame_in_generation(&self, generation: u64) -> Option<VideoFrame> {
        self.next_frame_for_generation(Some(generation))
    }

    fn next_frame_for_generation(&self, generation: Option<u64>) -> Option<VideoFrame> {
        let mut decoder = self.lock_decoder();
        if generation.is_some_and(|expected| self.generation.load(Ordering::Acquire) != expected) {
            return None;
        }
        let mut scheduler = self.lock_scheduler();
        let mut pending = self.lock_pending();
        let (Some(decoder), Some(scheduler)) = (decoder.as_mut(), scheduler.as_mut()) else {
            return None;
        };

        take_due_frame(scheduler, &mut pending, || decoder.next_frame())
    }

    pub fn wait_first_frame(&self, timeout: Duration) -> Option<VideoFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.next_frame() {
                return Some(frame);
            }
            if Instant::now() >= deadline {
                tracing::warn!("Timed out waiting for first video frame");
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn time_until_next_frame(&self) -> Option<Duration> {
        self.time_until_next_frame_for_generation(None)
    }

    /// Returns a deadline only when `generation` is still active.
    pub fn time_until_next_frame_in_generation(&self, generation: u64) -> Option<Duration> {
        self.time_until_next_frame_for_generation(Some(generation))
    }

    fn time_until_next_frame_for_generation(&self, generation: Option<u64>) -> Option<Duration> {
        let scheduler = self.lock_scheduler();
        let pending = self.lock_pending();
        if generation.is_some_and(|expected| self.generation.load(Ordering::Acquire) != expected) {
            return None;
        }
        let (Some(scheduler), Some(frame)) = (scheduler.as_ref(), pending.as_ref()) else {
            return None;
        };
        scheduler.time_until_next_frame(frame.pts)
    }

    pub fn metadata(&self) -> Option<VideoMetadata> {
        self.lock_decoder().as_ref().map(|d| d.metadata().clone())
    }

    pub fn decoder_info(&self) -> Option<DecoderInfo> {
        self.lock_decoder().as_ref().map(|d| d.decoder_info())
    }

    pub fn hw_accel_in_use(&self) -> HwAccel {
        self.lock_decoder()
            .as_ref()
            .map(VideoDecoder::hw_accel_in_use)
            .unwrap_or(HwAccel::Software)
    }

    pub fn position(&self) -> Option<Duration> {
        self.lock_scheduler().as_ref().map(|s| s.current_position())
    }

    pub fn is_paused(&self) -> bool {
        self.lock_scheduler()
            .as_ref()
            .map(|s| s.is_paused())
            .unwrap_or(false)
    }

    pub fn is_playing(&self) -> bool {
        self.lock_decoder().is_some()
    }

    fn lock_decoder(&self) -> std::sync::MutexGuard<'_, Option<VideoDecoder>> {
        self.decoder.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_scheduler(&self) -> std::sync::MutexGuard<'_, Option<FrameScheduler>> {
        self.scheduler.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_pending(&self) -> std::sync::MutexGuard<'_, Option<VideoFrame>> {
        self.pending_frame.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn take_due_frame(
    scheduler: &mut FrameScheduler,
    pending: &mut Option<VideoFrame>,
    mut next_frame: impl FnMut() -> Option<VideoFrame>,
) -> Option<VideoFrame> {
    let mut selected = None;

    if let Some(frame) = pending.take() {
        if !scheduler.should_display(frame.pts) {
            *pending = Some(frame);
            return None;
        }
        if scheduler.should_upload(frame.pts) {
            selected = Some(frame);
        }
    }

    while let Some(frame) = next_frame() {
        if !scheduler.should_display(frame.pts) {
            *pending = Some(frame);
            break;
        }
        if scheduler.should_upload(frame.pts) {
            selected = Some(frame);
        }
    }

    selected
}

impl Default for VideoPlayback {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn frame(pts_ms: u64) -> VideoFrame {
        VideoFrame {
            data: crate::video::VideoFrameData::Rgba(vec![pts_ms as u8]),
            width: 1,
            height: 1,
            pts: Duration::from_millis(pts_ms),
            index: pts_ms,
        }
    }

    #[test]
    fn retains_future_frame_until_due() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(1));
        let mut pending = None;
        let mut frames = VecDeque::from([frame(0), frame(100)]);

        let first = take_due_frame(&mut scheduler, &mut pending, || frames.pop_front()).unwrap();
        assert_eq!(first.pts, Duration::ZERO);
        assert_eq!(pending.as_ref().unwrap().pts, Duration::from_millis(100));

        scheduler.seek(Duration::from_millis(110)).unwrap();
        let second = take_due_frame(&mut scheduler, &mut pending, || None).unwrap();
        assert_eq!(second.pts, Duration::from_millis(100));
        assert!(pending.is_none());
    }

    #[test]
    fn selects_latest_due_frame() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(1));
        scheduler.seek(Duration::from_millis(50)).unwrap();
        let mut pending = None;
        let mut frames = VecDeque::from([frame(0), frame(16), frame(32), frame(100)]);

        let selected = take_due_frame(&mut scheduler, &mut pending, || frames.pop_front()).unwrap();
        assert_eq!(selected.pts, Duration::from_millis(32));
        assert_eq!(pending.as_ref().unwrap().pts, Duration::from_millis(100));
    }
}
