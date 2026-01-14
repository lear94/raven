use crate::core::RavenError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RavenConfig {
    pub repo_url: String,
}

// Default configuration if file is missing
impl Default for RavenConfig {
    fn default() -> Self {
        Self {
            repo_url: "https://github.com/lear94/raven-recipes.git".to_string(),
        }
    }
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

    // Load config or create default if missing
    pub async fn load(&self) -> Result<RavenConfig, RavenError> {
        if !self.path.exists() {
            let default_config = RavenConfig::default();
            self.save(&default_config).await?;
            return Ok(default_config);
        }

        let content = fs::read_to_string(&self.path).await?;
        toml::from_str(&content).map_err(RavenError::ParseError)
    }

    pub async fn save(&self, config: &RavenConfig) -> Result<(), RavenError> {
        let content = toml::to_string_pretty(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        fs::write(&self.path, content).await?;
        Ok(())
    }
}
