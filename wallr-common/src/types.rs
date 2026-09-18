//! Shared data types: wallpaper scaling, theme providers, GPU preference.
//!
//! These enums travel over IPC and live in config files, so they live in the
//! common crate where both the client and the daemon engine can use them
//! without pulling in GPU or Wayland dependencies.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeProvider {
    Matugen,
    Wallust,
    Pywal,
    #[default]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuSelection {
    #[default]
    Auto,
    Integrated,
    Discrete,
    Named(String),
}

impl std::fmt::Display for GpuSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Integrated => write!(f, "integrated"),
            Self::Discrete => write!(f, "discrete"),
            Self::Named(name) => write!(f, "{}", name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_selection_display() {
        assert_eq!(GpuSelection::Auto.to_string(), "auto");
        assert_eq!(GpuSelection::Integrated.to_string(), "integrated");
        assert_eq!(GpuSelection::Discrete.to_string(), "discrete");
        assert_eq!(
            GpuSelection::Named("NVIDIA".to_string()).to_string(),
            "NVIDIA"
        );
    }

    #[test]
    fn gpu_selection_default() {
        assert_eq!(GpuSelection::default(), GpuSelection::Auto);
    }
}
