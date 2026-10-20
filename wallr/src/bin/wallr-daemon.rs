use anyhow::Result;
use clap::Parser;
use wallr_common::cli::{Commands, WallrCli};
use wallr_common::config;
use wallr_core::daemon::Daemon;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = WallrCli::parse();
    let config_path = cli.config.clone();
    let mut daemon_config = config::load_config(config_path.as_deref())?;
    let max_fps = match cli.command {
        Commands::Daemon { max_fps } => max_fps,
        _ => anyhow::bail!("wallr-daemon only accepts the daemon command"),
    };
    if let Some(fps) = max_fps {
        daemon_config.daemon.max_fps = Some(fps);
    }
    Daemon::with_config_path(daemon_config, config_path)?
        .start()
        .await?;
    Ok(())
}
