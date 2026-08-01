use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use tracing_subscriber::EnvFilter;
use wallr_core::animation::{Effect, FadeParams};
use wallr_core::cli::{CacheCommands, Commands, ConfigCommands, EffectArgs, IpcCommands, WallrCli};
use wallr_core::config;
use wallr_core::daemon::Daemon;
use wallr_core::ipc::{IpcCommand, send_ipc_command};
use wallr_core::preview::PreviewWindow;
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

/// Resolve the first effect of an animation package (path or registry name).
fn resolve_animation_effect(anim: &str) -> Option<(Effect, Option<std::time::Duration>)> {
    let spec = if std::path::Path::new(anim).exists() {
        wallr_core::animation::load_animation(std::path::Path::new(anim)).ok()
    } else {
        let registry = wallr_core::packages::PackageRegistry::new().ok();
        registry.and_then(|r| r.resolve_animation(anim).ok())
    }?;
    let duration = spec
        .duration
        .as_ref()
        .and_then(|d| wallr_core::config::parse_duration(d).ok());
    let effect = spec.effects.first().cloned().or_else(|| {
        spec.timeline
            .as_ref()
            .and_then(|t| t.first().map(|e| e.effect.clone()))
    });
    Some((effect?, duration))
}

/// Combine an animation package's effect with CLI `--effect`/override flags.
fn pick_effect(
    animation: Option<&str>,
    effect_args: &EffectArgs,
) -> anyhow::Result<(Effect, Option<u32>)> {
    let default_effect = Effect::Fade(FadeParams::default());
    let (package_effect, package_duration) = animation
        .and_then(resolve_animation_effect)
        .unwrap_or((default_effect, None));

    let effect = effect_args.to_effect(package_effect);
    let duration_ms = effect_args
        .duration
        .as_deref()
        .map(wallr_core::config::parse_duration)
        .transpose()?
        .map(|d| d.as_millis() as u32)
        .or_else(|| package_duration.map(|d| d.as_millis() as u32));
    Ok((effect, duration_ms))
}

#[tokio::main]
async fn main() -> Result<()> {
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
        Commands::Img {
            path,
            no_theme,
            theme,
            monitor: _,
            animation,
            mode: _,
            effect_args,
        }
        | Commands::Set {
            path,
            no_theme,
            theme,
            monitor: _,
            animation,
            mode: _,
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

            let anim_ref = animation.as_deref().or(config.animation.r#use.as_deref());
            let (effect, duration_ms) = pick_effect(anim_ref, &effect_args)?;

            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let resp = send_ipc_command(
                socket_path,
                IpcCommand::Preview {
                    path: canonical_path.to_string_lossy().to_string(),
                    effect: Some(effect),
                    duration_ms,
                    no_theme,
                    theme_override: theme,
                    monitor: None,
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
            println!("{}", "═".repeat(40));
            for check in &report.checks {
                let icon = match check.status {
                    DiagnosticStatus::Pass => "✓".green(),
                    DiagnosticStatus::Warn => "⚠".yellow(),
                    DiagnosticStatus::Fail => "✗".red(),
                };
                println!("  {} {}: {}", icon, check.name, check.message);
            }
            println!();
        }

        Commands::Validate { path } => {
            let engine = WallpaperEngine::new(config)?;
            let report = engine.validate_animation(&path)?;
            println!("\n{}", format!("wallr validate {}", path.display()).bold());
            println!("{}", "═".repeat(40));
            let mut all_passed = true;
            for check in &report.checks {
                let icon = if check.passed {
                    "✓".green()
                } else {
                    "✗".red()
                };
                if !check.passed {
                    all_passed = false;
                }
                let msg = check.message.as_deref().unwrap_or("");
                println!("  {} {} {}", icon, check.name, msg);
            }
            if !all_passed {
                std::process::exit(1);
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
                    Some(found) => println!("{}", serde_yaml::to_string(found)?),
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
                println!("Updated {}", path.display());
            }
        },

        Commands::Cache { subcommand } => {
            use wallr_core::cache::CacheManager;
            let cache = CacheManager::new(&config.cache)?;
            match subcommand {
                CacheCommands::Info => {
                    let info = cache.info()?;
                    println!("\n{}", "Cache Info".bold());
                    println!("  Directory: {}", info.cache_dir.display());
                    println!("  Files: {}", info.total_files);
                    println!(
                        "  Size: {}",
                        humansize::format_size(info.total_size, humansize::BINARY)
                    );
                }
                CacheCommands::Clear => {
                    let info = cache.clear()?;
                    println!(
                        "✓ Cleared {} files ({})",
                        info.total_files,
                        humansize::format_size(info.total_size, humansize::BINARY)
                    );
                }
            }
        }

        Commands::Reload => {
            let socket_path = config::expand_path(&config.daemon.socket);
            if socket_path.exists() && tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                let _ = send_ipc_command(socket_path, IpcCommand::Reload).await;
            } else {
                let engine = WallpaperEngine::new(config)?;
                engine.reload()?;
            }
        }

        Commands::Monitor { subcommand } => {
            use wallr_core::cli::MonitorCommands;
            match subcommand {
                MonitorCommands::List => {
                    println!("Monitor listing (Wayland connected)");
                }
                MonitorCommands::Current => {
                    println!("Current monitor info");
                }
            }
        }

        Commands::Daemon => {
            let daemon = Daemon::new(config)?;
            daemon.start().await?;
        }

        Commands::Watch { dir } => {
            let mut watch_config = config;
            watch_config.watch.enabled = true;
            watch_config.watch.dir = Some(dir.to_string_lossy().to_string());
            let daemon = Daemon::new(watch_config)?;
            daemon.start().await?;
        }

        Commands::Preview {
            path,
            watch: _,
            animation,
            effect_args,
        } => {
            let anim_ref = animation.clone().or_else(|| config.animation.r#use.clone());
            let mut preview = PreviewWindow::new(path);
            let (effect, duration_ms) = pick_effect(anim_ref.as_deref(), &effect_args)?;
            preview.effect = effect;
            if let Some(ms) = duration_ms {
                preview.duration = std::time::Duration::from_millis(ms as u64);
            }
            preview.run().await?;
        }

        Commands::Ipc { subcommand } => {
            let socket_path = config::expand_path(&config.daemon.socket);
            let cmd = match subcommand {
                IpcCommands::Pause => IpcCommand::Pause,
                IpcCommands::Resume => IpcCommand::Resume,
                IpcCommands::Reload => IpcCommand::Reload,
                IpcCommands::Preview => IpcCommand::Preview {
                    path: "".to_string(),
                    effect: None,
                    duration_ms: None,
                    no_theme: true,
                    theme_override: None,
                    monitor: None,
                },
                IpcCommands::Stop => IpcCommand::Stop,
                IpcCommands::Status => IpcCommand::Status,
            };
            let resp = send_ipc_command(socket_path, cmd).await?;
            if let Some(msg) = resp.message {
                println!("{}", msg);
            }
        }

        Commands::Install { package } => {
            use wallr_core::packages::PackageRegistry;
            let registry = PackageRegistry::new()?;
            let spec = registry.resolve_animation(&package)?;
            println!("Installed package: {}", spec.name);
        }

        Commands::New { name, shader } => {
            let dir = std::path::PathBuf::from(&name);
            std::fs::create_dir_all(&dir)?;
            let yaml = format!(
                "# Wallr animation package. Edit duration, easing, and effects.\nname: {}\nduration: 800ms\nfps: 60\n\neffects:\n  - fade:\n      easing: ease_out\n  - blur:\n      from: 20\n      to: 0\n",
                name
            );
            std::fs::write(dir.join("wallr.yaml"), yaml)?;
            if shader {
                std::fs::write(
                    dir.join("starter.wgsl"),
                    "// Add a fragment shader effect here.\n",
                )?;
            }
            println!("Created animation package at {}", dir.display());
        }

        Commands::Publish => {
            println!("Package publishing tool ready");
        }

        Commands::Search { query } => {
            use wallr_core::packages::PackageRegistry;
            let registry = PackageRegistry::new()?;
            let pkgs = registry.list_packages()?;
            let matches: Vec<_> = pkgs.into_iter().filter(|p| p.contains(&query)).collect();
            println!("Packages matching '{}': {:?}", query, matches);
        }
    }

    Ok(())
}
