use crate::video::error::{VideoError, VideoResult};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ScheduledFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pts: Duration,
    pub index: u64,
}

impl ScheduledFrame {
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

pub struct FrameScheduler {
    start_time: Instant,
    duration: Duration,
    paused: bool,
    pause_duration: Duration,
    pause_start: Option<Instant>,
    current_frame_index: u64,
    last_frame_pts: Option<Duration>,
}

impl FrameScheduler {
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

    pub fn current_position(&self) -> Duration {
        let base = if self.paused {
            self.pause_start.unwrap_or(self.start_time)
        } else {
            Instant::now()
        };
        let elapsed = base.duration_since(self.start_time) - self.pause_duration;
        let total_ms = self.duration.as_millis().max(1);
        Duration::from_millis((elapsed.as_millis() % total_ms) as u64)
    }

    pub fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.pause_start = Some(Instant::now());
            tracing::debug!("Video paused at {:?}", self.current_position());
        }
    }

    pub fn resume(&mut self) {
        if self.paused {
            if let Some(pause_start) = self.pause_start.take() {
                self.pause_duration += pause_start.elapsed();
            }
            self.paused = false;
            tracing::debug!("Video resumed at {:?}", self.current_position());
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn seek(&mut self, position: Duration) -> VideoResult<()> {
        if position > self.duration {
            return Err(VideoError::SeekFailed(
                position,
                anyhow::anyhow!("Position exceeds video duration"),
            ));
        }
        self.start_time = Instant::now() - position;
        self.pause_duration = Duration::ZERO;
        self.last_frame_pts = None;
        tracing::info!("Seeked to {:?}", position);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.start_time = Instant::now();
        self.pause_duration = Duration::ZERO;
        self.pause_start = None;
        self.paused = false;
        self.current_frame_index = 0;
        self.last_frame_pts = None;
    }

    pub fn should_display(&self, frame_pts: Duration) -> bool {
        if self.paused {
            return false;
        }
        let current = self.current_position();
        if current < Duration::from_millis(100)
            && frame_pts > self.duration - Duration::from_millis(100)
        {
            return false;
        }
        if let Some(last_pts) = self.last_frame_pts
            && frame_pts < last_pts
            && current >= last_pts
        {
            return false;
        }
        frame_pts <= current
    }

    pub fn should_upload(&mut self, frame_pts: Duration) -> bool {
        if let Some(last_pts) = self.last_frame_pts
            && (frame_pts.as_millis() as i64 - last_pts.as_millis() as i64).abs() < 5
        {
            return false;
        }
        self.last_frame_pts = Some(frame_pts);
        true
    }

    pub fn next_frame_index(&mut self) -> u64 {
        let idx = self.current_frame_index;
        self.current_frame_index = self.current_frame_index.wrapping_add(1);
        idx
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn time_until_next_frame(&self, next_frame_pts: Duration) -> Option<Duration> {
        if self.paused {
            return None;
        }
        let current = self.current_position();
        if next_frame_pts <= current {
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
        assert!(!scheduler.should_upload(Duration::from_millis(102)));
        assert!(scheduler.should_upload(Duration::from_millis(200)));
    }

    #[test]
    fn test_waits_for_clock_before_decoder_wrap() {
        let mut scheduler = FrameScheduler::new(Duration::from_secs(1));
        scheduler.seek(Duration::from_millis(950)).unwrap();
        assert!(scheduler.should_upload(Duration::from_millis(900)));
        assert!(!scheduler.should_display(Duration::ZERO));

        scheduler.start_time = Instant::now() - Duration::from_millis(10);
        assert!(scheduler.should_display(Duration::ZERO));
    }
}
