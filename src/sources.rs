use crate::config::LOCAL_MODE;
use crate::core::{PackageName, RavenError, Recipe};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

pub struct SourceManager {
    local_path: PathBuf,
    remote_url: String,
}

#[derive(Default)]
pub struct LoadedRecipes {
    pub recipes: HashMap<PackageName, Recipe>,
    /// Recipes that were skipped, with the reason.
    pub warnings: Vec<String>,
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<(), RavenError> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RavenError::Git("'git' is not installed or not in PATH".into())
        } else {
            RavenError::Git(format!("failed to run git: {}", e))
        }
    })?;
    if !output.status.success() {
        return Err(RavenError::Git(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn git_output(args: &[&str], cwd: &Path) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

impl SourceManager {
    pub fn new(local_path: PathBuf, remote_url: String) -> Self {
        Self {
            local_path,
            remote_url,
        }
    }

    pub fn is_local_mode(&self) -> bool {
        self.remote_url == LOCAL_MODE
    }

    pub fn recipes_dir(&self) -> &Path {
        &self.local_path
    }

    /// Clones or fast-forwards the recipe repository. Returns `false` in local mode.
    pub fn sync(&self) -> Result<bool, RavenError> {
        if self.is_local_mode() {
            fs::create_dir_all(&self.local_path)?;
            return Ok(false);
        }

        if self.local_path.join(".git").exists() {
            let dir = Some(self.local_path.as_path());
            let current = git_output(&["remote", "get-url", "origin"], &self.local_path);
            if current.as_deref() != Some(self.remote_url.as_str()) {
                if current.is_some() {
                    git(&["remote", "set-url", "origin", &self.remote_url], dir)?;
                } else {
                    git(&["remote", "add", "origin", &self.remote_url], dir)?;
                }
                // The remote changed: replace local state with the new remote's.
                git(&["fetch", "--depth", "1", "origin"], dir)?;
                git(&["reset", "--hard", "FETCH_HEAD"], dir)?;
                git(&["clean", "-fdx"], dir)?;
            } else {
                git(&["pull", "--ff-only"], dir)?;
            }
            return Ok(true);
        }

        let non_empty = fs::read_dir(&self.local_path)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if non_empty {
            return Err(RavenError::Git(format!(
                "{} exists but is not a git repository. Move it away, or run \
                 'raven config --set-repo {}' to use it as a local recipe directory",
                self.local_path.display(),
                LOCAL_MODE
            )));
        }

        // Clone into a temporary sibling, then rename (no half-cloned state).
        let parent = self
            .local_path
            .parent()
            .ok_or_else(|| RavenError::Git("invalid recipes directory".into()))?;
        fs::create_dir_all(parent)?;
        let tmp = parent.join(".recipes.clone-tmp");
        if tmp.exists() {
            fs::remove_dir_all(&tmp)?;
        }
        let tmp_s = tmp.to_string_lossy().into_owned();
        if let Err(e) = git(
            &["clone", "--depth", "1", "--", &self.remote_url, &tmp_s],
            None,
        ) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
        if self.local_path.exists() {
            fs::remove_dir(&self.local_path)?;
        }
        fs::rename(&tmp, &self.local_path)?;
        Ok(true)
    }

    /// Loads all `*.toml` recipes. Invalid recipes are skipped with a warning
    /// so that one broken file doesn't make the whole repository unusable.
    pub fn load(&self) -> Result<LoadedRecipes, RavenError> {
        let mut out = LoadedRecipes::default();
        if !self.local_path.exists() {
            return Ok(out);
        }
        let mut origin: HashMap<PackageName, PathBuf> = HashMap::new();

        let walker = WalkDir::new(&self.local_path)
            .min_depth(1)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'));

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    out.warnings
                        .push(format!("cannot read recipe directory entry: {}", e));
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(|s| s.to_str()) != Some("toml")
            {
                continue;
            }
            let rel = path.strip_prefix(&self.local_path).unwrap_or(path);
            let recipe = fs::read_to_string(path)
                .map_err(RavenError::from)
                .and_then(|c| toml::from_str::<Recipe>(&c).map_err(RavenError::from))
                .and_then(|r| r.validate().map(|_| r));
            match recipe {
                Ok(r) => {
                    if let Some(first) = origin.get(&r.name) {
                        out.warnings.push(format!(
                            "{}: duplicate recipe for '{}' (already defined in {}), ignored",
                            rel.display(),
                            r.name,
                            first.display()
                        ));
                        continue;
                    }
                    origin.insert(r.name.clone(), rel.to_path_buf());
                    out.recipes.insert(r.name.clone(), r);
                }
                Err(e) => out.warnings.push(format!("{}: {}", rel.display(), e)),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
name = "hello"
version = "2.10"
description = "GNU Hello"
source_url = "https://example.com/hello.tar.gz"
sha256_sum = "31e066137a962676e89f69d1b65382de95a7ef7d914b8cb956f41ea72e0f516b"
build_commands = ["make"]
install_commands = ["make install"]
"#;

    #[test]
    fn loads_valid_and_reports_invalid_recipes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("recipes");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("hello.toml"), VALID).unwrap();
        fs::write(dir.join("sub/dup.toml"), VALID).unwrap();
        fs::write(dir.join("broken.toml"), "name = ").unwrap();
        fs::write(dir.join("badver.toml"), VALID.replace("2.10", "two")).unwrap();
        fs::write(dir.join(".git/ignored.toml"), "garbage").unwrap();
        fs::write(dir.join("README.md"), "not a recipe").unwrap();

        let sm = SourceManager::new(dir, LOCAL_MODE.into());
        let loaded = sm.load().unwrap();
        assert_eq!(loaded.recipes.len(), 1);
        assert_eq!(loaded.recipes[&"hello".into()].version, "2.10");
        assert_eq!(loaded.warnings.len(), 3, "{:?}", loaded.warnings);
    }

    #[test]
    fn missing_directory_is_empty() {
        let sm = SourceManager::new("/nonexistent/raven/recipes".into(), LOCAL_MODE.into());
        assert!(sm.load().unwrap().recipes.is_empty());
    }

    #[test]
    fn local_mode_sync_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let sm = SourceManager::new(tmp.path().join("r"), LOCAL_MODE.into());
        assert!(!sm.sync().unwrap());
        assert!(tmp.path().join("r").is_dir());
    }

    #[test]
    fn refuses_to_clobber_non_git_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("r");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("mine.toml"), VALID).unwrap();
        let sm = SourceManager::new(dir.clone(), "https://example.invalid/x.git".into());
        assert!(sm.sync().is_err());
        assert!(dir.join("mine.toml").exists());
    }

    #[test]
    fn clones_and_pulls_local_git_repository() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream");
        fs::create_dir_all(&upstream).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(&upstream)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {:?}", args);
        };
        run(&["init", "-q"]);
        fs::write(upstream.join("hello.toml"), VALID).unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);

        let url = format!("file://{}", upstream.display());
        let sm = SourceManager::new(tmp.path().join("recipes"), url);
        assert!(sm.sync().unwrap());
        assert_eq!(sm.load().unwrap().recipes.len(), 1);

        fs::write(upstream.join("hello.toml"), VALID.replace("2.10", "2.12")).unwrap();
        run(&["commit", "-qam", "bump"]);
        assert!(sm.sync().unwrap());
        assert_eq!(sm.load().unwrap().recipes[&"hello".into()].version, "2.12");
    }
}
