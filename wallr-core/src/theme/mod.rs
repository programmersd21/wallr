use crate::config::{MatugenConfig, ThemeProvider};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("failed to spawn theme provider: {0}")]
    SpawnError(#[from] std::io::Error),
    #[error("theme provider exited with code {0}: {1}")]
    NonZeroExit(i32, String),
}

pub fn dispatch_theme(
    provider: &ThemeProvider,
    image_path: &Path,
    matugen_config: &MatugenConfig,
) -> Result<(), ThemeError> {
    let effective_path = resolve_theme_image(image_path);
    let effective_ref: &Path = effective_path.as_deref().unwrap_or(image_path);
    match provider {
        ThemeProvider::Matugen if !matugen_config.enabled => Ok(()),
        ThemeProvider::Matugen => run_matugen(effective_ref, matugen_config),
        ThemeProvider::Wallust => run_wallust(effective_ref),
        ThemeProvider::Pywal => run_pywal(effective_ref),
        ThemeProvider::None => Ok(()),
    }
}

/// Returns a static image suitable for theme providers.
///
/// Video files (`mp4`, `webm`, `mkv`, `mov`, `avi`, `m4v`) and GIFs cannot be
/// consumed directly by `matugen`/`wallust`/`pywal`. We extract the first
/// frame to a cached PNG under `~/.cache/wallr/theme/` (hashed on source path
/// + mtime) and return that path. On any error we log and fall back to the
/// original path so wallpaper setting still succeeds.
#[allow(clippy::doc_markdown, clippy::doc_lazy_continuation)]
fn resolve_theme_image(image_path: &Path) -> Option<PathBuf> {
    let ext = image_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let is_video = matches!(ext.as_str(), "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v");
    let is_gif = ext == "gif";

    if !is_video && !is_gif {
        return None;
    }

    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("wallr")
        .join("theme");

    if let Err(e) = fs::create_dir_all(&cache_dir) {
        tracing::warn!("theme frame cache dir create failed: {e}");
        return None;
    }

    let key = match theme_cache_key(image_path) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("theme cache key failed for {}: {e}", image_path.display());
            return None;
        }
    };

    let dest = cache_dir.join(format!("{key}.png"));

    // Reuse cached frame if it is newer than the source.
    if dest.exists() {
        if let (Ok(dest_meta), Ok(src_meta)) = (fs::metadata(&dest), fs::metadata(image_path)) {
            if let (Ok(dest_mtime), Ok(src_mtime)) = (dest_meta.modified(), src_meta.modified()) {
                if dest_mtime >= src_mtime {
                    tracing::debug!("reusing cached theme frame {}", dest.display());
                    return Some(dest);
                }
            } else {
                return Some(dest);
            }
        } else {
            return Some(dest);
        }
    }

    let extract_res = if is_video {
        extract_video_first_frame(image_path, &dest)
    } else {
        extract_gif_first_frame(image_path, &dest)
    };

    match extract_res {
        Ok(()) => {
            tracing::info!(
                "extracted theme frame {} -> {}",
                image_path.display(),
                dest.display()
            );
            Some(dest)
        }
        Err(e) => {
            tracing::warn!(
                "failed to extract first frame from {}: {e}; falling back to original",
                image_path.display()
            );
            None
        }
    }
}

fn theme_cache_key(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(mtime.to_string().as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn extract_gif_first_frame(path: &Path, dest: &Path) -> Result<(), String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Write to a temp file then atomically rename to avoid partial writes
    let tmp = dest.with_extension("tmp.png");
    img.save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_video_first_frame(path: &Path, dest: &Path) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    ffmpeg::init().map_err(|e| format!("ffmpeg init failed: {e}"))?;

    let mut ictx = ffmpeg::format::input(path).map_err(|e| format!("open input: {e}"))?;

    let stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| "no video stream".to_string())?;
    let video_idx = stream.index();
    let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|e| format!("codec context: {e}"))?;
    let mut decoder = ctx.decoder().video().map_err(|e| format!("decoder: {e}"))?;

    let mut frame = ffmpeg::frame::Video::empty();
    let mut rgb = ffmpeg::frame::Video::empty();
    let mut scaler: Option<ffmpeg::software::scaling::context::Context> = None;

    // Try to decode the first decodable frame
    let mut found = false;
    let mut packed: Vec<u8> = Vec::new();
    let mut out_w: u32 = 0;
    let mut out_h: u32 = 0;

    'outer: for (s, packet) in ictx.packets() {
        if s.index() != video_idx {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|e| format!("send_packet: {e}"))?;
        if decoder.receive_frame(&mut frame).is_ok() {
            let (w, h, data) = convert_frame_to_rgb24(&mut frame, &mut rgb, &mut scaler)?;
            packed = data;
            out_w = w;
            out_h = h;
            found = true;
            break 'outer;
        }
    }

    if !found {
        // Flush decoder and try drained frames (e.g. single-frame video)
        let _ = decoder.send_eof();
        if decoder.receive_frame(&mut frame).is_ok() {
            let (w, h, data) = convert_frame_to_rgb24(&mut frame, &mut rgb, &mut scaler)?;
            packed = data;
            out_w = w;
            out_h = h;
            found = true;
        }
    }

    if !found {
        return Err("no frame decoded from video".to_string());
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let img = image::RgbImage::from_raw(out_w, out_h, packed)
        .ok_or_else(|| "failed to create image buffer".to_string())?;
    let tmp = dest.with_extension("tmp.png");
    img.save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

fn convert_frame_to_rgb24(
    frame: &mut ffmpeg_next::frame::Video,
    rgb: &mut ffmpeg_next::frame::Video,
    scaler: &mut Option<ffmpeg_next::software::scaling::context::Context>,
) -> Result<(u32, u32, Vec<u8>), String> {
    use ffmpeg_next as ffmpeg;
    let w = frame.width();
    let h = frame.height();
    let fmt = frame.format();
    if scaler.is_none()
        || scaler
            .as_ref()
            .map(|s| s.input().format != fmt || s.input().width != w || s.input().height != h)
            .unwrap_or(true)
    {
        *scaler = Some(
            ffmpeg::software::scaling::context::Context::get(
                fmt,
                w,
                h,
                ffmpeg::format::Pixel::RGB24,
                w,
                h,
                ffmpeg::software::scaling::flag::Flags::BILINEAR,
            )
            .map_err(|e| format!("scaler init: {e}"))?,
        );
    }
    scaler
        .as_mut()
        .unwrap()
        .run(frame, rgb)
        .map_err(|e| format!("scaler run: {e}"))?;

    let stride = rgb.stride(0);
    let row_bytes = w as usize * 3;
    let data = rgb.data(0);
    let mut packed = Vec::with_capacity(row_bytes * h as usize);
    for row in data.chunks(stride).take(h as usize) {
        packed.extend_from_slice(&row[..row_bytes]);
    }
    Ok((w, h, packed))
}

fn run_matugen(image_path: &Path, config: &MatugenConfig) -> Result<(), ThemeError> {
    let mut cmd = Command::new("matugen");
    cmd.arg("image")
        .arg(image_path)
        .arg("--mode")
        .arg(&config.mode)
        .arg("--type")
        .arg(&config.scheme)
        .arg("--contrast")
        .arg(config.contrast.to_string())
        .arg("--source-color-index")
        .arg("0")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    for arg in &config.args {
        cmd.arg(arg);
    }

    let mut child = cmd.spawn().map_err(ThemeError::SpawnError)?;

    if config.wait {
        let status = child.wait().map_err(ThemeError::SpawnError)?;
        if !status.success() {
            return Err(ThemeError::NonZeroExit(
                status.code().unwrap_or(-1),
                "matugen failed".to_string(),
            ));
        }
    }

    Ok(())
}

fn run_wallust(image_path: &Path) -> Result<(), ThemeError> {
    let mut cmd = Command::new("wallust");
    cmd.arg("run")
        .arg(image_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = cmd.status().map_err(ThemeError::SpawnError)?;
    if !status.success() {
        return Err(ThemeError::NonZeroExit(
            status.code().unwrap_or(-1),
            "wallust failed".to_string(),
        ));
    }

    Ok(())
}

fn run_pywal(image_path: &Path) -> Result<(), ThemeError> {
    let mut cmd = Command::new("wal");
    cmd.arg("-i")
        .arg(image_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = cmd.status().map_err(ThemeError::SpawnError)?;
    if !status.success() {
        return Err(ThemeError::NonZeroExit(
            status.code().unwrap_or(-1),
            "pywal failed".to_string(),
        ));
    }

    Ok(())
}

pub fn check_provider_available(provider: &ThemeProvider) -> bool {
    let binary = match provider {
        ThemeProvider::Matugen => "matugen",
        ThemeProvider::Wallust => "wallust",
        ThemeProvider::Pywal => "wal",
        ThemeProvider::None => return true,
    };

    Command::new("which")
        .arg(binary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|out| out.success())
        .unwrap_or(false)
}

pub fn detect_matugen_loop_risk() -> Option<String> {
    if let Ok(status) = fs::read_to_string("/proc/self/status")
        && let Some(ppid_line) = status.lines().find(|l| l.starts_with("PPid:"))
        && let Some(ppid) = ppid_line.split_whitespace().nth(1)
    {
        let cmdline_path = format!("/proc/{}/cmdline", ppid);
        if let Ok(cmdline) = fs::read_to_string(&cmdline_path)
            && cmdline.contains("matugen")
        {
            return Some("Detected matugen as parent process. This might cause an infinite loop if wallr is triggered by matugen.".to_string());
        }
    }
    None
}

/// Runs hook commands sequentially. User hooks output is preserved.
pub fn run_hooks(hooks: &[String]) -> Result<(), ThemeError> {
    for hook in hooks {
        let status = Command::new("sh")
            .arg("-c")
            .arg(hook)
            .status()
            .map_err(ThemeError::SpawnError)?;

        if !status.success() {
            return Err(ThemeError::NonZeroExit(
                status.code().unwrap_or(-1),
                format!("hook failed: {}", hook),
            ));
        }
    }
    Ok(())
}

/// Runs reload commands, via shell or pkill quietly (swallows failure if app isn't running).
pub fn run_reload_list(commands: &[String]) -> Result<(), ThemeError> {
    for cmd in commands {
        if cmd.contains(' ') {
            let _ = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        } else {
            let _ = Command::new("pkill")
                .arg("-SIGUSR2")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_provider_available_none() {
        assert!(check_provider_available(&ThemeProvider::None));
    }

    #[test]
    fn test_dispatch_none() {
        let matugen_cfg = MatugenConfig {
            enabled: false,
            mode: "dark".to_string(),
            scheme: "scheme-tonal-spot".to_string(),
            contrast: 0,
            wait: false,
            args: vec![],
        };
        let res = dispatch_theme(&ThemeProvider::None, Path::new("test.jpg"), &matugen_cfg);
        assert!(res.is_ok());
    }

    #[test]
    fn test_run_hooks_empty() {
        let res = run_hooks(&[]);
        assert!(res.is_ok());
    }

    #[test]
    fn test_detect_loop_risk() {
        // Should not detect a loop when wallr is not invoked by matugen
        assert!(detect_matugen_loop_risk().is_none());
    }
}
