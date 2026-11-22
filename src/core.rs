use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageName(pub String);

impl PackageName {
    /// Package names are used in filesystem paths and shell scripts, so they are
    /// restricted to a safe character set: `[A-Za-z0-9][A-Za-z0-9._+-]*`.
    pub fn is_valid(s: &str) -> bool {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphanumeric() => {}
            _ => return false,
        }
        s.len() <= 128 && chars.all(is_name_char)
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PackageName {
    fn from(s: &str) -> Self {
        PackageName(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HashSum(pub String);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Recipe {
    pub name: PackageName,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub target_arch: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub source_url: String,
    pub sha256_sum: HashSum,
    #[serde(default)]
    pub build_commands: Vec<String>,
    #[serde(default)]
    pub install_commands: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DependencyReq {
    pub name: PackageName,
    pub req: VersionReq,
}

impl Recipe {
    pub fn parsed_version(&self) -> Result<Version, RavenError> {
        parse_version(&self.version)
    }

    pub fn parse_dependencies(&self) -> Result<Vec<DependencyReq>, RavenError> {
        self.dependencies
            .iter()
            .map(|d| {
                parse_dependency(d).map_err(|e| {
                    RavenError::InvalidRecipe(format!("{}: dependency '{}': {}", self.name, d, e))
                })
            })
            .collect()
    }

    /// Validates every field that could otherwise cause a failure late in the
    /// pipeline (after downloading or building).
    pub fn validate(&self) -> Result<(), RavenError> {
        if !PackageName::is_valid(&self.name.0) {
            return Err(RavenError::InvalidRecipe(format!(
                "invalid package name '{}' (allowed: letters, digits, '.', '_', '+', '-')",
                self.name
            )));
        }
        self.parsed_version()
            .map_err(|e| RavenError::InvalidRecipe(format!("{}: {}", self.name, e)))?;
        let sum = self.sha256_sum.0.trim();
        if sum.len() != 64 || !sum.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(RavenError::InvalidRecipe(format!(
                "{}: sha256_sum must be 64 hexadecimal characters",
                self.name
            )));
        }
        if self.source_url.trim().is_empty() {
            return Err(RavenError::InvalidRecipe(format!(
                "{}: source_url is empty",
                self.name
            )));
        }
        for dep in self.parse_dependencies()? {
            if dep.name == self.name {
                return Err(RavenError::InvalidRecipe(format!(
                    "{}: package depends on itself",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

/// Parses a version string. Strict SemVer is preferred, but common upstream
/// formats such as `2.10` or `v1` are accepted and padded to `2.10.0` / `1.0.0`.
pub fn parse_version(s: &str) -> Result<Version, RavenError> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    if let Ok(v) = Version::parse(s) {
        return Ok(v);
    }
    let (core, rest) = match s.find(['-', '+']) {
        Some(i) => s.split_at(i),
        None => (s, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if (1..=2).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        let mut padded = parts.join(".");
        for _ in parts.len()..3 {
            padded.push_str(".0");
        }
        padded.push_str(rest);
        if let Ok(v) = Version::parse(&padded) {
            return Ok(v);
        }
    }
    Err(RavenError::Version(format!(
        "'{}' is not a valid version (expected MAJOR.MINOR.PATCH)",
        s
    )))
}

/// Parses a dependency spec: `name`, `name >= 1.0.0`, `name>=1.0`, `name >=1.0, <2.0`.
pub fn parse_dependency(spec: &str) -> Result<DependencyReq, RavenError> {
    let spec = spec.trim();
    let end = spec.find(|c: char| !is_name_char(c)).unwrap_or(spec.len());
    let (name, rest) = spec.split_at(end);
    if !PackageName::is_valid(name) {
        return Err(RavenError::Version(format!(
            "invalid package name in dependency '{}'",
            spec
        )));
    }
    let rest = rest.trim();
    let req_str = if rest.is_empty() { "*" } else { rest };
    let req = VersionReq::parse(req_str)
        .map_err(|e| RavenError::Version(format!("invalid requirement '{}': {}", req_str, e)))?;
    Ok(DependencyReq {
        name: PackageName(name.to_string()),
        req,
    })
}

#[derive(Error, Debug)]
pub enum RavenError {
    #[error("Checksum verification failed for {file}: expected {expected}, got {actual} (file may be corrupted or tampered)")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("Dependency resolution failed: {0}")]
    DependencyError(String),

    #[error("Package '{0}' is not installed")]
    NotInstalled(String),

    #[error("File conflict: {0}")]
    FileConflict(String),

    #[error("Build of '{package}' failed. Full log: {}\n--- last lines of build log ---\n{tail}", log.display())]
    BuildFailed {
        package: String,
        log: PathBuf,
        tail: String,
    },

    #[error("Invalid recipe: {0}")]
    InvalidRecipe(String),

    #[error("Unsupported: {0}")]
    Unsupported(String),

    #[error("Download failed: {0}")]
    Download(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Version error: {0}")]
    Version(String),

    #[error("Operation interrupted by user")]
    Interrupted,

    #[error("File system error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Database failure: {0}")]
    DbError(#[from] sqlx::Error),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Recipe parsing error: {0}")]
    ParseError(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(name: &str, version: &str, deps: &[&str]) -> Recipe {
        Recipe {
            name: name.into(),
            version: version.into(),
            description: String::new(),
            target_arch: None,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            source_url: "https://example.com/x.tar.gz".into(),
            sha256_sum: HashSum("a".repeat(64)),
            build_commands: vec![],
            install_commands: vec![],
        }
    }

    #[test]
    fn version_parsing_is_lenient() {
        assert_eq!(parse_version("1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_version("2.10").unwrap(), Version::new(2, 10, 0));
        assert_eq!(parse_version("v3").unwrap(), Version::new(3, 0, 0));
        assert_eq!(parse_version(" 1.0-rc1 ").unwrap().pre.as_str(), "rc1");
        assert!(parse_version("abc").is_err());
        assert!(parse_version("1..2").is_err());
        assert!(parse_version("").is_err());
    }

    #[test]
    fn dependency_parsing() {
        let d = parse_dependency("openssl >= 3.0.0").unwrap();
        assert_eq!(d.name.0, "openssl");
        assert!(d.req.matches(&Version::new(3, 1, 0)));
        assert!(!d.req.matches(&Version::new(2, 9, 0)));

        let d = parse_dependency("libfoo>=1.0, <2.0").unwrap();
        assert_eq!(d.name.0, "libfoo");
        assert!(d.req.matches(&Version::new(1, 5, 0)));
        assert!(!d.req.matches(&Version::new(2, 0, 0)));

        let d = parse_dependency("zlib").unwrap();
        assert!(d.req.matches(&Version::new(0, 0, 1)));

        assert!(parse_dependency("").is_err());
        assert!(parse_dependency(">= 1.0").is_err());
        assert!(parse_dependency("foo >= banana").is_err());
    }

    #[test]
    fn name_validation() {
        assert!(PackageName::is_valid("gcc"));
        assert!(PackageName::is_valid("libstdc++"));
        assert!(PackageName::is_valid("python3.12"));
        assert!(!PackageName::is_valid("../etc"));
        assert!(!PackageName::is_valid("-rf"));
        assert!(!PackageName::is_valid("a b"));
        assert!(!PackageName::is_valid(""));
    }

    #[test]
    fn recipe_validation() {
        assert!(recipe("ok", "1.0", &["dep >= 1"]).validate().is_ok());
        assert!(recipe("bad/name", "1.0", &[]).validate().is_err());
        assert!(recipe("ok", "x.y", &[]).validate().is_err());
        assert!(recipe("ok", "1.0", &["ok"]).validate().is_err());
        let mut r = recipe("ok", "1.0", &[]);
        r.sha256_sum = HashSum("123".into());
        assert!(r.validate().is_err());
    }
}
