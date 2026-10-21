#![recursion_limit = "256"]

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use colored::Colorize;
use tracing_subscriber::EnvFilter;
use wallr_common::cli::{Commands, ConfigCommands, EffectArgs, IpcCommands, WallrCli};
use wallr_common::config::{self, WallrConfig};
use wallr_common::effect::{Effect, FadeParams};
use wallr_core::ipc::{IpcCommand, IpcResponse, send_ipc_command};
use wallr_core::wallpaper::{DiagnosticStatus, WallpaperEngine};

/// Upper bound for `wallr set -` input. It is read before any format
/// detection or staging, so an endless pipe must not grow memory without
/// limit.
const MAX_STDIN_BYTES: u64 = 256 * 1024 * 1024;

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
    let last = parts[parts.len() - 1];
    if last.is_empty() || current.get(last).is_none() {
        anyhow::bail!("unknown config key: {key}");
    }
    current[last] = replacement;
    Ok(())
}

fn ensure_response(response: IpcResponse) -> anyhow::Result<IpcResponse> {
    if response.success {
        Ok(response)
    } else {
        anyhow::bail!(
            "{}",
            response
                .message
                .unwrap_or_else(|| "daemon reported failure".to_string())
        )
    }
}

fn print_cli_message(message: &str) {
    for line in message.trim_end().lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let lower = trimmed.to_ascii_lowercase();

        if lower.starts_with("error")
            || lower.starts_with("failed")
            || lower.starts_with("no ")
            || lower.starts_with("unknown ")
        {
            println!("{}{}", indent, trimmed.red());
        } else if let Some((label, value)) = trimmed.split_once(':') {
            println!("{}{}: {}", indent, label.cyan().bold(), value.trim_start());
        } else {
            println!("{}{}", indent, trimmed.green());
        }
    }
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
        .map(|duration| {
            u32::try_from(duration.as_millis()).map_err(|_| {
                anyhow::anyhow!(
                    "transition duration {}ms exceeds the supported maximum",
                    duration.as_millis()
                )
            })
        })
        .transpose()?;
    Ok((effect, duration_ms))
}

fn detect_image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        // Let the image decoder report the actual format. PNG is the common
        // screenshot format and is also what grim emits.
        "png"
    }
}

fn stage_stdin(bytes: &[u8], extension: &str) -> anyhow::Result<std::path::PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let directory = std::env::temp_dir();
    let mut attempt = 0u32;
    loop {
        let path = directory.join(format!(
            "wallr-stdin-{}-{}-{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            attempt,
            extension
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.flush()?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                attempt = attempt.wrapping_add(1);
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn daemon_command(
    config_path: Option<&std::path::Path>,
    max_fps: Option<u32>,
    verbose: u8,
    quiet: bool,
) -> anyhow::Result<std::process::Command> {
    let daemon = std::env::current_exe()?.with_file_name("wallr-daemon");
    let mut command = std::process::Command::new(daemon);
    if let Some(path) = config_path {
        command.arg("--config").arg(path);
    }
    if let Some(fps) = max_fps {
        command.arg("--max-fps").arg(fps.to_string());
    }
    if verbose > 0 {
        command.arg(format!("-{}", "v".repeat(usize::from(verbose))));
    }
    if quiet {
        command.arg("--quiet");
    }
    Ok(command)
}

fn help_command(command: clap::Command) -> clap::Command {
    command
        .help_template(
            "{before-help}{name}\n{about-with-newline}\nusage:\n  {usage}\n\narguments:\n{positionals}\noptions:\n{options}{after-help}",
        )
        .disable_help_flag(true)
        .mut_args(|arg| {
            if arg.get_id() == "help" {
                arg.help("show help")
            } else {
                arg
            }
        })
        .mut_subcommands(help_command)
}

fn parse_cli() -> WallrCli {
    let matches = help_command(WallrCli::command()).get_matches();
    WallrCli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit())
}

// All decoding, rendering, and filesystem work is dispatched through
// `spawn_blocking`; a current-thread scheduler avoids keeping one idle async
// worker per CPU in the long-lived daemon and keeps the IPC-only CLI light.
fn main() {
    let cli = parse_cli();
    let result = if matches!(
        &cli.command,
        Commands::Config {
            subcommand: ConfigCommands::Path
        }
    ) {
        let path = cli.config.clone().unwrap_or_else(config::config_path);
        println!("{}", path.display().to_string().cyan());
        return;
    } else if let Commands::Daemon { max_fps } = &cli.command {
        daemon_command(cli.config.as_deref(), *max_fps, cli.verbose, cli.quiet)
            .and_then(|mut command| command.status().map_err(Into::into))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    anyhow::bail!("wallr-daemon exited with {status}")
                }
            })
    } else {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(4)
            .build()
        {
            Ok(runtime) => runtime.block_on(async_main(cli)),
            Err(error) => Err(error.into()),
        }
    };
    if let Err(error) = result {
        eprintln!("{} {}", "error:".red().bold(), error);
        std::process::exit(1);
    }
}

async fn async_main(cli: WallrCli) -> Result<()> {
    let filter = if cli.quiet {
        "wallr=error"
    } else if cli.verbose > 1 {
        "wallr=trace"
    } else if cli.verbose == 1 {
        "wallr=debug"
    } else {
        "wallr=off"
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .try_init();

    let config_path_owned = cli.config.clone();
    let config_path = config_path_owned.as_deref();
    let config = match config::load_config(config_path) {
        Ok(config) => config,
        // `wallr config set -c new.yaml` is also how users create a fresh
        // config file. Keep that one command lenient, but report a missing
        // explicit file to every other command instead of silently using
        // defaults with the wrong socket/paths.
        Err(err)
            if config_path.is_some()
                && matches!(
                    &cli.command,
                    Commands::Config {
                        subcommand: ConfigCommands::Set { .. }
                    }
                ) =>
        {
            let _ = err;
            WallrConfig::default()
        }
        Err(err) => return Err(err.into()),
    };

    match cli.command {
        Commands::Set {
            path,
            no_theme,
            theme,
            fps,
            monitor,
            mode,
            effect_args,
        } => {
            let socket_path = config::expand_path(&config.daemon.socket)?;
            let daemon_running = tokio::net::UnixStream::connect(&socket_path).await.is_ok();

            if !daemon_running {
                if !config.daemon.auto_start {
                    anyhow::bail!(
                        "daemon is not running and daemon.auto_start is false; \
                         start it with `wallr daemon`"
                    );
                }
                let _ = daemon_command(config_path_owned.as_deref(), None, 0, false)?
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;

                let mut waited = 0u64;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    waited += 100;
                    if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                        break;
                    }
                    if waited >= 3000 {
                        anyhow::bail!(
                            "daemon did not start within 3 seconds. \
                             run `wallr daemon` manually and check for errors."
                        );
                    }
                }
            }

            let (effect, duration_ms) = pick_effect(&effect_args)?;

            // A `-` path reads image bytes from stdin (like `awww img -`),
            // staging them in a private temp file: downstream code paths key
            // GIF/static decoding off the file extension.
            let (path, staged) = if path.as_os_str() == "-" {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::io::stdin()
                    .take(MAX_STDIN_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                anyhow::ensure!(!bytes.is_empty(), "no image data on stdin");
                anyhow::ensure!(
                    bytes.len() as u64 <= MAX_STDIN_BYTES,
                    "stdin image exceeds the {} MiB limit",
                    MAX_STDIN_BYTES / (1024 * 1024)
                );
                let ext = detect_image_extension(&bytes);
                (stage_stdin(&bytes, ext)?, true)
            } else {
                (path, false)
            };
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let response = send_ipc_command(
                socket_path,
                IpcCommand::Preview {
                    path: canonical_path.to_string_lossy().to_string(),
                    effect: Some(effect),
                    duration_ms,
                    fps,
                    no_theme,
                    theme_override: theme,
                    monitor,
                    scaling_mode: mode.or(Some(config.wallpaper.mode.clone())),
                },
            )
            .await;
            if staged {
                let _ = std::fs::remove_file(&path);
            }
            ensure_response(response?)?;
            println!("{} {}", "✓".green().bold(), "wallpaper applied".green());
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
                    check.name.to_ascii_lowercase().bold(),
                    check.message.dimmed()
                );
            }
            println!();
        }

        Commands::Config { subcommand } => match subcommand {
            ConfigCommands::Path => {
                let path = config_path_owned.unwrap_or_else(config::config_path);
                println!("{}", path.display().to_string().cyan());
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
                println!("{} {}", "updated".green(), path.display());
            }
        },

        Commands::Reload => {
            let socket_path = config::expand_path(&config.daemon.socket)?;
            if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                let response =
                    ensure_response(send_ipc_command(socket_path, IpcCommand::Reload).await?)?;
                println!(
                    "{} {}",
                    "✓".green().bold(),
                    "configuration reloaded".green()
                );
                if let Some(msg) = response.message {
                    print_cli_message(&msg);
                }
            } else {
                let engine = WallpaperEngine::new(config)?;
                tokio::task::spawn_blocking(move || engine.reload_blocking()).await??;
                println!("{} configuration and hooks reloaded", "✓".green());
            }
        }

        Commands::Monitor { subcommand } => {
            use wallr_common::cli::MonitorCommands;
            let socket_path = config::expand_path(&config.daemon.socket)?;
            let response = match subcommand {
                MonitorCommands::List => {
                    send_ipc_command(socket_path, IpcCommand::MonitorList).await?
                }
                MonitorCommands::Current => {
                    send_ipc_command(socket_path, IpcCommand::MonitorCurrent).await?
                }
            };
            let response = ensure_response(response)?;
            if let Some(msg) = response.message {
                print_cli_message(&msg);
            }
        }

        Commands::Quit => {
            let socket_path = config::expand_path(&config.daemon.socket)?;
            if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
                let response =
                    ensure_response(send_ipc_command(socket_path, IpcCommand::Stop).await?)?;
                if let Some(msg) = response.message {
                    print_cli_message(&msg);
                }
            } else {
                anyhow::bail!("daemon is not running");
            }
        }

        Commands::Daemon { max_fps } => {
            let status = daemon_command(
                config_path_owned.as_deref(),
                max_fps,
                cli.verbose,
                cli.quiet,
            )?
            .status()?;
            if !status.success() {
                anyhow::bail!("wallr-daemon exited with {status}");
            }
        }

        Commands::Ipc { subcommand } => {
            let socket_path = config::expand_path(&config.daemon.socket)?;
            let cmd = match subcommand {
                IpcCommands::Pause { monitor } => IpcCommand::Pause { monitor },
                IpcCommands::Resume { monitor } => IpcCommand::Resume { monitor },
                IpcCommands::Reload => IpcCommand::Reload,
                IpcCommands::Stop => IpcCommand::Stop,
                IpcCommands::Status => IpcCommand::Status,
                IpcCommands::Info { monitor } => IpcCommand::Info { monitor },
                IpcCommands::Seek { timestamp, monitor } => {
                    let timestamp_ms = wallr_common::cli::parse_timestamp_ms(&timestamp)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    IpcCommand::Seek {
                        timestamp_ms,
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
            let response = ensure_response(send_ipc_command(socket_path, cmd).await?)?;
            if let Some(msg) = response.message {
                print_cli_message(&msg);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{detect_image_extension, set_config_value};

    #[test]
    fn detects_stdin_formats_from_magic() {
        assert_eq!(detect_image_extension(b"GIF89a...."), "gif");
        assert_eq!(detect_image_extension(b"\x89PNG\r\n\x1a\n...."), "png");
        assert_eq!(detect_image_extension(b"\xff\xd8\xff...."), "jpg");
        assert_eq!(detect_image_extension(b"RIFF1234WEBP...."), "webp");
        assert_eq!(detect_image_extension(b"unknown"), "png");
    }

    #[test]
    fn config_set_rejects_unknown_final_keys() {
        let mut value = serde_yaml::from_str("wallpaper:\n  mode: fill\n").unwrap();
        assert!(set_config_value(&mut value, "wallpaper.nope", serde_yaml::Value::Null).is_err());
        assert!(set_config_value(&mut value, "wallpaper.mode", serde_yaml::Value::Null).is_ok());
    }
}
