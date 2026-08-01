//! Animated wallpaper (GIF) decoding and playback timing.
//!
//! The daemon decodes an animated GIF into memory once, uses its first frame
//! as the transition's incoming texture, and then plays the frames live after
//! the transition completes. Playback is wall-clock driven: `frame_index_at`
//! maps elapsed time onto the frame timeline and loops forever.

use std::path::Path;
use std::time::Duration;

/// A fully decoded animated GIF held in memory for live playback.
pub struct AnimatedImage {
    frames: Vec<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    delays: Vec<Duration>,
    total: Duration,
}

impl AnimatedImage {
    /// Decodes `path` as an animated GIF. Returns `Ok(None)` when the file is
    /// not a GIF so callers can keep their existing static-image path.
    pub fn decode(path: &Path) -> anyhow::Result<Option<Self>> {
        use image::AnimationDecoder;
        use image::ImageDecoder;

        let format = image::ImageReader::open(path)?
            .with_guessed_format()?
            .format();
        if format != Some(image::ImageFormat::Gif) {
            return Ok(None);
        }

        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(path)?,
        ))?;
        let (width, height) = decoder.dimensions();
        let mut frames = Vec::new();
        let mut delays = Vec::new();
        for frame in decoder.into_frames() {
            let frame = frame?;
            // GIF delays are centiseconds; browsers treat a 0 delay as 100ms.
            let (numer, denom) = frame.delay().numer_denom_ms();
            let millis = numer.checked_div(denom).unwrap_or(100);
            let delay = Duration::from_millis(millis as u64)
                .clamp(Duration::from_millis(20), Duration::from_secs(5));
            frames.push(frame.buffer().as_raw().clone());
            delays.push(delay);
        }
        if frames.is_empty() {
            return Ok(None);
        }
        let total = delays.iter().copied().sum();
        Ok(Some(Self {
            frames,
            width,
            height,
            delays,
            total,
        }))
    }

    /// RGBA8 bytes of the first frame, used as the transition's incoming image.
    pub fn first_frame(&self) -> &[u8] {
        &self.frames[0]
    }

    pub fn frame_at(&self, index: usize) -> &[u8] {
        &self.frames[index.min(self.frames.len() - 1)]
    }

    /// Index of the frame to display at `elapsed` time, looping forever.
    pub fn frame_index_at(&self, elapsed: Duration) -> usize {
        let total_ms = self.total.as_millis().max(1);
        let mut t = elapsed.as_millis() % total_ms;
        for (i, delay) in self.delays.iter().enumerate() {
            let ms = delay.as_millis();
            if t < ms {
                return i;
            }
            t -= ms;
        }
        self.frames.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::AnimatedImage;
    use std::time::Duration;

    fn sample() -> AnimatedImage {
        AnimatedImage {
            frames: vec![vec![0; 4], vec![1; 4], vec![2; 4]],
            width: 1,
            height: 1,
            delays: vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(300),
            ],
            total: Duration::from_millis(600),
        }
    }

    #[test]
    fn frame_index_tracks_delays() {
        let anim = sample();
        assert_eq!(anim.frame_index_at(Duration::ZERO), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(99)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(100)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(299)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(300)), 2);
        assert_eq!(anim.frame_index_at(Duration::from_millis(599)), 2);
    }

    #[test]
    fn frame_index_loops() {
        let anim = sample();
        // 600ms is one full cycle; the playhead wraps back to frame 0.
        assert_eq!(anim.frame_index_at(Duration::from_millis(600)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(610)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(700)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(3000)), 0);
    }

    #[test]
    fn frame_at_clamps_out_of_range() {
        let anim = sample();
        assert_eq!(anim.frame_at(0), &[0, 0, 0, 0]);
        assert_eq!(anim.frame_at(999), &[2, 2, 2, 2]);
        assert_eq!(anim.first_frame(), &[0, 0, 0, 0]);
    }
}
