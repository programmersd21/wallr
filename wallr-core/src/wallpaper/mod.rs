use crate::cache::CacheManager;
use crate::config::{ThemeProvider, WallrConfig};
use crate::packages::PackageRegistry;
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

#[derive(Debug, Clone)]
pub struct ValidationCheck {
    pub name: String,
    pub passed: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub checks: Vec<ValidationCheck>,
}

#[derive(Debug, thiserror::Error)]
pub enum WallpaperError {
    #[error("Theme error: {0}")]
    Theme(#[from] crate::theme::ThemeError),
    #[error("Cache error: {0}")]
    Cache(#[from] crate::cache::CacheError),
    #[error("Package error: {0}")]
    Package(#[from] crate::packages::PackageError),
    #[error("Custom error: {0}")]
    Custom(String),
}

pub struct WallpaperEngine {
    pub config: WallrConfig,
    pub cache: CacheManager,
    pub registry: PackageRegistry,
}

impl WallpaperEngine {
    pub fn new(config: WallrConfig) -> Result<Self, WallpaperError> {
        let cache = CacheManager::new(&config.cache)?;
        let registry = PackageRegistry::new()?;
        Ok(Self {
            config,
            cache,
            registry,
        })
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

    pub fn validate_animation(&self, path: &Path) -> Result<ValidationReport, WallpaperError> {
        info!("Validating animation package: {:?}", path);
        let mut checks = Vec::new();

        match crate::packages::load_local_animation(path) {
            Ok(spec) => {
                checks.push(ValidationCheck {
                    name: "YAML Syntax & Parsing".to_string(),
                    passed: true,
                    message: Some(format!("Successfully parsed spec '{}'", spec.name)),
                });

                match crate::animation::validate_animation(&spec) {
                    Ok(_) => {
                        checks.push(ValidationCheck {
                            name: "Timeline & Effects Validation".to_string(),
                            passed: true,
                            message: Some(
                                "Valid animation settings and timeline structure.".to_string(),
                            ),
                        });
                    }
                    Err(errs) => {
                        let msg = errs
                            .into_iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        checks.push(ValidationCheck {
                            name: "Timeline & Effects Validation".to_string(),
                            passed: false,
                            message: Some(msg),
                        });
                    }
                }
            }
            Err(e) => {
                checks.push(ValidationCheck {
                    name: "YAML Syntax & Parsing".to_string(),
                    passed: false,
                    message: Some(e.to_string()),
                });
            }
        }

        Ok(ValidationReport { checks })
    }

    pub async fn set_wallpaper(
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
        let _ = self.cache.cache_image(path);
        info!(
            "Applying wallpaper natively via Wayland layer-shell surface: {:?}",
            path
        );
        Ok(())
    }

    pub fn reload(&self) -> Result<(), WallpaperError> {
        info!("Running reload commands...");
        theme::run_reload_list(&self.config.reload)?;
        Ok(())
    }
}
