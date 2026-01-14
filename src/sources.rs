use crate::core::{PackageName, RavenError, Recipe};
use git2::Repository;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use walkdir::WalkDir;

pub struct SourceManager {
    local_path: PathBuf,
    remote_url: String,
}

impl SourceManager {
    pub fn new(local_path: PathBuf, remote_url: String) -> Self {
        Self {
            local_path,
            remote_url,
        }
    }

    pub fn sync(&self) -> Result<(), RavenError> {
        if !self.local_path.exists() {
            Repository::clone(&self.remote_url, &self.local_path)?;
        } else {
            let status = Command::new("git")
                .current_dir(&self.local_path)
                .arg("pull")
                .status()
                .map_err(|e| RavenError::IoError(e))?;

            if !status.success() {
                eprintln!("Warning: Failed to update recipes (offline mode?)");
            }
        }
        Ok(())
    }

    pub fn load(&self) -> Result<HashMap<PackageName, Recipe>, RavenError> {
        let mut recipes = HashMap::new();

        for entry in WalkDir::new(&self.local_path)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("toml") {
                let content = std::fs::read_to_string(entry.path())?;

                let recipe: Recipe = toml::from_str(&content).map_err(RavenError::ParseError)?;

                if let Err(e) = semver::Version::parse(&recipe.version) {
                    return Err(RavenError::DependencyError(format!(
                        "Invalid version in {}: {}",
                        recipe.name.0, e
                    )));
                }

                recipes.insert(recipe.name.clone(), recipe);
            }
        }

        Ok(recipes)
    }
}
