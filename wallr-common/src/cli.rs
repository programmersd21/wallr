use crate::effect::{Effect, EffectOverrides, apply_effect_overrides, effect_from_name};
use crate::types::{ScalingMode, ThemeProvider};
use clap::builder::styling::{AnsiColor, Styles};
use clap::{ArgAction, Args, Parser, Subcommand};
use std::path::PathBuf;

/// transition options for wallpaper changes.
#[derive(Args, Debug, Clone, Default)]
pub struct EffectArgs {
    /// Transition effect: fade, wipe, slide, wave, grow, outer (plus aliases:
    /// simple, left, right, top, bottom, center, any, random)
    #[arg(short = 'e', long, value_name = "name", value_parser = parse_effect_name)]
    pub effect: Option<String>,

    /// Transition duration (e.g. 700ms, 1s, 1.2s)
    #[arg(short = 'd', long, value_name = "time")]
    pub duration: Option<String>,

    /// Effect origin: preset (top_left, top, top_right, left, center, right, bottom_left, bottom, bottom_right) or "x,y" (0..1)
    #[arg(short = 'o', long, value_name = "preset|x,y")]
    pub origin: Option<String>,

    /// Angle for wipe/slide in degrees (0 = right-to-left, 90 = top-to-bottom, 270 = bottom-to-top)
    #[arg(short = 'a', long, value_name = "deg")]
    pub angle: Option<f32>,

    /// Direction vector "x,y" (e.g. "1,0" or "-1,0")
    #[arg(long, value_name = "x,y")]
    pub direction: Option<String>,

    /// Easing curve: linear, ease_in, ease_out, ease_in_out, bezier
    #[arg(long, value_enum)]
    pub easing: Option<crate::effect::Easing>,

    /// Initial parameter value (fade opacity)
    #[arg(long, value_name = "val")]
    pub from: Option<f32>,

    /// Target parameter value (fade opacity)
    #[arg(long, value_name = "val")]
    pub to: Option<f32>,

    /// Wave frequency in Hz
    #[arg(long, value_name = "hz")]
    pub frequency: Option<f32>,

    /// Wave amplitude
    #[arg(long, value_name = "val")]
    pub amplitude: Option<f32>,

    /// Wipe feather softness (0.01 - 0.5)
    #[arg(long, value_name = "val")]
    pub softness: Option<f32>,
}

impl EffectArgs {
    /// build an `Effect` from `--effect <name>` + all override flags.
    /// falls back to `fallback` when no effect name is given.
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
    about = "wayland wallpaper engine with gpu animations and theme pipelines",
    version,
    override_usage = "wallr [options] <command>",
    disable_help_subcommand = true,
    disable_version_flag = true,
    help_template = "{before-help}{name} {version}\n\n{about}\n\nusage:\n  {usage}\n\ncommands:\n{subcommands}\n\noptions:\n{options}{after-help}",
    styles = Styles::styled()
        .header(AnsiColor::Cyan.on_default().bold())
        .usage(AnsiColor::Blue.on_default().bold())
        .literal(AnsiColor::Green.on_default().bold())
        .placeholder(AnsiColor::Yellow.on_default())
)]
pub struct WallrCli {
    #[command(subcommand)]
    pub command: Commands,

    /// custom config file path
    #[arg(global = true, short = 'c', long, value_name = "path")]
    pub config: Option<PathBuf>,

    /// verbose logging (-v, -vv)
    #[arg(global = true, short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// quiet mode (suppress non-error output)
    #[arg(global = true, short = 'q', long)]
    pub quiet: bool,

    /// show version
    #[arg(short = 'V', long, action = ArgAction::Version)]
    pub version: Option<bool>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// set wallpaper image or video
    #[command(alias = "img")]
    Set {
        /// wallpaper file path (image, gif, or video); `-` reads from stdin
        #[arg(value_name = "path")]
        path: PathBuf,

        /// target output/monitor
        #[arg(short = 'm', long, value_name = "output")]
        monitor: Option<String>,

        /// scaling mode: fill, fit, stretch, center, tile
        #[arg(long, value_enum)]
        mode: Option<ScalingMode>,

        /// disable theme extraction
        #[arg(long)]
        no_theme: bool,

        /// override theme engine (matugen, wallust, pywal)
        #[arg(short = 't', long, value_enum)]
        theme: Option<ThemeProvider>,

        /// Transition overrides
        #[command(flatten)]
        effect_args: EffectArgs,
    },

    /// run system and dependency diagnostics
    Doctor,

    /// view and edit configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },

    /// reload config and restart theme hooks
    Reload,

    /// query connected outputs
    Monitor {
        #[command(subcommand)]
        subcommand: MonitorCommands,
    },

    /// run background daemon
    Daemon {
        /// max render fps limit
        #[arg(long, value_name = "fps")]
        max_fps: Option<u32>,
    },

    /// daemon ipc controls (playback, blanking, info)
    Ipc {
        #[command(subcommand)]
        subcommand: IpcCommands,
    },

    /// stop running daemon
    Quit,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// get config value by key
    Get {
        #[arg(value_name = "key")]
        key: String,
    },
    /// set config value by key
    Set {
        #[arg(value_name = "key")]
        key: String,
        #[arg(value_name = "value")]
        value: String,
    },
    /// print path to active config file
    Path,
}

#[derive(Subcommand, Debug)]
pub enum MonitorCommands {
    /// list all connected display outputs
    List,
    /// display current focused output
    Current,
}

#[derive(Subcommand, Debug)]
pub enum IpcCommands {
    /// pause video/gif playback
    Pause {
        /// target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// resume video/gif playback
    Resume {
        /// target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// re-render current wallpaper
    Reload,
    /// shut down the daemon
    Stop,
    /// query daemon status
    Status,
    /// query gpu, video decoder, and display info
    Info {
        /// target specific monitor
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// seek video to position (hh:mm:ss or seconds)
    Seek {
        /// position in hh:mm:ss or seconds
        #[arg(value_name = "timestamp")]
        timestamp: String,
        /// target specific monitor
        #[arg(short = 'm', long)]
        monitor: Option<String>,
    },
    /// blank output to black with transition
    Blank {
        /// target specific monitor (default: all)
        #[arg(short = 'm', long)]
        monitor: Option<String>,
        #[command(flatten)]
        effect_args: EffectArgs,
    },
    /// restore wallpaper on blanked output
    Restore {
        /// target specific monitor (default: all)
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
