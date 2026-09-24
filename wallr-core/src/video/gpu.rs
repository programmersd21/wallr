use crate::video::error::{VideoError, VideoResult};
use std::fmt;

pub use wallr_common::types::GpuSelection;

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
    let adapters: Vec<_> = instance
        .enumerate_adapters(wgpu::Backends::all())
        .into_iter()
        .map(|adapter| {
            let info = adapter.get_info();
            (
                adapter,
                AdapterInfo {
                    name: info.name,
                    backend: info.backend,
                    device_type: info.device_type,
                    driver: info.driver,
                    driver_info: info.driver_info,
                },
            )
        })
        .collect();
    if adapters.is_empty() {
        return Err(VideoError::AdapterNotFound(
            "No GPU adapters detected".to_string(),
        ));
    }

    let selected = match preference {
        GpuSelection::Auto => adapters
            .iter()
            .find(|(_, info)| info.is_integrated())
            .or_else(|| adapters.first()),
        GpuSelection::Integrated => adapters.iter().find(|(_, info)| info.is_integrated()),
        GpuSelection::Discrete => adapters.iter().find(|(_, info)| info.is_discrete()),
        GpuSelection::Named(name) => {
            let requested = name.to_ascii_lowercase();
            adapters
                .iter()
                .find(|(_, info)| info.name.to_ascii_lowercase().contains(&requested))
        }
    };

    let selected = selected.ok_or_else(|| {
        VideoError::AdapterNotFound(format!("No adapter matching preference: {}", preference))
    })?;

    tracing::info!("Selected GPU adapter: {}", selected.1);
    Ok(selected.0.clone())
}

pub fn adapter_diagnostics(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "GPU: {} ({:?})\nBackend: {:?}\nDriver: {}\nDriver Info: {}",
        info.name, info.device_type, info.backend, info.driver, info.driver_info
    )
}
