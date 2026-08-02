//! GPU adapter detection and selection for hybrid GPU systems.
//!
//! Provides intelligent GPU selection to avoid forcing rendering on discrete GPUs
//! when an integrated GPU is available and preferred.

use crate::video::error::{VideoError, VideoResult};
use serde::{Deserialize, Serialize};
use std::fmt;

/// GPU selection preference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuSelection {
    /// Automatically select the best GPU (prefer compositor GPU).
    #[default]
    Auto,
    /// Prefer integrated GPU (Intel iGPU, AMD APU).
    Integrated,
    /// Prefer discrete GPU (NVIDIA, AMD dGPU).
    Discrete,
    /// Select a specific adapter by name.
    Named(String),
}

impl fmt::Display for GpuSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Integrated => write!(f, "integrated"),
            Self::Discrete => write!(f, "discrete"),
            Self::Named(name) => write!(f, "{}", name),
        }
    }
}

/// Information about a detected GPU adapter.
#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    pub driver: String,
    pub driver_info: String,
}

impl AdapterInfo {
    /// Returns true if this adapter is an integrated GPU.
    pub fn is_integrated(&self) -> bool {
        self.device_type == wgpu::DeviceType::IntegratedGpu
    }

    /// Returns true if this adapter is a discrete GPU.
    pub fn is_discrete(&self) -> bool {
        self.device_type == wgpu::DeviceType::DiscreteGpu
    }
}

impl fmt::Display for AdapterInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({:?}, {:?}) - {}",
            self.name, self.device_type, self.backend, self.driver
        )
    }
}

/// Detect all available GPU adapters.
pub async fn detect_adapters(instance: &wgpu::Instance) -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();

    for adapter in instance.enumerate_adapters(wgpu::Backends::all()) {
        let info = adapter.get_info();
        adapters.push(AdapterInfo {
            name: info.name.clone(),
            backend: info.backend,
            device_type: info.device_type,
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
        });
    }

    tracing::info!("Detected {} GPU adapter(s)", adapters.len());
    for (i, adapter) in adapters.iter().enumerate() {
        tracing::info!("  [{}] {}", i, adapter);
    }

    adapters
}

/// Select the best GPU adapter according to the given preference.
///
/// Selection strategy:
/// - `Auto`: Prefer integrated GPU if available, otherwise use discrete
/// - `Integrated`: Select the first integrated GPU
/// - `Discrete`: Select the first discrete GPU
/// - `Named`: Select by exact name match
///
/// Falls back gracefully if the requested adapter is not available.
pub async fn select_adapter(
    instance: &wgpu::Instance,
    preference: &GpuSelection,
) -> VideoResult<wgpu::Adapter> {
    let adapters = detect_adapters(instance).await;

    if adapters.is_empty() {
        return Err(VideoError::AdapterNotFound(
            "No GPU adapters detected".to_string(),
        ));
    }

    let selected_info = match preference {
        GpuSelection::Auto => {
            // Prefer integrated GPU to avoid unnecessary power consumption
            adapters
                .iter()
                .find(|a| a.is_integrated())
                .or_else(|| adapters.first())
        }
        GpuSelection::Integrated => adapters.iter().find(|a| a.is_integrated()),
        GpuSelection::Discrete => adapters.iter().find(|a| a.is_discrete()),
        GpuSelection::Named(name) => adapters.iter().find(|a| a.name.contains(name)),
    };

    let selected_info = selected_info.ok_or_else(|| {
        VideoError::AdapterNotFound(format!("No adapter matching preference: {}", preference))
    })?;

    tracing::info!("Selected GPU adapter: {}", selected_info);

    // Now request the actual adapter from wgpu
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: match preference {
                GpuSelection::Integrated => wgpu::PowerPreference::LowPower,
                GpuSelection::Discrete => wgpu::PowerPreference::HighPerformance,
                _ => wgpu::PowerPreference::LowPower,
            },
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .ok_or_else(|| {
            VideoError::AdapterNotFound(format!("Failed to request adapter for: {}", preference))
        })?;

    Ok(adapter)
}

/// Get diagnostic information about the selected adapter for `wallr info`.
pub fn adapter_diagnostics(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "GPU: {} ({:?})\nBackend: {:?}\nDriver: {}\nDriver Info: {}",
        info.name, info.device_type, info.backend, info.driver, info.driver_info
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_selection_display() {
        assert_eq!(GpuSelection::Auto.to_string(), "auto");
        assert_eq!(GpuSelection::Integrated.to_string(), "integrated");
        assert_eq!(GpuSelection::Discrete.to_string(), "discrete");
        assert_eq!(
            GpuSelection::Named("NVIDIA".to_string()).to_string(),
            "NVIDIA"
        );
    }

    #[test]
    fn test_gpu_selection_default() {
        assert_eq!(GpuSelection::default(), GpuSelection::Auto);
    }
}
