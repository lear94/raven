use crate::core::RavenError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Special repository value: recipes are managed by hand in the recipes directory.
pub const LOCAL_MODE: &str = "local-mode";

fn default_repo() -> String {
    "https://github.com/lear94/raven-recipes.git".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RavenConfig {
    #[serde(default = "default_repo")]
    pub repo_url: String,
}

impl Default for RavenConfig {
    fn default() -> Self {
        Self {
            repo_url: default_repo(),
        }
    }
}

pub fn validate_repo_url(url: &str) -> Result<(), RavenError> {
    if url.trim().is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(RavenError::Config(format!(
            "invalid repository '{}': must be a git URL, a local path or '{}'",
            url, LOCAL_MODE
        )));
    }
    if url.starts_with('-') {
        return Err(RavenError::Config(format!(
            "invalid repository '{}': must not start with '-'",
            url
        )));
    }
    Ok(())
}

pub struct ConfigManager {
    path: PathBuf,
}

impl ConfigManager {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("config.toml"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the configuration, falling back to defaults if the file is missing.
    pub fn load(&self) -> Result<RavenConfig, RavenError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RavenConfig::default()),
            Err(e) => return Err(e.into()),
        };
        let config: RavenConfig = toml::from_str(&content)
            .map_err(|e| RavenError::Config(format!("{}: {}", self.path.display(), e)))?;
        validate_repo_url(&config.repo_url)?;
        Ok(config)
    }

    /// Saves atomically (write to a temporary file, then rename).
    pub fn save(&self, config: &RavenConfig) -> Result<(), RavenError> {
        validate_repo_url(&config.repo_url)?;
        let content =
            toml::to_string_pretty(config).map_err(|e| RavenError::Config(e.to_string()))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, content)?;
        fs::File::open(&tmp)?.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing_and_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let cm = ConfigManager::new(tmp.path());
        assert_eq!(cm.load().unwrap(), RavenConfig::default());
        assert!(!cm.path().exists(), "loading must not create files");

        let cfg = RavenConfig {
            repo_url: "https://gitlab.com/test/repo.git".into(),
        };
        cm.save(&cfg).unwrap();
        assert_eq!(cm.load().unwrap(), cfg);
    }

    #[test]
    fn rejects_invalid_files_and_urls() {
        let tmp = tempfile::tempdir().unwrap();
        let cm = ConfigManager::new(tmp.path());
        fs::write(cm.path(), "repo_url = [").unwrap();
        assert!(cm.load().is_err());
        assert!(validate_repo_url("").is_err());
        assert!(validate_repo_url("has space").is_err());
        assert!(validate_repo_url("--upload-pack=x").is_err());
        assert!(validate_repo_url(LOCAL_MODE).is_ok());
        assert!(validate_repo_url("/srv/recipes").is_ok());
    }
}
