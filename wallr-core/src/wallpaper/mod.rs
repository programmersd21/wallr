use crate::config::{ThemeProvider, WallrConfig};
use crate::theme;
use anyhow::Result;
use std::path::Path;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SetOptions {
    pub no_theme: bool,
    /// Per-invocation theme provider override (wins over config.theme.provider).
    pub theme_provider: Option<ThemeProvider>,
    pub monitor: Option<String>,
}

#[derive(Debug, Clone)]
pub enum DiagnosticStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct DiagnosticCheck {
    pub name: String,
    pub status: DiagnosticStatus,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub checks: Vec<DiagnosticCheck>,
}

#[derive(Debug, thiserror::Error)]
pub enum WallpaperError {
    #[error("Theme error: {0}")]
    Theme(#[from] crate::theme::ThemeError),
    #[error("Custom error: {0}")]
    Custom(String),
}

pub struct WallpaperEngine {
    pub config: WallrConfig,
}

impl WallpaperEngine {
    pub fn new(config: WallrConfig) -> Result<Self, WallpaperError> {
        Ok(Self { config })
    }

    pub fn doctor(&self) -> DiagnosticReport {
        let mut checks = Vec::new();

        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            checks.push(DiagnosticCheck {
                name: "Wayland Session".to_string(),
                status: DiagnosticStatus::Pass,
                message: "Wayland display socket detected.".to_string(),
            });
            checks.push(DiagnosticCheck {
                    name: "wlr-layer-shell".to_string(),
                    status: DiagnosticStatus::Warn,
                    message: "Protocol probing occurs when the daemon binds layer-shell; run wallr daemon for a definitive check.".to_string(),
                });
        } else {
            checks.push(DiagnosticCheck {
                name: "Wayland Session".to_string(),
                status: DiagnosticStatus::Fail,
                message: "$WAYLAND_DISPLAY is not set. wallr requires a Wayland environment."
                    .to_string(),
            });
        }

        let provider_binary = match self.config.theme.provider {
            ThemeProvider::Matugen => Some("matugen"),
            ThemeProvider::Wallust => Some("wallust"),
            ThemeProvider::Pywal => Some("wal"),
            ThemeProvider::None => None,
        };

        if let Some(bin) = provider_binary {
            let is_available = theme::check_provider_available(&self.config.theme.provider);
            if is_available {
                checks.push(DiagnosticCheck {
                    name: format!("Theme Provider ({})", bin),
                    status: DiagnosticStatus::Pass,
                    message: format!("Binary '{}' is available on PATH.", bin),
                });
            } else {
                checks.push(DiagnosticCheck {
                    name: format!("Theme Provider ({})", bin),
                    status: DiagnosticStatus::Fail,
                    message: format!("Binary '{}' was not found on PATH.", bin),
                });
            }
        }

        if let Some(warning) = theme::detect_matugen_loop_risk() {
            checks.push(DiagnosticCheck {
                name: "Infinite Loop Risk".to_string(),
                status: DiagnosticStatus::Warn,
                message: warning,
            });
        } else {
            checks.push(DiagnosticCheck {
                name: "Infinite Loop Risk".to_string(),
                status: DiagnosticStatus::Pass,
                message: "No immediate parent loop risks detected.".to_string(),
            });
        }

        DiagnosticReport { checks }
    }

    /// Asynchronous entry point. The engine keeps no Wayland state, so the
    /// blocking hook/theme work is safe to run on a dedicated worker thread.
    pub async fn set_wallpaper(
        &self,
        path: &Path,
        options: &SetOptions,
    ) -> Result<(), WallpaperError> {
        let config = self.config.clone();
        let path = path.to_path_buf();
        let options = options.clone();
        match tokio::task::spawn_blocking(move || {
            let mut engine = WallpaperEngine::new(config)?;
            engine.set_wallpaper_blocking(&path, &options)
        })
        .await
        {
            Ok(result) => result,
            Err(err) => Err(WallpaperError::Custom(format!(
                "wallpaper task failed: {err}"
            ))),
        }
    }

    /// Synchronous implementation for callers that must run it on a blocking
    /// worker. Theme providers and hooks spawn and wait for external
    /// processes; running them directly on a Tokio worker stalls the IPC
    /// loop and every other task on that runtime.
    pub fn set_wallpaper_blocking(
        &mut self,
        path: &Path,
        options: &SetOptions,
    ) -> Result<(), WallpaperError> {
        info!("Setting wallpaper to: {:?}", path);

        let result = (|| {
            theme::run_hooks(&self.config.hooks.before)?;
            self.apply_wallpaper(path)?;
            if !options.no_theme {
                let provider = options
                    .theme_provider
                    .as_ref()
                    .unwrap_or(&self.config.theme.provider);
                if *provider != ThemeProvider::None {
                    theme::dispatch_theme(provider, path, &self.config.matugen)?;
                }
            }
            theme::run_reload_list(&self.config.reload)?;
            theme::run_hooks(&self.config.hooks.after)?;
            Ok::<(), WallpaperError>(())
        })();
        if result.is_err() {
            let _ = theme::run_hooks(&self.config.hooks.error);
        }
        result
    }

    fn apply_wallpaper(&self, path: &Path) -> Result<(), WallpaperError> {
        if !path.is_file() {
            return Err(WallpaperError::Custom(format!(
                "wallpaper file not found: {}",
                path.display()
            )));
        }
        // Rendering happens in the daemon before this engine runs; this step
        // only validates that the source file still exists for theme/hooks.
        Ok(())
    }

    pub fn reload(&self) -> Result<(), WallpaperError> {
        self.reload_blocking()
    }

    /// Synchronous implementation for blocking workers; see
    /// [`WallpaperEngine::set_wallpaper_blocking`].
    pub fn reload_blocking(&self) -> Result<(), WallpaperError> {
        info!("Running reload commands...");
        theme::run_reload_list(&self.config.reload)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_rejects_missing_file() {
        let engine = WallpaperEngine::new(WallrConfig::default()).unwrap();
        let result = engine.apply_wallpaper(Path::new("/definitely/missing/wallpaper.png"));
        assert!(result.is_err());
    }

    #[test]
    fn set_wallpaper_blocking_rejects_missing_file() {
        let mut engine = WallpaperEngine::new(WallrConfig::default()).unwrap();
        let options = SetOptions {
            no_theme: true,
            theme_provider: None,
            monitor: None,
        };
        let result =
            engine.set_wallpaper_blocking(Path::new("/definitely/missing/wallpaper.png"), &options);
        assert!(result.is_err());
    }

    #[test]
    fn set_wallpaper_blocking_accepts_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wall.png");
        std::fs::write(&path, b"not a real image").unwrap();
        let mut engine = WallpaperEngine::new(WallrConfig::default()).unwrap();
        let options = SetOptions {
            no_theme: true,
            theme_provider: None,
            monitor: None,
        };
        assert!(engine.set_wallpaper_blocking(&path, &options).is_ok());
    }

    #[tokio::test]
    async fn set_wallpaper_async_rejects_missing_file() {
        let engine = WallpaperEngine::new(WallrConfig::default()).unwrap();
        let options = SetOptions {
            no_theme: true,
            theme_provider: None,
            monitor: None,
        };
        let result = engine
            .set_wallpaper(Path::new("/definitely/missing/wallpaper.png"), &options)
            .await;
        assert!(result.is_err());
    }
}
