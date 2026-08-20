//! Animated wallpaper (GIF) streaming and playback timing.
//!
//! The daemon parses the GIF header blocks once to learn frame delays and
//! total duration (no pixel decode), then decodes frames on demand with the
//! fast `gif` crate. Decoded frames are stored in memory — raw when they fit
//! the budget, zstd-compressed otherwise — so looping playback skips the
//! re-decode entirely: each loop is a memcpy (raw) or a decompress (zstd).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gif::DisposalMethod;

/// Maximum bytes of decoded frame data kept in RAM. Frames beyond this are
/// still decoded on demand, but not cached across loop wraps.
const CACHE_BUDGET: usize = 256 * 1024 * 1024;
const MAX_GIF_WORKING_SET: usize = 512 * 1024 * 1024;

/// A cached frame: raw RGBA8 or zstd-compressed RGBA8. The whole animation
/// uses one representation, chosen at decode time: raw when the full decoded
/// size fits [`CACHE_BUDGET`] (small wallpapers → memcpy-speed loops),
/// zstd otherwise (big wallpapers compress ~30:1 and still fit).
#[derive(Clone)]
enum CachedFrame {
    Raw(Vec<u8>),
    Zstd(Vec<u8>),
}

struct GifReader {
    decoder: gif::Decoder<std::io::BufReader<std::fs::File>>,
}

struct DecodedFrame {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    dispose: DisposalMethod,
    rgba: Vec<u8>,
}

impl GifReader {
    fn open(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let decoder = options.read_info(std::io::BufReader::new(file))?;
        Ok(Self { decoder })
    }

    /// Decodes the next frame into RGBA8. Returns `None` on end of stream or
    /// a truncated/corrupt tail (the caller restarts the stream).
    fn next_rgba(&mut self) -> Option<DecodedFrame> {
        match self.decoder.read_next_frame() {
            Ok(Some(frame)) => Some(DecodedFrame {
                left: frame.left,
                top: frame.top,
                width: frame.width,
                height: frame.height,
                dispose: frame.dispose,
                rgba: frame.buffer.as_ref().to_vec(),
            }),
            Ok(None) | Err(_) => None,
        }
    }
}

struct GifInfo {
    width: u32,
    height: u32,
    delays: Vec<Duration>,
    /// True when every frame is full-canvas with no transparency, so the
    /// decoder can skip canvas compositing entirely.
    opaque: bool,
}

pub struct AnimatedImage {
    path: PathBuf,
    pub width: u32,
    pub height: u32,
    delays: Vec<Duration>,
    total: Duration,
    /// Per-frame cached data, `None` when not (yet) cached.
    cache: Vec<Option<CachedFrame>>,
    cache_bytes: usize,
    reader: Option<GifReader>,
    next_index: usize,
    /// Canvas for transparency compositing; also the scratch target for
    /// freshly decoded opaque frames.
    canvas: Vec<u8>,
    scratch: Vec<u8>,
    /// Persistent zstd context, reused across frames (creating one per call
    /// is measurably slower).
    decompressor: zstd::bulk::Decompressor<'static>,
    prev_save: Vec<u8>,
    opaque: bool,
    raw_cache: bool,
}

impl AnimatedImage {
    /// Parses `path`'s GIF header for timing metadata without decoding any
    /// pixels. Returns `Ok(None)` when the file is not a GIF so callers can
    /// keep their existing static-image path.
    pub fn decode(path: &Path) -> anyhow::Result<Option<Self>> {
        let bytes = std::fs::read(path)?;
        let Some(info) = scan_gif(&bytes)? else {
            return Ok(None);
        };
        if info.delays.is_empty() {
            return Ok(None);
        }
        let total = info.delays.iter().copied().sum();
        let (pixels, raw_size) = gif_allocation_sizes(info.width, info.height, info.delays.len())?;
        let frame_count = info.delays.len();
        let raw_cache = raw_size <= CACHE_BUDGET;
        if !raw_cache {
            tracing::debug!(
                "GIF too large for raw cache ({:.1}MB > {}MB), using zstd",
                raw_size as f64 / 1e6,
                CACHE_BUDGET / 1024 / 1024
            );
        }
        Ok(Some(Self {
            path: path.to_path_buf(),
            width: info.width,
            height: info.height,
            delays: info.delays,
            total,
            cache: vec![None; frame_count],
            cache_bytes: 0,
            reader: Some(GifReader::open(path)?),
            next_index: 0,
            canvas: vec![0; pixels * 4],
            scratch: vec![0; pixels * 4],
            decompressor: zstd::bulk::Decompressor::new()?,
            prev_save: Vec::new(),
            opaque: info.opaque,
            raw_cache,
        }))
    }

    fn restart(&mut self) {
        self.reader = GifReader::open(&self.path).ok();
        self.next_index = 0;
    }

    /// Decodes the next frame, compositing it onto the canvas and caching it.
    /// Returns `false` at end of stream / on error.
    fn decode_next(&mut self) -> bool {
        let index = self.next_index;
        let Some(reader) = self.reader.as_mut() else {
            return false;
        };
        let Some(f) = reader.next_rgba() else {
            return false;
        };
        let (w, _h) = (self.width as usize, self.height as usize);

        if self.opaque {
            self.canvas.copy_from_slice(&f.rgba);
        } else {
            // Transparency compositing: the frame's rect is blended onto the
            // persistent canvas, then the disposal method is applied.
            let fw = f.width as usize;
            let fh = f.height as usize;
            let left = f.left as usize;
            let top = f.top as usize;

            if f.dispose == DisposalMethod::Previous {
                let need = fw * fh * 4;
                if self.prev_save.len() != need {
                    self.prev_save = vec![0; need];
                }
                for y in 0..fh {
                    let src = (y * w + left) * 4;
                    self.prev_save[y * fw * 4..(y + 1) * fw * 4]
                        .copy_from_slice(&self.canvas[src..src + fw * 4]);
                }
            }

            for y in 0..fh {
                let src = &f.rgba[y * fw * 4..(y + 1) * fw * 4];
                let dst = ((top + y) * w + left) * 4;
                for px in 0..fw {
                    let si = px * 4;
                    if src[si + 3] != 0 {
                        let di = dst + si;
                        self.canvas[di..di + 4].copy_from_slice(&src[si..si + 4]);
                    }
                }
            }

            match f.dispose {
                DisposalMethod::Background => {
                    for y in 0..fh {
                        let dst = ((top + y) * w + left) * 4;
                        self.canvas[dst..dst + fw * 4].fill(0);
                    }
                }
                DisposalMethod::Previous => {
                    for y in 0..fh {
                        let dst = ((top + y) * w + left) * 4;
                        self.canvas[dst..dst + fw * 4]
                            .copy_from_slice(&self.prev_save[y * fw * 4..(y + 1) * fw * 4]);
                    }
                }
                _ => {}
            }
        }

        // Cache the freshly decoded frame if the budget allows: raw when the
        // whole animation fits, zstd otherwise.
        let raw_len = self.canvas.len();
        if index < self.cache.len() && self.cache[index].is_none() {
            if self.raw_cache {
                self.cache_bytes += raw_len;
                self.cache[index] = Some(CachedFrame::Raw(self.canvas.clone()));
            } else {
                let compressed =
                    zstd::bulk::compress(&self.canvas, 1).unwrap_or_else(|_| self.canvas.clone());
                if self.cache_bytes + compressed.len() <= CACHE_BUDGET {
                    self.cache_bytes += compressed.len();
                    self.cache[index] = Some(CachedFrame::Zstd(compressed));
                }
            }
        }

        self.next_index += 1;
        true
    }

    /// Ensure `index` is decoded, advancing the streaming decoder as needed.
    /// Frames already in the cache are skipped without re-decoding.
    /// Restarts from frame 0 on loop wrap or end of stream.
    fn ensure_upto(&mut self, index: usize) {
        if index < self.next_index {
            self.restart();
        }
        let mut guard = 0;
        while self.next_index <= index {
            if self.next_index < self.cache.len() && self.cache[self.next_index].is_some() {
                self.next_index += 1;
                continue;
            }
            let before = self.next_index;
            if !self.decode_next() {
                // End of stream while seeking forward: restart from 0.
                self.restart();
            }
            guard += 1;
            // Safety valve against truncated streams that never progress.
            if self.next_index == before || guard > self.delays.len() + 1 {
                break;
            }
        }
    }

    /// RGBA8 bytes of the first frame, used as the transition's incoming
    /// image.
    pub fn first_frame(&mut self) -> &[u8] {
        self.frame_at(0)
    }

    pub fn frame_at(&mut self, index: usize) -> &[u8] {
        let index = index.min(self.delays.len().saturating_sub(1));
        self.ensure_upto(index);
        match self.cache.get(index) {
            Some(Some(CachedFrame::Raw(raw))) => return raw,
            Some(Some(CachedFrame::Zstd(compressed))) => {
                let size = self.canvas.len();
                let used = self
                    .decompressor
                    .decompress_to_buffer(compressed, &mut self.scratch[..size])
                    .unwrap_or(0);
                if used == size {
                    return &self.scratch[..used];
                }
                return &self.canvas;
            }
            _ => {}
        }
        &self.canvas
    }

    /// Decompresses the frame at `index` directly into `out` (which must be
    /// exactly `width * height * 4` bytes), skipping the shared scratch
    /// buffer. Returns `false` when the frame is not cached yet.
    pub fn decompress_into(&mut self, index: usize, out: &mut [u8]) -> bool {
        let index = index.min(self.delays.len().saturating_sub(1));
        self.ensure_upto(index);
        match self.cache.get(index) {
            Some(Some(CachedFrame::Raw(raw))) => {
                out.copy_from_slice(raw);
                true
            }
            Some(Some(CachedFrame::Zstd(compressed))) => self
                .decompressor
                .decompress_to_buffer(compressed, out)
                .map(|used| used == out.len())
                .unwrap_or(false),
            _ => false,
        }
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
        self.delays.len() - 1
    }

    /// Cumulative time at which frame `index` begins. For `index == len` this
    /// equals the total duration (the next wrap back to frame 0).
    pub fn frame_start(&self, index: usize) -> Duration {
        self.delays.iter().take(index).copied().sum()
    }

    pub fn frame_count(&self) -> usize {
        self.delays.len()
    }

    pub fn cache_bytes(&self) -> usize {
        self.cache_bytes
    }

    pub fn raw_cached(&self) -> usize {
        self.cache
            .iter()
            .filter(|c| matches!(c, Some(CachedFrame::Raw(_))))
            .count()
    }

    pub fn zstd_cached(&self) -> usize {
        self.cache
            .iter()
            .filter(|c| matches!(c, Some(CachedFrame::Zstd(_))))
            .count()
    }

    pub fn total_duration(&self) -> Duration {
        self.total
    }
}

fn gif_allocation_sizes(
    width: u32,
    height: u32,
    frame_count: usize,
) -> anyhow::Result<(usize, usize)> {
    let pixels = usize::try_from(u64::from(width) * u64::from(height))?;
    let frame_bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("GIF frame size overflow for {width}x{height}"))?;
    let working_set = frame_bytes
        .checked_mul(3)
        .ok_or_else(|| anyhow::anyhow!("GIF working-set overflow for {width}x{height}"))?;
    anyhow::ensure!(
        working_set <= MAX_GIF_WORKING_SET,
        "GIF {width}x{height} requires approximately {:.1} MiB of decode working memory, exceeding the {} MiB safety limit",
        working_set as f64 / (1024.0 * 1024.0),
        MAX_GIF_WORKING_SET / 1024 / 1024
    );
    Ok((pixels, frame_bytes.saturating_mul(frame_count)))
}

/// Parses GIF header blocks (screen descriptor, graphic control extensions,
/// image descriptors) without decoding pixel data, returning the timeline.
fn scan_gif(bytes: &[u8]) -> anyhow::Result<Option<GifInfo>> {
    if bytes.len() < 13 || &bytes[0..3] != b"GIF" {
        return Ok(None);
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    let packed = bytes[10];
    let gct_size = 3 * (1 << ((packed & 0x07) + 1));
    let mut pos = 13 + gct_size;
    if pos > bytes.len() {
        return Ok(None);
    }

    let mut delays = Vec::new();
    let mut opaque = true;
    let mut frame_count = 0usize;

    while pos < bytes.len() {
        match bytes[pos] {
            0x3B => break, // trailer
            0x2C => {
                // Image descriptor: left, top, w, h (u16 each), packed byte.
                if pos + 10 > bytes.len() {
                    break;
                }
                let left = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]) as u32;
                let top = u16::from_le_bytes([bytes[pos + 3], bytes[pos + 4]]) as u32;
                let iw = u16::from_le_bytes([bytes[pos + 5], bytes[pos + 6]]) as u32;
                let ih = u16::from_le_bytes([bytes[pos + 7], bytes[pos + 8]]) as u32;
                let ipacked = bytes[pos + 9];
                pos += 10;
                if ipacked & 0x80 != 0 {
                    pos += 3 * (1 << ((ipacked & 0x07) + 1));
                }
                if pos >= bytes.len() {
                    break;
                }
                pos += 1; // LZW minimum code size
                pos = skip_sub_blocks(bytes, pos);
                if left != 0 || top != 0 || iw != width || ih != height {
                    opaque = false;
                }
                frame_count += 1;
            }
            0x21 => {
                // Extension: 0xF9 = graphic control extension (delay).
                if pos + 2 <= bytes.len() && bytes[pos + 1] == 0xF9 {
                    if pos + 8 > bytes.len() {
                        break;
                    }
                    let gce_packed = bytes[pos + 3];
                    if gce_packed & 0x01 != 0 {
                        opaque = false; // transparency flag
                    }
                    // Delay in centiseconds; browsers treat 0 as 100ms, but
                    // keep the prior 20ms clamp for zero delays.
                    let centis = u16::from_le_bytes([bytes[pos + 4], bytes[pos + 5]]);
                    let millis = if centis == 0 { 20 } else { centis as u64 * 10 };
                    delays.push(
                        Duration::from_millis(millis)
                            .clamp(Duration::from_millis(20), Duration::from_secs(5)),
                    );
                    pos += 8;
                    continue;
                }
                pos += 2;
                pos = skip_sub_blocks(bytes, pos);
            }
            _ => break, // corrupt block: stop scanning
        }
    }

    if frame_count == 0 {
        return Ok(None);
    }
    // If no GCE appeared at all, every frame gets the default delay.
    if delays.is_empty() {
        delays = vec![Duration::from_millis(20); frame_count];
    }
    Ok(Some(GifInfo {
        width,
        height,
        delays,
        opaque,
    }))
}

/// Skips a run of length-prefixed data sub-blocks; returns the position after
/// the terminating zero-length block.
fn skip_sub_blocks(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        if pos >= bytes.len() {
            return bytes.len();
        }
        let n = bytes[pos] as usize;
        pos += 1;
        if n == 0 {
            return pos;
        }
        pos += n;
        if pos > bytes.len() {
            return bytes.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn delays() -> Vec<Duration> {
        vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ]
    }

    fn anim_with(delays: Vec<Duration>) -> AnimatedImage {
        AnimatedImage {
            path: PathBuf::new(),
            width: 1,
            height: 1,
            total: delays.iter().copied().sum(),
            cache: vec![None; delays.len()],
            cache_bytes: 0,
            reader: None,
            next_index: 0,
            canvas: vec![0; 4],
            scratch: vec![0; 4],
            decompressor: zstd::bulk::Decompressor::new().unwrap(),
            prev_save: Vec::new(),
            opaque: true,
            raw_cache: true,
            delays,
        }
    }

    #[test]
    fn frame_index_tracks_delays() {
        let anim = anim_with(delays());
        assert_eq!(anim.frame_index_at(Duration::ZERO), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(99)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(100)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(299)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(300)), 2);
        assert_eq!(anim.frame_index_at(Duration::from_millis(599)), 2);
    }

    #[test]
    fn frame_index_loops() {
        let anim = anim_with(delays());
        // 600ms is one full cycle; the playhead wraps back to frame 0.
        assert_eq!(anim.frame_index_at(Duration::from_millis(600)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(610)), 0);
        assert_eq!(anim.frame_index_at(Duration::from_millis(700)), 1);
        assert_eq!(anim.frame_index_at(Duration::from_millis(3000)), 0);
    }

    #[test]
    fn scan_rejects_non_gif() {
        assert!(scan_gif(b"not a gif at all").unwrap().is_none());
        assert!(scan_gif(&[0u8; 100]).unwrap().is_none());
    }

    #[test]
    fn rejects_gif_working_sets_before_allocating_canvases() {
        assert!(gif_allocation_sizes(7_680, 4_320, 2).is_ok());
        assert!(gif_allocation_sizes(16_384, 16_384, 1).is_err());
    }
}
