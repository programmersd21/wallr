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
    #[error("package validation failed: {0}")]
    Validation(String),
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
        let base = self.load_package(reference)?;
        let extends = base.spec.extends.clone();
        resolve_extends(base.spec, &extends, self)
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

    /// Install a remote package (`username/repo` or `github:username/repo`)
    /// into the local registry at `packages/<repo>/wallr.yaml`.
    pub fn install_package(&self, reference: &str) -> Result<AnimationSpec, PackageError> {
        let (_, repo) = parse_remote_reference(reference)?;
        let content = download_remote_spec(reference)?;
        let raw = crate::animation::parse_animation_yaml(&content)?;
        let extends = raw.extends.clone();
        let spec = resolve_extends(raw, &extends, self)?;
        let install_dir = self.packages_dir.join(&repo);
        fs::create_dir_all(&install_dir)?;
        fs::write(install_dir.join("wallr.yaml"), &content)?;
        Ok(spec)
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
        PackageError::Validation(
            errors
                .into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    Ok(base)
}

/// Parse a remote package reference, accepting either `username/repo` or the
/// `github:username/repo` form used in `extends` lists.
fn parse_remote_reference(reference: &str) -> Result<(String, String), PackageError> {
    let stripped = reference.strip_prefix("github:").unwrap_or(reference);
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|part| part.is_empty() || part.contains('@'))
        || parts.iter().any(|part| *part == "." || *part == "..")
    {
        return Err(PackageError::InvalidReference(reference.to_string()));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Download the raw YAML for a remote package by probing the repository root
/// for `wallr.yaml` or `<repo>.yaml` on the default branches.
fn download_remote_spec(reference: &str) -> Result<String, PackageError> {
    let (owner, repo) = parse_remote_reference(reference)?;
    let candidates = ["wallr.yaml".to_string(), format!("{repo}.yaml")];
    for file_name in &candidates {
        for branch in ["main", "master"] {
            let url =
                format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{file_name}");
            if let Ok(out) = std::process::Command::new("curl")
                .arg("-fsSL")
                .arg(&url)
                .output()
                && out.status.success()
                && !out.stdout.is_empty()
            {
                return Ok(String::from_utf8_lossy(&out.stdout).to_string());
            }
        }
    }
    Err(PackageError::DownloadError(format!(
        "unable to fetch {owner}/{repo} from GitHub"
    )))
}

pub fn fetch_remote_package(reference: &str) -> Result<AnimationSpec, PackageError> {
    let (owner, repo) = parse_remote_reference(reference)?;

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

    let content = download_remote_spec(reference)?;
    let spec = crate::animation::parse_animation_yaml(&content)?;
    crate::animation::validate_animation(&spec).map_err(|errors| {
        PackageError::Validation(
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
            if let Ok(package) = registry.load_package(reference) {
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
    use crate::animation::Effect;

    fn temp_packages_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wallr-packages-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_package(dir: &Path, name: &str, yaml: &str) {
        let pkg_dir = dir.join(name);
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("wallr.yaml"), yaml).unwrap();
    }

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

    #[test]
    fn test_parse_remote_reference() {
        assert_eq!(
            parse_remote_reference("programmersd21/wallr").unwrap(),
            ("programmersd21".to_string(), "wallr".to_string())
        );
        assert_eq!(
            parse_remote_reference("github:programmersd21/wallr").unwrap(),
            ("programmersd21".to_string(), "wallr".to_string())
        );
        for bad in [
            "", "single", "a/b/c", "a/", "/b", "@/repo", "a/b@c", "..", "../repo", ".",
        ] {
            assert!(
                parse_remote_reference(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn test_resolve_animation_applies_extends() {
        let dir = temp_packages_dir("extends");
        write_package(
            &dir,
            "base",
            "name: base\nduration: 500ms\neffects:\n  - fade\n",
        );
        write_package(
            &dir,
            "child",
            "name: child\nextends:\n  - base\neffects:\n  - blur:\n      from: 8\n      to: 0\n",
        );
        let registry = PackageRegistry { packages_dir: dir };

        let spec = registry.resolve_animation("child").unwrap();
        assert_eq!(spec.name, "child");
        assert_eq!(spec.duration.as_deref(), Some("500ms"));
        assert_eq!(spec.effects.len(), 2);
        assert!(matches!(spec.effects[0], Effect::Blur(_)));
        assert!(matches!(spec.effects[1], Effect::Fade(_)));
    }

    #[test]
    fn test_resolve_animation_rejects_extends_cycles() {
        let dir = temp_packages_dir("cycle");
        write_package(
            &dir,
            "a",
            "name: a\nduration: 1s\nextends: [b]\neffects: [fade]\n",
        );
        write_package(
            &dir,
            "b",
            "name: b\nduration: 1s\nextends: [a]\neffects: [fade]\n",
        );
        let registry = PackageRegistry { packages_dir: dir };

        assert!(matches!(
            registry.resolve_animation("a"),
            Err(PackageError::CircularExtends(_))
        ));
    }
}
