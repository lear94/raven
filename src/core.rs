use semver::VersionReq;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct PackageName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashSum(pub String);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Recipe {
    pub name: PackageName,
    pub version: String,
    pub description: String,
    pub target_arch: Option<String>,
    pub dependencies: Vec<String>,
    pub source_url: String,
    pub sha256_sum: HashSum,
    pub build_commands: Vec<String>,
    pub install_commands: Vec<String>,
}

pub struct DependencyReq {
    pub name: PackageName,
    pub req: VersionReq,
}

impl Recipe {
    pub fn parse_dependencies(&self) -> Result<Vec<DependencyReq>, RavenError> {
        let mut parsed = Vec::new();

        for dep_str in &self.dependencies {
            let parts: Vec<&str> = dep_str.splitn(2, ' ').collect();
            let name = parts[0];
            let req_str = if parts.len() > 1 { parts[1] } else { "*" };

            let req = VersionReq::parse(req_str).map_err(|e| {
                RavenError::DependencyError(format!("Invalid requirement for {}: {}", name, e))
            })?;

            parsed.push(DependencyReq {
                name: PackageName(name.to_string()),
                req,
            });
        }
        Ok(parsed)
    }
}

#[derive(Error, Debug)]
pub enum RavenError {
    #[error("Checksum verification failed (Files may be corrupted or tampered)")]
    HashMismatch,

    #[error("Dependency resolution failed: {0}")]
    DependencyError(String),

    #[error("File system error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Database failure: {0}")]
    DbError(#[from] sqlx::Error),

    #[error("Network connection failed")]
    NetworkError(#[from] reqwest::Error),

    #[error("Git repository error: {0}")]
    GitError(#[from] git2::Error),

    #[error("Recipe parsing error: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Version Error: {0}")]
    VersionError(#[from] semver::Error),
}

pub struct TransactionManager {
    pub db: SqlitePool,
    pub staging_root: PathBuf,
}

impl TransactionManager {
    pub async fn new(db_url: &str, staging_root: PathBuf) -> Result<Self, RavenError> {
        let db = SqlitePool::connect(db_url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS packages (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                hash TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS package_files (
                package_name TEXT NOT NULL,
                filepath TEXT NOT NULL,
                PRIMARY KEY (package_name, filepath)
            );
            CREATE TABLE IF NOT EXISTS dependencies (
                package TEXT NOT NULL,
                depends_on TEXT NOT NULL,
                PRIMARY KEY (package, depends_on)
            );",
        )
        .execute(&db)
        .await?;

        if !staging_root.exists() {
            tokio::fs::create_dir_all(&staging_root).await?;
        }

        Ok(Self { db, staging_root })
    }

    // NEW: Retrieve installed packages for upgrade checks
    pub async fn list_installed(&self) -> Result<Vec<(PackageName, String)>, RavenError> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, version FROM packages")
            .fetch_all(&self.db)
            .await?;

        let packages = rows.into_iter().map(|(n, v)| (PackageName(n), v)).collect();

        Ok(packages)
    }

    pub async fn install_package(
        &self,
        recipe: &Recipe,
        artifact_path: &Path,
    ) -> Result<(), RavenError> {
        let mut tx = self.db.begin().await?;

        let pkg_stage = self
            .staging_root
            .join(format!("{}_{}", recipe.name.0, recipe.version));
        if pkg_stage.exists() {
            tokio::fs::remove_dir_all(&pkg_stage).await?;
        }
        tokio::fs::create_dir_all(&pkg_stage).await?;

        let status = std::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", artifact_path.display()))
            .arg(format!("{}/", pkg_stage.display()))
            .status()?;

        if !status.success() {
            return Err(RavenError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to copy artifacts to staging",
            )));
        }

        for entry in walkdir::WalkDir::new(&pkg_stage)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let src = entry.path();
                let relative = src.strip_prefix(&pkg_stage).unwrap();
                let dest = Path::new("/").join(relative);

                if let Some(parent) = dest.parent() {
                    if !parent.exists() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                }

                if dest.exists() {
                    let _ = tokio::fs::remove_file(&dest).await;
                }

                match tokio::fs::rename(&src, &dest).await {
                    Ok(_) => {}
                    Err(_) => {
                        tokio::fs::copy(&src, &dest).await?;
                        let _ = tokio::fs::remove_file(&src).await;
                    }
                }

                sqlx::query(
                    "INSERT OR REPLACE INTO package_files (package_name, filepath) VALUES (?, ?)",
                )
                .bind(&recipe.name.0)
                .bind(dest.to_string_lossy().to_string())
                .execute(&mut *tx)
                .await?;
            }
        }

        for dep in &recipe.dependencies {
            let parts: Vec<&str> = dep.split_whitespace().collect();
            let dep_name = parts[0];
            sqlx::query("INSERT OR IGNORE INTO dependencies (package, depends_on) VALUES (?, ?)")
                .bind(&recipe.name.0)
                .bind(dep_name)
                .execute(&mut *tx)
                .await?;
        }

        sqlx::query("INSERT OR REPLACE INTO packages (name, version, hash) VALUES (?, ?, ?)")
            .bind(&recipe.name.0)
            .bind(&recipe.version)
            .bind(&recipe.sha256_sum.0)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        if pkg_stage.exists() {
            let _ = tokio::fs::remove_dir_all(pkg_stage).await;
        }

        Ok(())
    }

    pub async fn remove_package(&self, pkg_name: &PackageName) -> Result<(), RavenError> {
        let mut tx = self.db.begin().await?;

        let deps: Vec<(String,)> =
            sqlx::query_as("SELECT package FROM dependencies WHERE depends_on = ?")
                .bind(&pkg_name.0)
                .fetch_all(&mut *tx)
                .await?;

        if !deps.is_empty() {
            return Err(RavenError::DependencyError(format!(
                "Cannot remove '{}', it is required by: {:?}",
                pkg_name.0, deps
            )));
        }

        let files: Vec<(String,)> =
            sqlx::query_as("SELECT filepath FROM package_files WHERE package_name = ?")
                .bind(&pkg_name.0)
                .fetch_all(&mut *tx)
                .await?;

        for (f,) in files {
            let path = Path::new(&f);
            if path.exists() {
                let _ = tokio::fs::remove_file(path).await;
            }
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }

        sqlx::query("DELETE FROM package_files WHERE package_name = ?")
            .bind(&pkg_name.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM dependencies WHERE package = ?")
            .bind(&pkg_name.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM packages WHERE name = ?")
            .bind(&pkg_name.0)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}
