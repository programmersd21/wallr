use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WallrConfig {
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    #[serde(default)]
    pub animation: AnimationConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub matugen: MatugenConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub reload: Vec<String>,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub video: VideoConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsConfig {
    #[serde(default)]
    pub matugen: PluginConfig,
    #[serde(default)]
    pub pywal: PluginConfig,
    #[serde(default)]
    pub wallust: PluginConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub mode: ScalingMode,
    #[serde(default)]
    pub monitors: Vec<MonitorConfig>,
    #[serde(default = "default_true")]
    pub loop_video: bool,
    #[serde(default = "default_true")]
    pub mute: bool,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            default: None,
            mode: ScalingMode::Fill,
            monitors: vec![],
            loop_video: true,
            mute: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    #[serde(default = "default_hw_decode")]
    pub hw_decode: String,
    #[serde(default)]
    pub preferred_gpu: crate::video::GpuSelection,
    #[serde(default = "default_preload_frames")]
    pub preload_frames: usize,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            hw_decode: default_hw_decode(),
            preferred_gpu: crate::video::GpuSelection::Auto,
            preload_frames: default_preload_frames(),
        }
    }
}

fn default_hw_decode() -> String {
    "auto".to_string()
}

fn default_preload_frames() -> usize {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub name: String,
    #[serde(default)]
    pub file: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationConfig {
    #[serde(rename = "use", default)]
    pub r#use: Option<String>,
    #[serde(default = "default_duration")]
    pub duration: String,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            r#use: None,
            duration: "2000ms".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default)]
    pub provider: ThemeProvider,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            provider: ThemeProvider::Matugen,
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatugenConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_scheme")]
    pub scheme: String,
    #[serde(default)]
    pub contrast: i32,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for MatugenConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "dark".to_string(),
            scheme: "scheme-tonal-spot".to_string(),
            contrast: 0,
            wait: true,
            args: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub error: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default = "default_socket")]
    pub socket: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            auto_start: true,
            socket: "$XDG_RUNTIME_DIR/wallr.sock".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default = "default_debounce")]
    pub debounce: String,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: None,
            debounce: "500ms".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_cache_dir")]
    pub dir: String,
    #[serde(default = "default_max_size")]
    pub max_size: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            dir: "~/.cache/wallr".to_string(),
            max_size: "512MB".to_string(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_mode() -> String {
    "dark".to_string()
}
fn default_scheme() -> String {
    "scheme-tonal-spot".to_string()
}
fn default_duration() -> String {
    "2000ms".to_string()
}
fn default_socket() -> String {
    "/tmp/wallr.sock".to_string()
}
fn default_debounce() -> String {
    "500ms".to_string()
}
fn default_cache_dir() -> String {
    "~/.cache/wallr".to_string()
}
fn default_max_size() -> String {
    "512MB".to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    ParseError(#[from] serde_yaml::Error),
    #[error("invalid duration format: {0}")]
    InvalidDuration(String),
    #[error("invalid size format: {0}")]
    InvalidSize(String),
    #[error("invalid config value: {0}")]
    InvalidValue(String),
}

pub fn load_config(path: Option<&Path>) -> Result<WallrConfig, ConfigError> {
    let p = path.map(|p| p.to_path_buf()).unwrap_or_else(config_path);
    if !p.exists() {
        return Ok(WallrConfig::default());
    }
    let content = std::fs::read_to_string(p)?;
    let config: WallrConfig = serde_yaml::from_str(&content)?;
    Ok(config)
}

pub fn expand_path(path: &str) -> PathBuf {
    let mut path_str = path.to_string();

    if (path_str.starts_with("~/") || path_str == "~")
        && let Ok(home) = std::env::var("HOME")
    {
        if path_str == "~" {
            path_str = home;
        } else {
            path_str = path_str.replacen("~", &home, 1);
        }
    }

    let mut expanded = String::new();
    let mut chars = path_str.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            let mut env_var = String::new();
            while let Some(&next_c) = chars.peek() {
                if next_c.is_alphanumeric() || next_c == '_' {
                    env_var.push(next_c);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Ok(val) = std::env::var(&env_var) {
                expanded.push_str(&val);
            }
        } else {
            expanded.push(c);
        }
    }

    PathBuf::from(expanded)
}

pub fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("WALLR_CONFIG") {
        return PathBuf::from(path);
    }

    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("wallr/config.yaml");
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/wallr/config.yaml");
    }

    PathBuf::from("/tmp/wallr/config.yaml")
}

pub fn parse_duration(s: &str) -> Result<std::time::Duration, ConfigError> {
    let s = s.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        let val: u64 = ms
            .parse()
            .map_err(|_| ConfigError::InvalidDuration(s.to_string()))?;
        Ok(std::time::Duration::from_millis(val))
    } else if let Some(sec) = s.strip_suffix('s') {
        let val: f64 = sec
            .parse()
            .map_err(|_| ConfigError::InvalidDuration(s.to_string()))?;
        if !val.is_finite() || val < 0.0 {
            return Err(ConfigError::InvalidDuration(s.to_string()));
        }
        Ok(std::time::Duration::from_secs_f64(val))
    } else if let Ok(val) = s.parse::<u64>() {
        Ok(std::time::Duration::from_millis(val))
    } else {
        Err(ConfigError::InvalidDuration(s.to_string()))
    }
}

pub fn parse_size(s: &str) -> Result<u64, ConfigError> {
    let s = s.trim().to_uppercase();
    if let Some(num) = s.strip_suffix("GB") {
        let val: u64 = num
            .parse()
            .map_err(|_| ConfigError::InvalidSize(s.to_string()))?;
        Ok(val * 1024 * 1024 * 1024)
    } else if let Some(num) = s.strip_suffix("MB") {
        let val: u64 = num
            .parse()
            .map_err(|_| ConfigError::InvalidSize(s.to_string()))?;
        Ok(val * 1024 * 1024)
    } else if let Some(num) = s.strip_suffix("KB") {
        let val: u64 = num
            .parse()
            .map_err(|_| ConfigError::InvalidSize(s.to_string()))?;
        Ok(val * 1024)
    } else if let Some(num) = s.strip_suffix('B') {
        let val: u64 = num
            .parse()
            .map_err(|_| ConfigError::InvalidSize(s.to_string()))?;
        Ok(val)
    } else if let Ok(val) = s.parse::<u64>() {
        Ok(val)
    } else {
        Err(ConfigError::InvalidSize(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = WallrConfig::default();
        assert_eq!(cfg.wallpaper.mode, ScalingMode::Fill);
        assert_eq!(cfg.animation.duration, "2000ms");
        assert!(cfg.matugen.enabled);
    }

    #[test]
    fn test_load_config_missing_file() {
        let path = Path::new("/nonexistent/config.yaml");
        let cfg = load_config(Some(path)).unwrap();
        assert_eq!(cfg.wallpaper.mode, ScalingMode::Fill);
    }

    #[test]
    fn test_parse_duration_ms() {
        let dur = parse_duration("500ms").unwrap();
        assert_eq!(dur.as_millis(), 500);
    }

    #[test]
    fn test_parse_duration_s() {
        let dur = parse_duration("2s").unwrap();
        assert_eq!(dur.as_secs(), 2);
    }

    #[test]
    fn test_parse_size_mb() {
        let bytes = parse_size("512MB").unwrap();
        assert_eq!(bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_gb() {
        let bytes = parse_size("2GB").unwrap();
        assert_eq!(bytes, 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_expand_path_tilde() {
        let expanded = expand_path("~/test.jpg");
        assert!(!expanded.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn test_expand_path_env() {
        unsafe {
            std::env::set_var("TEST_VAR", "my_folder");
        }
        let expanded = expand_path("/tmp/$TEST_VAR/file.png");
        assert!(expanded.to_string_lossy().contains("my_folder"));
    }

    #[test]
    fn test_roundtrip_serialize() {
        let cfg = WallrConfig::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let parsed: WallrConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.animation.duration, cfg.animation.duration);
    }
}
