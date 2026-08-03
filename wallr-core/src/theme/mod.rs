use crate::config::{MatugenConfig, ThemeProvider};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

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
    match provider {
        ThemeProvider::Matugen if !matugen_config.enabled => Ok(()),
        ThemeProvider::Matugen => run_matugen(image_path, matugen_config),
        ThemeProvider::Wallust => run_wallust(image_path),
        ThemeProvider::Pywal => run_pywal(image_path),
        ThemeProvider::None => Ok(()),
    }
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
