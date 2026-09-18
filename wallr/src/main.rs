#![recursion_limit = "256"]

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use tracing_subscriber::EnvFilter;
use wallr_common::cli::{Commands, ConfigCommands, EffectArgs, IpcCommands, WallrCli};
use wallr_common::config;
use wallr_common::effect::{Effect, FadeParams};
use wallr_core::daemon::Daemon;
use wallr_core::ipc::{IpcCommand, send_ipc_command};
use wallr_core::wallpaper::{DiagnosticStatus, WallpaperEngine};

fn config_value<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a serde_yaml::Value> {
    key.split('.')
        .try_fold(value, |current, part| current.get(part))
}

fn set_config_value(
    value: &mut serde_yaml::Value,
    key: &str,
    replacement: serde_yaml::Value,
) -> anyhow::Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        anyhow::bail!("config key cannot be empty");
    }
    let mut current = value;
    for part in &parts[..parts.len() - 1] {
        current = current
            .get_mut(*part)
            .ok_or_else(|| anyhow::anyhow!("unknown config key: {key}"))?;
    }
    current[parts[parts.len() - 1]] = replacement;
    Ok(())
}

/// Combine CLI `--effect`/override flags into the transition effect.
/// With no `--effect` flag the default is a plain linear crossfade,
/// matching awww's default `simple` feel; explicit `-e fade` keeps polish.
fn pick_effect(effect_args: &EffectArgs) -> anyhow::Result<(Effect, Option<u32>)> {
    let effect = effect_args.to_effect(Effect::Fade(FadeParams {
        easing: wallr_common::effect::Easing::Linear,
        ..FadeParams::default()
    }));
    let duration_ms = effect_args
        .duration
        .as_deref()
        .map(wallr_common::config::parse_duration)
        .transpose()?
        .map(|d| d.as_millis() as u32);
    Ok((effect, duration_ms))
}

// All decoding, rendering, and filesystem work is dispatched through
// `spawn_blocking`; a current-thread scheduler avoids keeping one idle async
// worker per CPU in the long-lived daemon and keeps the IPC-only CLI light.
fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = WallrCli::parse();

    let filter = if cli.verbose > 0 {
        "wallr=debug"
    } else if cli.quiet {
        "wallr=error"
    } else {
        "wallr=off"
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .try_init();

    let config_path = cli.config.as_deref().map(std::path::Path::new);
    let config = config::load_config(config_path)?;

    match cli.command {
        Commands::Set {
            path,
            no_theme,
            theme,
            monitor,
            mode,
            effect_args,
        } => {
            let socket_path = config::expand_path(&config.daemon.socket);
            let daemon_running =
                socket_path.exists() && tokio::net::UnixStream::connect(&socket_path).await.is_ok();

            if !daemon_running {
                let exe = std::env::current_exe()?;
                let _ = std::process::Command::new(&exe)
                    .arg("daemon")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;

                let mut waited = 0u64;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    waited += 100;
                    if socket_path.exists()
                        && tokio::net::UnixStream::connect(&socket_path).await.is_ok()
                    {
                        break;
                    }
                    if waited >= 3000 {
                        anyhow::bail!(
                            "Daemon did not start within 3 seconds. \
                             Run `wallr daemon` manually and check for errors."
                        );
                    }
                }
            }

            let (effect, duration_ms) = pick_effect(&effect_args)?;

            // A `-` path reads the image bytes from stdin (like `awww img -`),
            // staging them in a temp file: downstream code paths key GIF
            // animation and video detection off the file extension.
            let path = if path.as_os_str() == "-" {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::io::stdin().read_to_end(&mut bytes)?;
                anyhow::ensure!(!bytes.is_empty(), "no image data on stdin");
                let ext = if bytes.starts_with(b"GIF8") {
                    "gif"
                } else {
                    "png"
                };
                let staged = std::env::temp_dir().join(format!(
                    "wallr-stdin-{}-{}.{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                    ext
                ));
                std::fs::write(&staged, &bytes)?;
                staged
            } else {
                path
            };
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let resp = send_ipc_command(
                socket_path,
                IpcCommand::Preview {
                    path: canonical_path.to_string_lossy().to_string(),
                    effect: Some(effect),
                    duration_ms,
                    no_theme,
                    theme_override: theme,
                    monitor,
                    scaling_mode: mode.or(Some(config.wallpaper.mode.clone())),
                },
            )
            .await?;

            if !resp.success
                && let Some(msg) = resp.message
            {
                anyhow::bail!("{}", msg);
            }
        }

        Commands::Doctor => {
            let engine = WallpaperEngine::new(config)?;
            let report = engine.doctor();
            println!("\n{}", "wallr doctor".bold());
            println!("{}", "─".repeat(32).dimmed());
            for check in &report.checks {
                let icon = match check.status {
                    DiagnosticStatus::Pass => "✓".green(),
                    DiagnosticStatus::Warn => "!".yellow(),
                    DiagnosticStatus::Fail => "✗".red(),
                };
                println!(
                    "  {} {:<22} {}",
                    icon,
                    check.name.bold(),
                    check.message.dimmed()
                );
            }
            println!();
        }

        Commands::Config { subcommand } => match subcommand {
            ConfigCommands::Path => {
                println!("{}", config::config_path().display());
            }
            ConfigCommands::Get { key } => {
                let yaml = serde_yaml::to_string(&config)?;
                let value: serde_yaml::Value = serde_yaml::from_str(&yaml)?;
                match config_value(&value, &key) {
                    Some(found) => println!("{}", serde_yaml::to_string(found)?.trim_end()),
                    None => anyhow::bail!("unknown config key: {key}"),
                }
            }
            ConfigCommands::Set { key, value } => {
                let path = cli.config.clone().unwrap_or_else(config::config_path);
                let mut yaml: serde_yaml::Value = if path.exists() {
                    serde_yaml::from_str(&std::fs::read_to_string(&path)?)?
                } else {
                    serde_yaml::to_value(&config)?
                };
                let replacement: serde_yaml::Value =
                    serde_yaml::from_str(&value).unwrap_or(serde_yaml::Value::String(value));
                set_config_value(&mut yaml, &key, replacement)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, serde_yaml::to_string(&yaml)?)?;
                println!("{} {}", "Updated".green(), path.display());
            }
        },

        Commands::Reload => {
            let socket_path = config::expand_path(&config.daemon.socket);
            if socket_path.exists() && tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                let resp = send_ipc_command(socket_path, IpcCommand::Reload).await?;
                if let Some(msg) = resp.message {
                    println!("{}", msg);
                }
            } else {
                let engine = WallpaperEngine::new(config)?;
                engine.reload()?;
                println!("{} Reloaded configuration & hooks", "✓".green());
            }
        }

        Commands::Monitor { subcommand } => {
            use wallr_common::cli::MonitorCommands;
            let socket_path = config::expand_path(&config.daemon.socket);
            match subcommand {
                MonitorCommands::List => {
                    let resp = send_ipc_command(socket_path, IpcCommand::MonitorList).await?;
                    if let Some(msg) = resp.message {
                        println!("{}", msg.trim_end());
                    }
                }
                MonitorCommands::Current => {
                    let resp = send_ipc_command(socket_path, IpcCommand::MonitorCurrent).await?;
                    if let Some(msg) = resp.message {
                        println!("{}", msg.trim_end());
                    }
                }
            }
        }

        Commands::Quit => {
            let socket_path = config::expand_path(&config.daemon.socket);
            if socket_path.exists() && tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                let resp = send_ipc_command(socket_path, IpcCommand::Stop).await?;
                if let Some(msg) = resp.message {
                    println!("{}", msg);
                }
            } else {
                anyhow::bail!("daemon is not running");
            }
        }

        Commands::Daemon { max_fps } => {
            let mut final_config = config;
            if let Some(fps) = max_fps {
                final_config.daemon.max_fps = Some(fps);
            }
            let daemon = Daemon::new(final_config)?;
            daemon.start().await?;
        }

        Commands::Ipc { subcommand } => {
            let socket_path = config::expand_path(&config.daemon.socket);
            let cmd = match subcommand {
                IpcCommands::Pause { monitor } => IpcCommand::Pause { monitor },
                IpcCommands::Resume { monitor } => IpcCommand::Resume { monitor },
                IpcCommands::Reload => IpcCommand::Reload,
                IpcCommands::Preview => IpcCommand::Preview {
                    path: "".to_string(),
                    effect: None,
                    duration_ms: None,
                    no_theme: true,
                    theme_override: None,
                    monitor: None,
                    scaling_mode: None,
                },
                IpcCommands::Stop => IpcCommand::Stop,
                IpcCommands::Status => IpcCommand::Status,
                IpcCommands::Info { monitor } => IpcCommand::Info { monitor },
                IpcCommands::Seek { timestamp, monitor } => {
                    let ms = if timestamp.contains(':') {
                        let parts: Vec<&str> = timestamp.split(':').collect();
                        match parts.len() {
                            2 => {
                                let min: u64 = parts[0].parse()?;
                                let sec: u64 = parts[1].parse()?;
                                (min * 60 + sec) * 1000
                            }
                            3 => {
                                let hr: u64 = parts[0].parse()?;
                                let min: u64 = parts[1].parse()?;
                                let sec: u64 = parts[2].parse()?;
                                (hr * 3600 + min * 60 + sec) * 1000
                            }
                            _ => anyhow::bail!("invalid timestamp format. Use HH:MM:SS or seconds"),
                        }
                    } else {
                        let sec: f64 = timestamp.parse()?;
                        (sec * 1000.0) as u64
                    };
                    IpcCommand::Seek {
                        timestamp_ms: ms,
                        monitor,
                    }
                }
                IpcCommands::Blank {
                    monitor,
                    effect_args,
                } => {
                    let (effect, duration_ms) = pick_effect(&effect_args)?;
                    let opt_effect = if effect_args.effect.is_some()
                        || effect_args.from.is_some()
                        || effect_args.to.is_some()
                    {
                        Some(effect)
                    } else {
                        None
                    };
                    IpcCommand::Blank {
                        monitor,
                        effect: opt_effect,
                        duration_ms,
                    }
                }
                IpcCommands::Restore {
                    monitor,
                    effect_args,
                } => {
                    let (effect, duration_ms) = pick_effect(&effect_args)?;
                    let opt_effect = if effect_args.effect.is_some()
                        || effect_args.from.is_some()
                        || effect_args.to.is_some()
                    {
                        Some(effect)
                    } else {
                        None
                    };
                    IpcCommand::Restore {
                        monitor,
                        effect: opt_effect,
                        duration_ms,
                    }
                }
            };
            let resp = send_ipc_command(socket_path, cmd).await?;
            if let Some(msg) = resp.message {
                println!("{}", msg);
            }
        }
    }

    Ok(())
}
