use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use std::io::IsTerminal;
use tracing_subscriber::EnvFilter;
use wallr_common::cli::WallrDaemonCli;
use wallr_common::config;
use wallr_core::daemon::Daemon;

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

fn main() -> Result<()> {
    let matches = help_command(WallrDaemonCli::command()).get_matches();
    let cli = WallrDaemonCli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .build()?
        .block_on(run(cli))
}

async fn run(cli: WallrDaemonCli) -> Result<()> {
    let config_path = cli.config.clone();
    let mut daemon_config = config::load_config(config_path.as_deref())?;
    if let Some(fps) = cli.max_fps {
        daemon_config.daemon.max_fps = Some(fps);
    }

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
        .with_ansi(std::io::stderr().is_terminal())
        .try_init();

    Daemon::with_config_path(daemon_config, config_path)?
        .start()
        .await?;
    Ok(())
}
