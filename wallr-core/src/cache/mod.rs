use crate::config::CacheConfig;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug)]
pub struct CacheInfo {
    pub total_files: usize,
    pub total_size: u64,
    pub cache_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("cache directory not accessible: {0}")]
    DirError(String),
}

pub struct CacheManager {
    cache_dir: PathBuf,
    max_size: u64,
}

impl CacheManager {
    pub fn new(config: &CacheConfig) -> Result<Self, CacheError> {
        let cache_dir = crate::config::expand_path(&config.dir);
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)?;
        }
        let max_size = crate::config::parse_size(&config.max_size)
            .map_err(|e| CacheError::DirError(e.to_string()))?;
        Ok(Self {
            cache_dir,
            max_size,
        })
    }

    pub fn cache_image(&self, path: &Path) -> Result<PathBuf, CacheError> {
        let key = Self::cache_key(path)?;
        let ext = path.extension().unwrap_or_default().to_string_lossy();
        let cache_path = self.cache_dir.join(format!("{}.{}", key, ext));

        if !cache_path.exists() {
            // Enforce cache size limit by evicting oldest files first
            if self.max_size > 0 {
                self.evict_if_needed(path)?;
            }
            fs::copy(path, &cache_path)?;
        }

        Ok(cache_path)
    }

    fn evict_if_needed(&self, incoming: &Path) -> Result<(), CacheError> {
        let incoming_size = fs::metadata(incoming).map(|m| m.len()).unwrap_or(0);
        let info = self.info()?;

        if info.total_size + incoming_size <= self.max_size {
            return Ok(());
        }

        // Collect cache files sorted by modification time (oldest first)
        let mut entries: Vec<_> = fs::read_dir(&self.cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
            .filter(|e| e.file_name() != "state.json")
            .collect();

        entries.sort_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        let mut current_size = info.total_size;
        for entry in entries {
            if current_size + incoming_size <= self.max_size {
                break;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            fs::remove_file(entry.path())?;
            current_size = current_size.saturating_sub(size);
        }

        Ok(())
    }

    pub fn clear(&self) -> Result<CacheInfo, CacheError> {
        let mut freed_size = 0;
        let mut count = 0;

        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_file() {
                    freed_size += meta.len();
                    count += 1;
                    fs::remove_file(entry.path())?;
                }
            }
        }

        Ok(CacheInfo {
            total_files: count,
            total_size: freed_size,
            cache_dir: self.cache_dir.clone(),
        })
    }

    pub fn info(&self) -> Result<CacheInfo, CacheError> {
        let mut total_size = 0;
        let mut count = 0;

        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_file() {
                    total_size += meta.len();
                    count += 1;
                }
            }
        }

        Ok(CacheInfo {
            total_files: count,
            total_size,
            cache_dir: self.cache_dir.clone(),
        })
    }

    pub fn cache_key(path: &Path) -> Result<String, CacheError> {
        let meta = fs::metadata(path)?;
        let mtime = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(mtime.to_string().as_bytes());

        Ok(hex::encode(hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wallr_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_cache_key_deterministic() {
        let dir = temp_dir();
        let path = dir.join("test_key.txt");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"test").unwrap();

        let key1 = CacheManager::cache_key(&path).unwrap();
        let key2 = CacheManager::cache_key(&path).unwrap();
        assert_eq!(key1, key2);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_cache_info_empty() {
        let dir = temp_dir().join("empty_cache");
        let config = CacheConfig {
            dir: dir.to_string_lossy().to_string(),
            max_size: "1KB".to_string(),
        };
        let manager = CacheManager::new(&config).unwrap();

        let info = manager.info().unwrap();
        assert_eq!(info.total_files, 0);
        assert_eq!(info.total_size, 0);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_clear_cache() {
        let dir = temp_dir().join("clear_cache");
        let config = CacheConfig {
            dir: dir.to_string_lossy().to_string(),
            max_size: "1KB".to_string(),
        };
        let manager = CacheManager::new(&config).unwrap();

        let file_path = dir.join("test.txt");
        fs::write(&file_path, "test").unwrap();

        let info = manager.clear().unwrap();
        assert_eq!(info.total_files, 1);

        let info_after = manager.info().unwrap();
        assert_eq!(info_after.total_files, 0);

        let _ = fs::remove_dir_all(dir);
    }
}
