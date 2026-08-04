use crate::animation::{Effect, EffectOverrides, apply_effect_overrides, effect_from_name};
use crate::config::{ScalingMode, ThemeProvider};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Transition + customization flags shared by `img` / `set` / `preview`.
#[derive(Args, Debug, Clone, Default)]
pub struct EffectArgs {
    /// Transition effect: simple, fade, blur, wipe, slide, left, right, top,
    /// bottom, zoom, pixelate, ripple, dissolve, wave, grow, center, outer,
    /// any, random
    #[arg(long, value_name = "NAME", value_parser = parse_effect_name)]
    pub effect: Option<String>,

    /// Transition duration, for example 800ms, 1s, or 1.2s
    #[arg(long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// Effect origin: a preset (top_left, top, top_right, left, center, right,
    /// bottom_left, bottom, bottom_right) or a normalized "x,y" (0..1)
    #[arg(long, value_name = "PRESET|X,Y")]
    pub origin: Option<String>,

    /// Wipe/wave travel angle in degrees (0 = right, 90 = up)
    #[arg(long, value_name = "DEG")]
    pub angle: Option<f32>,

    /// Wipe/slide direction vector "x,y" (e.g. "1,0" for left-to-right)
    #[arg(long, value_name = "X,Y")]
    pub direction: Option<String>,

    /// Easing curve: linear, ease_in, ease_out, ease_in_out
    #[arg(long, value_enum)]
    pub easing: Option<crate::animation::Easing>,

    /// Start value (fade opacity, blur radius, zoom scale, pixelate size)
    #[arg(long, value_name = "VALUE")]
    pub from: Option<f32>,

    /// End value (fade opacity, blur radius, zoom scale, pixelate size)
    #[arg(long, value_name = "VALUE")]
    pub to: Option<f32>,

    /// Wave/ripple frequency
    #[arg(long, value_name = "HZ")]
    pub frequency: Option<f32>,

    /// Wave/ripple amplitude
    #[arg(long, value_name = "VALUE")]
    pub amplitude: Option<f32>,

    /// Ripple expansion speed
    #[arg(long, value_name = "VALUE")]
    pub speed: Option<f32>,

    /// Wipe feather / dissolve edge softness
    #[arg(long, value_name = "VALUE")]
    pub softness: Option<f32>,

    /// Dissolve noise scale
    #[arg(long, value_name = "VALUE")]
    pub scale: Option<f32>,
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
            speed: self.speed,
            softness: self.softness,
            scale: self.scale,
        }
    }
}

/// Validate `--effect` against the known effect names.
fn parse_effect_name(s: &str) -> Result<String, String> {
    if crate::animation::effect_names().contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "unknown effect '{}' — expected one of: {}",
            s,
            crate::animation::effect_names().join(", ")
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

/// Parse "x,y" into a direction vector (not normalized here — shader normalizes).
fn parse_vec2(s: &str) -> Option<[f32; 2]> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse::<f32>().ok()?;
    let y = parts.next()?.trim().parse::<f32>().ok()?;
    Some([x, y])
}

#[derive(Parser, Debug)]
#[command(
    name = "wallr",
    about = "Wayland wallpaper engine with animation and theme pipelines",
    version
)]
pub struct WallrCli {
    #[command(subcommand)]
    pub command: Commands,

    /// Custom config file path
    #[arg(global = true, long)]
    pub config: Option<PathBuf>,

    /// Increase log verbosity
    #[arg(global = true, short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-error output
    #[arg(global = true, short = 'q', long)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Set wallpaper with animation + theme pipeline
    Img {
        /// Path to the wallpaper image
        path: PathBuf,
        /// Disable theme generation
        #[arg(long)]
        no_theme: bool,
        /// Theme provider override for this call (matugen, wallust, pywal)
        #[arg(long, value_enum)]
        theme: Option<ThemeProvider>,
        /// Target monitor
        #[arg(long)]
        monitor: Option<String>,
        /// Animation package to use
        #[arg(long)]
        animation: Option<String>,
        /// Scaling mode
        #[arg(long, value_enum)]
        mode: Option<ScalingMode>,
        /// Transition effect + customization flags
        #[command(flatten)]
        effect_args: EffectArgs,
    },
    /// Alias/superset of img
    Set {
        /// Path to the wallpaper image
        path: PathBuf,
        /// Disable theme generation
        #[arg(long)]
        no_theme: bool,
        /// Theme provider override for this call (matugen, wallust, pywal)
        #[arg(long, value_enum)]
        theme: Option<ThemeProvider>,
        /// Target monitor
        #[arg(long)]
        monitor: Option<String>,
        /// Animation package to use
        #[arg(long)]
        animation: Option<String>,
        /// Scaling mode
        #[arg(long, value_enum)]
        mode: Option<ScalingMode>,
        /// Transition effect + customization flags
        #[command(flatten)]
        effect_args: EffectArgs,
    },
    /// Environment diagnostics
    Doctor,
    /// Lint animation YAML
    Validate {
        /// Path to animation YAML file
        path: PathBuf,
    },
    /// Configuration management
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },
    /// Cache management
    Cache {
        #[command(subcommand)]
        subcommand: CacheCommands,
    },
    /// Re-run reload list without changing wallpaper
    Reload,
    /// Monitor management
    Monitor {
        #[command(subcommand)]
        subcommand: MonitorCommands,
    },

    // Core Daemon & Package commands
    /// Start the background wallpaper daemon
    Daemon,
    /// Create a working animation package starter.
    New {
        /// Package name or output directory.
        name: String,
        /// Include a raw WGSL shader starter.
        #[arg(long)]
        shader: bool,
    },
    /// Watch a directory for new images and automatically set them
    Watch {
        /// Directory to watch
        dir: PathBuf,
    },
    /// Preview an animation or wallpaper image
    Preview {
        /// Path to preview
        path: PathBuf,
        /// Watch for changes
        #[arg(long)]
        watch: bool,
        /// Animation package or YAML file to preview
        #[arg(long, value_name = "PACKAGE|FILE")]
        animation: Option<String>,
        /// Transition effect + customization flags
        #[command(flatten)]
        effect_args: EffectArgs,
    },
    /// Install an animation package from registry or repository
    Install {
        /// Package to install
        package: String,
    },
    /// Publish an animation package to the registry
    Publish,
    /// Search for packages in the registry
    Search {
        /// Query to search for
        query: String,
    },
    /// Control the running daemon via IPC commands
    Ipc {
        #[command(subcommand)]
        subcommand: IpcCommands,
    },
    /// Gracefully stop the running daemon (alias for `ipc stop`)
    Quit,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Get a configuration value
    Get { key: String },
    /// Set a configuration value
    Set { key: String, value: String },
    /// Print the config path
    Path,
}

#[derive(Subcommand, Debug)]
pub enum CacheCommands {
    /// Clear the cache
    Clear,
    /// Show cache info
    Info,
}

#[derive(Subcommand, Debug)]
pub enum MonitorCommands {
    /// List monitors
    List,
    /// Show current monitor
    Current,
}

#[derive(Subcommand, Debug)]
pub enum IpcCommands {
    /// Pause animations
    Pause {
        /// Target specific monitor (default: all)
        #[arg(long)]
        monitor: Option<String>,
    },
    /// Resume animations
    Resume {
        /// Target specific monitor (default: all)
        #[arg(long)]
        monitor: Option<String>,
    },
    /// Reload wallpaper
    Reload,
    /// Preview animation
    Preview,
    /// Stop daemon
    Stop,
    /// Get daemon status
    Status,
    /// Get video decoder and GPU information
    Info {
        /// Target specific monitor (default: all outputs)
        #[arg(long)]
        monitor: Option<String>,
    },
    /// Seek video to timestamp (format: HH:MM:SS or seconds)
    Seek {
        /// Timestamp in format HH:MM:SS or seconds
        timestamp: String,
        /// Target specific monitor (default: first output)
        #[arg(long)]
        monitor: Option<String>,
    },
    /// Blank an output without replacing persisted wallpaper
    Blank {
        /// Target specific monitor (default: all)
        #[arg(long)]
        monitor: Option<String>,
    },
    /// Restore a previously blanked output
    Restore {
        /// Target specific monitor (default: all)
        #[arg(long)]
        monitor: Option<String>,
    },
}
