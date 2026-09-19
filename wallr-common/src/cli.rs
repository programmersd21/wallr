use crate::effect::{Effect, EffectOverrides, apply_effect_overrides, effect_from_name};
use crate::types::{ScalingMode, ThemeProvider};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Transition options for wallpaper changes.
#[derive(Args, Debug, Clone, Default)]
pub struct EffectArgs {
    /// Transition effect: simple, fade, wipe, slide, left, right, top, bottom, wave, grow, center, outer, any, random
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

/// Validate `--effect` against the known effect names.
fn parse_effect_name(s: &str) -> Result<String, String> {
    if crate::effect::effect_names().contains(&s) {
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
    /// Trigger preview transition
    Preview,
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
