use crate::video::{
    DecoderInfo, FrameScheduler, HwAccel, VideoDecoder, VideoError, VideoFrame, VideoMetadata,
};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct VideoPlayback {
    decoder: Mutex<Option<VideoDecoder>>,
    scheduler: Mutex<Option<FrameScheduler>>,
    current_frame: Mutex<Option<VideoFrame>>,
}

impl VideoPlayback {
    pub fn new() -> Self {
        Self {
            decoder: Mutex::new(None),
            scheduler: Mutex::new(None),
            current_frame: Mutex::new(None),
        }
    }

    pub fn start(
        &self,
        path: &Path,
        hw_accel: HwAccel,
    ) -> Result<VideoMetadata, crate::video::error::VideoError> {
        self.stop();
        let decoder = VideoDecoder::new(path, hw_accel)?;
        let metadata = decoder.metadata().clone();
        let scheduler = FrameScheduler::new(metadata.duration);
        *self.lock_decoder() = Some(decoder);
        *self.lock_scheduler() = Some(scheduler);
        tracing::info!(
            "Video started: {}x{} @ {:.2} fps, {:?}",
            metadata.width,
            metadata.height,
            metadata.fps,
            metadata.duration
        );
        Ok(metadata)
    }

    pub fn stop(&self) {
        *self.lock_decoder() = None;
        *self.lock_scheduler() = None;
        *self.lock_current() = None;
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
        {
            let mut decoder = self.lock_decoder();
            let mut scheduler = self.lock_scheduler();
            let scheduler = scheduler.as_mut().ok_or_else(|| {
                VideoError::SeekFailed(timestamp, anyhow::anyhow!("no video is playing"))
            })?;
            scheduler.seek(timestamp)?;
            if let Some(d) = decoder.as_mut() {
                d.seek(timestamp);
                d.drain();
            }
        }
        tracing::info!("Video seeked to {:?}", timestamp);
        Ok(())
    }

    pub fn next_frame(&self) -> Option<VideoFrame> {
        let mut decoder = self.lock_decoder();
        let mut scheduler = self.lock_scheduler();
        let (Some(decoder), Some(scheduler)) = (decoder.as_mut(), scheduler.as_mut()) else {
            return None;
        };
        while let Some(frame) = decoder.next_frame() {
            if scheduler.should_display(frame.pts) && scheduler.should_upload(frame.pts) {
                *self.lock_current() = Some(frame.clone());
                return Some(frame);
            }
        }
        None
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

    pub fn current_frame(&self) -> Option<VideoFrame> {
        self.lock_current().clone()
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

    fn lock_current(&self) -> std::sync::MutexGuard<'_, Option<VideoFrame>> {
        self.current_frame.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Default for VideoPlayback {
    fn default() -> Self {
        Self::new()
    }
}
