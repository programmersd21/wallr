use crate::effect::{Effect, EffectOverrides, apply_effect_overrides, effect_from_name};
use crate::types::{ScalingMode, ThemeProvider};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Transition options for wallpaper changes.
#[derive(Args, Debug, Clone, Default)]
pub struct EffectArgs {
    /// Transition effect: fade, wipe, slide, wave, grow, outer (plus aliases:
    /// simple, left, right, top, bottom, center, any, random)
    #[arg(short = 'e', long, value_name = "NAME", value_parser = parse_effect_name)]
    pub effect: Option<String>,

    /// Transition duration (e.g. 700ms, 1s, 1.2s)
    #[arg(short = 'd', long, value_name = "TIME")]
    pub duration: Option<String>,

    /// Effect origin: preset (top_left, top, top_right, left, center, right, bottom_left, bottom, bottom_right) or "x,y" (0..1)
    #[arg(short = 'o', long, value_name = "PRESET|X,Y")]
    pub origin: Option<String>,

    /// Angle for wipe/slide in degrees (0 = right-to-left, 90 = top-to-bottom, 270 = bottom-to-top)
    #[arg(short = 'a', long, value_name = "DEG")]
    pub angle: Option<f32>,

    /// Direction vector "x,y" (e.g. "1,0" or "-1,0")
    #[arg(long, value_name = "X,Y")]
    pub direction: Option<String>,

    /// Easing curve: linear, ease_in, ease_out, ease_in_out, bezier
    #[arg(long, value_enum)]
    pub easing: Option<crate::effect::Easing>,

    /// Initial parameter value (fade opacity)
    #[arg(long, value_name = "VAL")]
    pub from: Option<f32>,

    /// Target parameter value (fade opacity)
    #[arg(long, value_name = "VAL")]
    pub to: Option<f32>,

    /// Wave frequency in Hz
    #[arg(long, value_name = "HZ")]
    pub frequency: Option<f32>,

    /// Wave amplitude
    #[arg(long, value_name = "VAL")]
    pub amplitude: Option<f32>,

    /// Wipe feather softness (0.01 - 0.5)
    #[arg(long, value_name = "VAL")]
    pub softness: Option<f32>,
}

impl EffectArgs {
    /// Build an `Effect` from `--effect <name>` + all override flags.
    /// Falls back to `fallback` when no effect name is given.
    pub fn to_effect(&self, fallback: Effect) -> Effect {
        let mut effect = self
            .effect
            .as_deref()
            .and_then(effect_from_name)
            .unwrap_or(fallback);
        apply_effect_overrides(&mut effect, &self.to_overrides());
        effect
    }

    pub fn to_overrides(&self) -> EffectOverrides {
        let origin = self.origin.as_deref().and_then(parse_origin);
        EffectOverrides {
            origin,
            origin_preset: if origin.is_none() {
                self.origin.clone()
            } else {
                None
            },
            direction: self.direction.as_deref().and_then(parse_vec2),
            angle: self.angle,
            easing: self.easing,
            from: self.from,
            to: self.to,
            frequency: self.frequency,
            amplitude: self.amplitude,
            softness: self.softness,
        }
    }
}

/// Validate `--effect`. Aliases (left, right, top, bottom, center, any,
/// random, simple) resolve through the real parser; the error message shows
/// only the six canonical transitions.
fn parse_effect_name(s: &str) -> Result<String, String> {
    if crate::effect::effect_from_name(s).is_some() {
        Ok(s.to_string())
    } else {
        Err(format!(
            "unknown effect '{}' - expected one of: {}",
            s,
            crate::effect::effect_names().join(", ")
        ))
    }
}

/// Parse "x,y" into a normalized origin, or None if it's not numeric.
fn parse_origin(s: &str) -> Option<(f32, f32)> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse::<f32>().ok()?;
    let y = parts.next()?.trim().parse::<f32>().ok()?;
    Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
}

/// Parse "x,y" into a direction vector.
fn parse_vec2(s: &str) -> Option<[f32; 2]> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse::<f32>().ok()?;
    let y = parts.next()?.trim().parse::<f32>().ok()?;
    Some([x, y])
}

/// Parse an IPC seek timestamp into milliseconds.
///
/// Accepts `SS`, `MM:SS`, or `HH:MM:SS` with optional fractional seconds for
/// the bare-seconds form. Rejects non-finite, negative, and overflowing input
/// instead of saturating or wrapping through a cast.
pub fn parse_timestamp_ms(timestamp: &str) -> Result<u64, String> {
    let timestamp = timestamp.trim();
    if timestamp.is_empty() {
        return Err("timestamp cannot be empty".to_string());
    }
    if timestamp.contains(':') {
        let parts: Vec<&str> = timestamp.split(':').collect();
        if !matches!(parts.len(), 2 | 3) {
            return Err("invalid timestamp format. Use HH:MM:SS or seconds".to_string());
        }
        let mut values = Vec::with_capacity(parts.len());
        for part in &parts {
            let value: u64 = part
                .parse()
                .map_err(|_| format!("invalid timestamp component: {part}"))?;
            values.push(value);
        }
        let seconds = if values.len() == 3 {
            if values[1] >= 60 || values[2] >= 60 {
                return Err("minutes and seconds must be below 60".to_string());
            }
            values[0]
                .checked_mul(3600)
                .and_then(|v| v.checked_add(values[1] * 60))
                .and_then(|v| v.checked_add(values[2]))
                .ok_or_else(|| "timestamp overflows".to_string())?
        } else {
            if values[1] >= 60 {
                return Err("seconds must be below 60".to_string());
            }
            values[0]
                .checked_mul(60)
                .and_then(|v| v.checked_add(values[1]))
                .ok_or_else(|| "timestamp overflows".to_string())?
        };
        seconds
            .checked_mul(1000)
            .ok_or_else(|| "timestamp overflows".to_string())
    } else {
        let seconds: f64 = timestamp
            .parse()
            .map_err(|_| format!("invalid timestamp: {timestamp}"))?;
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("timestamp must be a finite non-negative number".to_string());
        }
        let millis = (seconds * 1000.0).round();
        if millis > u64::MAX as f64 {
            return Err("timestamp overflows".to_string());
        }
        Ok(millis as u64)
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "wallr",
    about = "Wayland wallpaper engine with GPU animations and theme pipelines",
    version
)]
pub struct WallrCli {
    #[command(subcommand)]
    pub command: Commands,

    /// Custom config file path
    #[arg(global = true, short = 'c', long)]
    pub config: Option<PathBuf>,

    /// Verbose logging (-v, -vv)
    #[arg(global = true, short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode (suppress non-error output)
    #[arg(global = true, short = 'q', long)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Set wallpaper image or video
    #[command(alias = "img")]
    Set {
        /// Wallpaper file path (image, GIF, or video); `-` reads from stdin
        path: PathBuf,

        /// Target output/monitor
        #[arg(short = 'm', long, value_name = "OUTPUT")]
        monitor: Option<String>,

        /// Scaling mode: fill, fit, stretch, center, tile
        #[arg(long, value_enum)]
        mode: Option<ScalingMode>,

        /// Disable theme extraction
        #[arg(long)]
        no_theme: bool,

        /// Override theme engine (matugen, wallust, pywal)
        #[arg(short = 't', long, value_enum)]
        theme: Option<ThemeProvider>,

        /// Transition overrides
        #[command(flatten)]
        effect_args: EffectArgs,
    },

    /// Run system and dependency diagnostics
    Doctor,

    /// View and edit configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },

    /// Reload config and restart theme hooks
    Reload,

    /// Query connected outputs
    Monitor {
        #[command(subcommand)]
        subcommand: MonitorCommands,
    },

    /// Run background daemon
    Daemon {
        /// Max render FPS limit
        #[arg(long)]
        max_fps: Option<u32>,
    },

    /// Daemon IPC controls (playback, blanking, info)
    Ipc {
        #[command(subcommand)]
        subcommand: IpcCommands,
    },

    /// Stop running daemon
    Quit,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Get config value by key
    Get { key: String },
    /// Set config value by key
    Set { key: String, value: String },
    /// Print path to active config file
    Path,
}

#[derive(Subcommand, Debug)]
pub enum MonitorCommands {
    /// List all connected display outputs
    List,
    /// Display current focused output
    Current,
}

#[derive(Subcommand, Debug)]
pub enum IpcCommands {
    /// Pause video/GIF playback
    Pause {
        /// Target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// Resume video/GIF playback
    Resume {
        /// Target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// Re-render current wallpaper
    Reload,
    /// Shut down the daemon
    Stop,
    /// Query daemon status
    Status,
    /// Query GPU, video decoder, and display info
    Info {
        /// Target specific monitor
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// Seek video to position (HH:MM:SS or seconds)
    Seek {
        /// Position in HH:MM:SS or seconds
        timestamp: String,
        /// Target specific monitor
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// Blank output to black with transition
    Blank {
        /// Target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
        #[command(flatten)]
        effect_args: EffectArgs,
    },
    /// Restore wallpaper on blanked output
    Restore {
        /// Target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
        #[command(flatten)]
        effect_args: EffectArgs,
    },
}

#[cfg(test)]
mod tests {
    use super::parse_timestamp_ms;

    #[test]
    fn parses_seek_timestamps() {
        assert_eq!(parse_timestamp_ms("0").unwrap(), 0);
        assert_eq!(parse_timestamp_ms("90").unwrap(), 90_000);
        assert_eq!(parse_timestamp_ms("1.5").unwrap(), 1_500);
        assert_eq!(parse_timestamp_ms("01:30").unwrap(), 90_000);
        assert_eq!(parse_timestamp_ms("01:02:03").unwrap(), 3_723_000);
    }

    #[test]
    fn rejects_invalid_seek_timestamps() {
        for value in ["", "-1", "NaN", "inf", "1e300", "1:60", "1:2:60", "1:2:3:4"] {
            assert!(parse_timestamp_ms(value).is_err(), "{value} should fail");
        }
    }
}
