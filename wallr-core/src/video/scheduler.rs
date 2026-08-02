//! Frame scheduler with PTS-based timing for accurate video playback.
//!
//! Handles frame timing using presentation timestamps (PTS) to ensure
//! accurate playback independent of monitor refresh rate.

use crate::video::error::{VideoError, VideoResult};
use std::time::{Duration, Instant};

/// A video frame with its presentation timestamp.
#[derive(Debug, Clone)]
pub struct ScheduledFrame {
    /// Frame data (RGBA8 format).
    pub data: Vec<u8>,
    /// Width of the frame.
    pub width: u32,
    /// Height of the frame.
    pub height: u32,
    /// Presentation timestamp relative to video start.
    pub pts: Duration,
    /// Unique frame index for change detection.
    pub index: u64,
}

impl ScheduledFrame {
    /// Create a new scheduled frame.
    pub fn new(data: Vec<u8>, width: u32, height: u32, pts: Duration, index: u64) -> Self {
        Self {
            data,
            width,
            height,
            pts,
            index,
        }
    }
}

/// Frame scheduler that manages playback timing based on presentation timestamps.
pub struct FrameScheduler {
    /// Start time of playback.
    start_time: Instant,
    /// Total duration of the video.
    duration: Duration,
    /// Whether playback is paused.
    paused: bool,
    /// Accumulated pause duration.
    pause_duration: Duration,
    /// Time when pause started.
    pause_start: Option<Instant>,
    /// Current frame index for change detection.
    current_frame_index: u64,
    /// Last frame that was presented.
    last_frame_pts: Option<Duration>,
}

impl FrameScheduler {
    /// Create a new frame scheduler.
    pub fn new(duration: Duration) -> Self {
        Self {
            start_time: Instant::now(),
            duration,
            paused: false,
            pause_duration: Duration::ZERO,
            pause_start: None,
            current_frame_index: 0,
            last_frame_pts: None,
        }
    }

    /// Get the current playback position, accounting for pauses and looping.
    pub fn current_position(&self) -> Duration {
        if self.paused {
            // If paused, return the position at pause time
            if let Some(pause_start) = self.pause_start {
                let elapsed = pause_start.duration_since(self.start_time) - self.pause_duration;
                let total_ms = self.duration.as_millis().max(1);
                let elapsed_ms = elapsed.as_millis() % total_ms;
                return Duration::from_millis(elapsed_ms as u64);
            }
        }

        let elapsed = self.start_time.elapsed() - self.pause_duration;
        let total_ms = self.duration.as_millis().max(1);
        let elapsed_ms = elapsed.as_millis() % total_ms;
        Duration::from_millis(elapsed_ms as u64)
    }

    /// Pause playback.
    pub fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.pause_start = Some(Instant::now());
            tracing::debug!("Video playback paused at {:?}", self.current_position());
        }
    }

    /// Resume playback.
    pub fn resume(&mut self) {
        if self.paused {
            if let Some(pause_start) = self.pause_start.take() {
                self.pause_duration += pause_start.elapsed();
            }
            self.paused = false;
            tracing::debug!("Video playback resumed at {:?}", self.current_position());
        }
    }

    /// Check if playback is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Seek to a specific position in the video.
    pub fn seek(&mut self, position: Duration) -> VideoResult<()> {
        if position > self.duration {
            return Err(VideoError::SeekFailed(
                position,
                anyhow::anyhow!("Position exceeds video duration"),
            ));
        }

        // Reset timing to make the sought position appear as "now"
        self.start_time = Instant::now() - position;
        self.pause_duration = Duration::ZERO;
        self.last_frame_pts = None;

        tracing::info!("Seeked to {:?}", position);
        Ok(())
    }

    /// Reset the scheduler to the beginning.
    pub fn reset(&mut self) {
        self.start_time = Instant::now();
        self.pause_duration = Duration::ZERO;
        self.pause_start = None;
        self.paused = false;
        self.current_frame_index = 0;
        self.last_frame_pts = None;
    }

    /// Check if a frame should be displayed at the current time.
    ///
    /// Returns true if the frame's PTS is within the display window.
    pub fn should_display(&self, frame_pts: Duration) -> bool {
        if self.paused {
            return false;
        }

        let current = self.current_position();

        // Handle wrap-around at loop boundary
        if current < Duration::from_millis(100)
            && frame_pts > self.duration - Duration::from_millis(100)
        {
            // Current position wrapped around, old frame is outdated
            return false;
        }

        // Display if frame PTS is at or before current position
        frame_pts <= current
    }

    /// Determine if we should upload this frame to the GPU.
    ///
    /// Returns true if this is a new frame (different from last uploaded).
    pub fn should_upload(&mut self, frame_pts: Duration) -> bool {
        if let Some(last_pts) = self.last_frame_pts
            && (frame_pts.as_millis() as i64 - last_pts.as_millis() as i64).abs() < 5
        {
            // Same frame (within 5ms tolerance)
            return false;
        }

        self.last_frame_pts = Some(frame_pts);
        true
    }

    /// Get the next frame index and increment the counter.
    pub fn next_frame_index(&mut self) -> u64 {
        let idx = self.current_frame_index;
        self.current_frame_index = self.current_frame_index.wrapping_add(1);
        idx
    }

    /// Get the video duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Calculate time until the next frame should be displayed.
    ///
    /// Returns None if paused or if the next frame should be displayed immediately.
    pub fn time_until_next_frame(&self, next_frame_pts: Duration) -> Option<Duration> {
        if self.paused {
            return None;
        }

        let current = self.current_position();
        if next_frame_pts <= current {
            // Frame should be displayed now
            return Some(Duration::ZERO);
        }

        Some(next_frame_pts - current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_scheduler_position_advances() {
        let scheduler = FrameScheduler::new(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(100));
        let pos = scheduler.current_position();
        assert!(pos >= Duration::from_millis(90));
        assert!(pos <= Duration::from_millis(150));
    }

    #[test]
    fn test_scheduler_loops() {
        let mut scheduler = FrameScheduler::new(Duration::from_millis(100));
        // Simulate time passing beyond duration
        scheduler.start_time = Instant::now() - Duration::from_millis(250);
        let pos = scheduler.current_position();
        assert!(pos >= Duration::from_millis(40));
        assert!(pos <= Duration::from_millis(60));
    }

    #[test]
    fn test_pause_resume() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(50));

        scheduler.pause();
        assert!(scheduler.is_paused());
        let pos_at_pause = scheduler.current_position();

        thread::sleep(Duration::from_millis(100));
        let pos_during_pause = scheduler.current_position();

        // Position should not advance during pause
        assert!((pos_during_pause.as_millis() as i64 - pos_at_pause.as_millis() as i64).abs() < 10);

        scheduler.resume();
        assert!(!scheduler.is_paused());
    }

    #[test]
    fn test_seek() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(60));

        assert!(scheduler.seek(Duration::from_secs(30)).is_ok());
        let pos = scheduler.current_position();
        assert!(pos >= Duration::from_secs(29));
        assert!(pos <= Duration::from_secs(31));

        assert!(scheduler.seek(Duration::from_secs(100)).is_err());
    }

    #[test]
    fn test_should_upload_deduplication() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(10));

        assert!(scheduler.should_upload(Duration::from_millis(100)));
        assert!(!scheduler.should_upload(Duration::from_millis(102))); // Within tolerance
        assert!(scheduler.should_upload(Duration::from_millis(200))); // New frame
    }
}
