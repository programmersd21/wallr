use crate::video::error::{VideoError, VideoResult};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuSelection {
    #[default]
    Auto,
    Integrated,
    Discrete,
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

#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    pub driver: String,
    pub driver_info: String,
}

impl AdapterInfo {
    pub fn is_integrated(&self) -> bool {
        self.device_type == wgpu::DeviceType::IntegratedGpu
    }

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
    for (i, a) in adapters.iter().enumerate() {
        tracing::info!("  [{}] {}", i, a);
    }
    adapters
}

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

    let selected = match preference {
        GpuSelection::Auto => adapters
            .iter()
            .find(|a| a.is_integrated())
            .or_else(|| adapters.first()),
        GpuSelection::Integrated => adapters.iter().find(|a| a.is_integrated()),
        GpuSelection::Discrete => adapters.iter().find(|a| a.is_discrete()),
        GpuSelection::Named(name) => adapters.iter().find(|a| a.name.contains(name)),
    };

    let selected = selected.ok_or_else(|| {
        VideoError::AdapterNotFound(format!("No adapter matching preference: {}", preference))
    })?;

    tracing::info!("Selected GPU adapter: {}", selected);

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
