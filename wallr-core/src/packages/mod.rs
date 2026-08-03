use crate::animation::AnimationSpec;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("package not found: {0}")]
    NotFound(String),
    #[error("failed to read package: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("failed to parse package: {0}")]
    ParseError(#[from] serde_yaml::Error),
    #[error("circular extends detected: {0}")]
    CircularExtends(String),
    #[error("invalid package reference: {0}")]
    InvalidReference(String),
    #[error("animation error: {0}")]
    AnimationError(#[from] crate::animation::AnimationError),
    #[error("Failed to download remote package: {0}")]
    DownloadError(String),
}

pub struct Package {
    pub name: String,
    pub path: PathBuf,
    pub spec: AnimationSpec,
}

pub struct PackageRegistry {
    pub packages_dir: PathBuf,
}

impl PackageRegistry {
    pub fn new() -> Result<Self, PackageError> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let packages_dir = PathBuf::from(home).join(".local/share/wallr/packages");
        if !packages_dir.exists() {
            fs::create_dir_all(&packages_dir)?;
        }
        Ok(Self { packages_dir })
    }

    pub fn load_package(&self, name: &str) -> Result<Package, PackageError> {
        let pkg_path = self.packages_dir.join(name);
        if !pkg_path.exists() {
            return Err(PackageError::NotFound(name.to_string()));
        }

        let yaml_path = ["wallr.yaml", "animation.yaml"]
            .iter()
            .map(|file| pkg_path.join(file))
            .find(|path| path.is_file())
            .ok_or_else(|| PackageError::NotFound(format!("{name} (missing wallr.yaml)")))?;
        let content = fs::read_to_string(&yaml_path)?;
        let spec = crate::animation::parse_animation_yaml(&content)?;

        Ok(Package {
            name: name.to_string(),
            path: pkg_path,
            spec,
        })
    }

    pub fn resolve_animation(&self, reference: &str) -> Result<AnimationSpec, PackageError> {
        let parts: Vec<&str> = reference.split('/').collect();
        if parts.len() != 2 {
            return Err(PackageError::InvalidReference(reference.to_string()));
        }

        let pkg_name = parts[0];
        let package = self.load_package(pkg_name)?;

        Ok(package.spec)
    }

    pub fn list_packages(&self) -> Result<Vec<String>, PackageError> {
        let mut packages = Vec::new();
        if !self.packages_dir.exists() {
            return Ok(packages);
        }

        for entry in fs::read_dir(&self.packages_dir)? {
            let entry = entry?;
            if entry.metadata()?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                packages.push(name.to_string());
            }
        }
        Ok(packages)
    }
}

pub fn resolve_extends(
    mut base: AnimationSpec,
    extends: &[String],
    registry: &PackageRegistry,
) -> Result<AnimationSpec, PackageError> {
    detect_cycles(extends, registry)?;

    for parent_ref in extends {
        let parent_spec = if parent_ref.starts_with("github:") {
            fetch_remote_package(parent_ref)?
        } else {
            registry.resolve_animation(parent_ref)?
        };

        if base.duration.is_none() {
            base.duration = parent_spec.duration;
        }
        if base.timeline.is_none() {
            base.timeline = parent_spec.timeline;
        }
        for (key, value) in parent_spec.variables {
            base.variables.entry(key).or_insert(value);
        }
        for (key, value) in parent_spec.custom_effects {
            base.custom_effects.entry(key).or_insert(value);
        }
        for effect in parent_spec.effects {
            if !base.effects.contains(&effect) {
                base.effects.push(effect);
            }
        }
    }
    crate::animation::validate_animation(&base).map_err(|errors| {
        PackageError::DownloadError(
            errors
                .into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    Ok(base)
}

pub fn fetch_remote_package(reference: &str) -> Result<AnimationSpec, PackageError> {
    let stripped = reference.strip_prefix("github:").unwrap_or(reference);
    let subparts: Vec<&str> = stripped.split('/').collect();
    if subparts.len() != 2
        || subparts
            .iter()
            .any(|part| part.is_empty() || part.contains('@'))
    {
        return Err(PackageError::InvalidReference(reference.to_string()));
    }

    let owner = subparts[0];
    let repo = subparts[1];
    let tag = "main";

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let cache_dir = PathBuf::from(home).join(".cache/wallr/packages");
    if !cache_dir.exists() {
        let _ = fs::create_dir_all(&cache_dir);
    }

    let cache_file = cache_dir.join(format!("{}_{}.yaml", owner, repo));

    if cache_file.exists() {
        let cached_content = fs::read_to_string(&cache_file)?;
        let spec = crate::animation::parse_animation_yaml(&cached_content)?;
        return Ok(spec);
    }

    let candidates = vec!["wallr.yaml".to_string(), format!("{repo}.yaml")];
    let mut output = None;
    for file_path in candidates {
        let url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{tag}/{file_path}");
        let candidate = std::process::Command::new("curl")
            .arg("-fsSL")
            .arg(&url)
            .output();
        if let Ok(out) = candidate
            && out.status.success()
            && !out.stdout.is_empty()
        {
            output = Some(out);
            break;
        }
    }

    match output {
        Some(out) => {
            let content = String::from_utf8_lossy(&out.stdout).to_string();
            let spec = crate::animation::parse_animation_yaml(&content)?;
            crate::animation::validate_animation(&spec).map_err(|errors| {
                PackageError::DownloadError(
                    errors
                        .into_iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
            let _ = fs::write(&cache_file, &content);
            Ok(spec)
        }
        None => Err(PackageError::DownloadError(format!(
            "unable to fetch {owner}/{repo} from GitHub"
        ))),
    }
}

pub fn detect_cycles(extends: &[String], _registry: &PackageRegistry) -> Result<(), PackageError> {
    let mut visited = HashSet::new();
    let mut active = HashSet::new();
    if extends.len() != extends.iter().collect::<HashSet<_>>().len() {
        let duplicate = extends
            .iter()
            .find(|reference| {
                extends
                    .iter()
                    .filter(|candidate| *candidate == *reference)
                    .count()
                    > 1
            })
            .cloned()
            .unwrap_or_default();
        return Err(PackageError::CircularExtends(duplicate));
    }
    fn visit(
        reference: &str,
        registry: &PackageRegistry,
        visited: &mut HashSet<String>,
        active: &mut HashSet<String>,
    ) -> Result<(), PackageError> {
        if active.contains(reference) {
            return Err(PackageError::CircularExtends(reference.to_string()));
        }
        if !visited.insert(reference.to_string()) {
            return Ok(());
        }
        active.insert(reference.to_string());
        if !reference.starts_with("github:") {
            let package_name = reference.split('/').next().unwrap_or(reference);
            if let Ok(package) = registry.load_package(package_name) {
                for parent in &package.spec.extends {
                    visit(parent, registry, visited, active)?;
                }
            }
        }
        active.remove(reference);
        Ok(())
    }
    for ext in extends {
        visit(ext, _registry, &mut visited, &mut active)?;
    }
    Ok(())
}

pub fn load_local_animation(path: &Path) -> Result<AnimationSpec, PackageError> {
    let content = fs::read_to_string(path)?;
    let spec = crate::animation::parse_animation_yaml(&content)?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_no_cycles() {
        let registry = PackageRegistry {
            packages_dir: PathBuf::from("/tmp"),
        };
        assert!(detect_cycles(&["base".to_string(), "common".to_string()], &registry).is_ok());
    }

    #[test]
    fn test_detect_cycles() {
        let registry = PackageRegistry {
            packages_dir: PathBuf::from("/tmp"),
        };
        assert!(detect_cycles(&["base".to_string(), "base".to_string()], &registry).is_err());
    }

    #[test]
    fn test_list_packages_empty() {
        let registry = PackageRegistry {
            packages_dir: PathBuf::from("/does/not/exist"),
        };
        let pkgs = registry.list_packages().unwrap();
        assert!(pkgs.is_empty());
    }
}
