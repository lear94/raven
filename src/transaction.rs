//! Transactional installation and removal of package files, backed by SQLite.
//!
//! Installation runs in phases so that a failure at any point leaves the system
//! in its previous state:
//!
//! 1. **Scan & preflight**: inspect the build artifact and detect file conflicts
//!    with other packages before touching the filesystem.
//! 2. **Stage**: copy every file next to its destination under a temporary name
//!    (same filesystem, so the final step is an atomic `rename`).
//! 3. **Swap**: move existing files aside as backups and rename staged files
//!    into place.
//! 4. **Commit**: record files, dependencies and version in one DB transaction.
//! 5. **Cleanup**: delete backups and files left over from the previous version.
//!
//! If anything fails during phases 2-4, every change is rolled back.

use crate::core::{PackageName, RavenError, Recipe};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: PackageName,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct InstalledDependency {
    pub package: PackageName,
    pub depends_on: PackageName,
    pub req: String,
}

pub struct TransactionManager {
    db: SqlitePool,
    install_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Symlink,
    Dir,
}

impl EntryKind {
    fn code(self) -> &'static str {
        match self {
            EntryKind::File => "f",
            EntryKind::Symlink => "l",
            EntryKind::Dir => "d",
        }
    }
}

struct ArtifactEntry {
    src: PathBuf,
    dest: PathBuf,
    kind: EntryKind,
}

/// A file staged next to its destination, waiting to be swapped in.
struct Staged {
    temp: PathBuf,
    dest: PathBuf,
}

/// A destination that has been swapped in (with the backup of what was there).
struct Swapped {
    dest: PathBuf,
    backup: Option<PathBuf>,
}

/// Tracks every filesystem change of an installation so it can be undone.
#[derive(Default)]
struct Journal {
    created_dirs: Vec<PathBuf>,
    staged: Vec<Staged>,
    swapped: Vec<Swapped>,
}

impl Journal {
    fn rollback(&mut self) {
        for s in self.swapped.drain(..).rev() {
            let _ = fs::remove_file(&s.dest);
            if let Some(b) = s.backup {
                let _ = fs::rename(&b, &s.dest);
            }
        }
        for s in self.staged.drain(..) {
            let _ = fs::remove_file(&s.temp);
        }
        for d in self.created_dirs.drain(..).rev() {
            let _ = fs::remove_dir(&d);
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn path_str(p: &Path) -> Result<String, RavenError> {
    p.to_str().map(str::to_string).ok_or_else(|| {
        RavenError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("non UTF-8 path not supported: {}", p.display()),
        ))
    })
}

/// Builds a hidden sibling path, e.g. `/usr/bin/.hello.raven-new.1234`.
fn sibling(dest: &Path, tag: &str) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!(".{}.raven-{}.{}", name, tag, std::process::id()))
}

fn exists_no_follow(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

impl TransactionManager {
    pub async fn open(db_path: &Path, install_root: PathBuf) -> Result<Self, RavenError> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(30));
        let db = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        let tm = Self { db, install_root };
        tm.migrate().await?;
        Ok(tm)
    }

    async fn migrate(&self) -> Result<(), RavenError> {
        let mut tx = self.db.begin().await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *tx)
            .await?;

        if version > SCHEMA_VERSION {
            return Err(RavenError::Config(format!(
                "database schema v{} is newer than this raven supports (v{}); upgrade raven",
                version, SCHEMA_VERSION
            )));
        }

        if version < 1 {
            // Original schema (CREATE IF NOT EXISTS keeps pre-versioned databases working).
            for stmt in [
                "CREATE TABLE IF NOT EXISTS packages (
                    name TEXT PRIMARY KEY,
                    version TEXT NOT NULL,
                    hash TEXT NOT NULL
                )",
                "CREATE TABLE IF NOT EXISTS package_files (
                    package_name TEXT NOT NULL,
                    filepath TEXT NOT NULL,
                    PRIMARY KEY (package_name, filepath)
                )",
                "CREATE TABLE IF NOT EXISTS dependencies (
                    package TEXT NOT NULL,
                    depends_on TEXT NOT NULL,
                    PRIMARY KEY (package, depends_on)
                )",
            ] {
                sqlx::query(stmt).execute(&mut *tx).await?;
            }
        }

        if version < 2 {
            for stmt in [
                "ALTER TABLE packages ADD COLUMN installed_at INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE package_files ADD COLUMN kind TEXT NOT NULL DEFAULT 'f'",
                "ALTER TABLE dependencies ADD COLUMN req TEXT NOT NULL DEFAULT '*'",
                "CREATE INDEX IF NOT EXISTS idx_package_files_path ON package_files(filepath)",
                "CREATE INDEX IF NOT EXISTS idx_dependencies_dep ON dependencies(depends_on)",
            ] {
                sqlx::query(stmt).execute(&mut *tx).await?;
            }
        }

        sqlx::query(&format!("PRAGMA user_version = {}", SCHEMA_VERSION))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_installed(&self) -> Result<Vec<InstalledPackage>, RavenError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, version FROM packages ORDER BY name")
                .fetch_all(&self.db)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(n, v)| InstalledPackage {
                name: PackageName(n),
                version: v,
            })
            .collect())
    }

    pub async fn installed_version(
        &self,
        name: &PackageName,
    ) -> Result<Option<String>, RavenError> {
        Ok(
            sqlx::query_scalar("SELECT version FROM packages WHERE name = ?")
                .bind(&name.0)
                .fetch_optional(&self.db)
                .await?,
        )
    }

    pub async fn all_dependencies(&self) -> Result<Vec<InstalledDependency>, RavenError> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT package, depends_on, req FROM dependencies")
                .fetch_all(&self.db)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(p, d, r)| InstalledDependency {
                package: PackageName(p),
                depends_on: PackageName(d),
                req: r,
            })
            .collect())
    }

    async fn reverse_dependencies(&self, name: &PackageName) -> Result<Vec<String>, RavenError> {
        Ok(sqlx::query_scalar(
            "SELECT d.package FROM dependencies d
             JOIN packages p ON p.name = d.package
             WHERE d.depends_on = ? ORDER BY d.package",
        )
        .bind(&name.0)
        .fetch_all(&self.db)
        .await?)
    }

    async fn files_of(&self, name: &PackageName) -> Result<Vec<(String, String)>, RavenError> {
        Ok(
            sqlx::query_as("SELECT filepath, kind FROM package_files WHERE package_name = ?")
                .bind(&name.0)
                .fetch_all(&self.db)
                .await?,
        )
    }

    async fn other_owners(&self, path: &str, me: &str) -> Result<Vec<String>, RavenError> {
        Ok(sqlx::query_scalar(
            "SELECT package_name FROM package_files
             WHERE filepath = ? AND package_name != ? AND kind != 'd'",
        )
        .bind(path)
        .bind(me)
        .fetch_all(&self.db)
        .await?)
    }

    async fn dir_has_other_owner(&self, path: &str, me: &str) -> Result<bool, RavenError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM package_files WHERE filepath = ? AND package_name != ?",
        )
        .bind(path)
        .bind(me)
        .fetch_one(&self.db)
        .await?;
        Ok(n > 0)
    }

    fn scan_artifact(&self, artifact: &Path) -> Result<Vec<ArtifactEntry>, RavenError> {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(artifact)
            .min_depth(1)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry.map_err(|e| {
                RavenError::IoError(std::io::Error::other(format!(
                    "cannot read build artifact: {}",
                    e
                )))
            })?;
            let rel = entry
                .path()
                .strip_prefix(artifact)
                .map_err(|e| RavenError::IoError(std::io::Error::other(e.to_string())))?;
            let ft = entry.file_type();
            let kind = if ft.is_symlink() {
                EntryKind::Symlink
            } else if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else {
                return Err(RavenError::FileConflict(format!(
                    "unsupported file type in package (device, fifo or socket): {}",
                    rel.display()
                )));
            };
            let dest = self.install_root.join(rel);
            path_str(&dest)?;
            entries.push(ArtifactEntry {
                src: entry.path().to_path_buf(),
                dest,
                kind,
            });
        }
        Ok(entries)
    }

    pub async fn install_package(
        &self,
        recipe: &Recipe,
        artifact: &Path,
    ) -> Result<(), RavenError> {
        let name = recipe.name.0.as_str();
        let deps = recipe.parse_dependencies()?;
        let entries = self.scan_artifact(artifact)?;

        if !entries.iter().any(|e| e.kind != EntryKind::Dir) {
            return Err(RavenError::FileConflict(format!(
                "package '{}' produced no files (check install_commands and DESTDIR=/out)",
                name
            )));
        }

        // ---- Phase 1: preflight ------------------------------------------------
        let old_records = self.files_of(&recipe.name).await?;
        let old_dirs: HashSet<&str> = old_records
            .iter()
            .filter(|(_, k)| k == "d")
            .map(|(p, _)| p.as_str())
            .collect();

        let mut conflicts = Vec::new();
        for e in &entries {
            let dest_s = path_str(&e.dest)?;
            match e.kind {
                EntryKind::Dir => {
                    if exists_no_follow(&e.dest) && !e.dest.is_dir() {
                        conflicts.push(format!("{} exists and is not a directory", dest_s));
                    }
                }
                EntryKind::File | EntryKind::Symlink => {
                    if let Ok(md) = fs::symlink_metadata(&e.dest) {
                        if md.is_dir() {
                            conflicts.push(format!("{} exists and is a directory", dest_s));
                            continue;
                        }
                    }
                    let owners = self.other_owners(&dest_s, name).await?;
                    if !owners.is_empty() {
                        conflicts.push(format!("{} is owned by {}", dest_s, owners.join(", ")));
                    }
                }
            }
        }
        if !conflicts.is_empty() {
            let total = conflicts.len();
            let mut msg = conflicts
                .into_iter()
                .take(10)
                .collect::<Vec<_>>()
                .join("\n  ");
            if total > 10 {
                msg.push_str(&format!("\n  ... and {} more", total - 10));
            }
            return Err(RavenError::FileConflict(format!(
                "cannot install '{}':\n  {}",
                name, msg
            )));
        }

        // ---- Phases 2 & 3: stage and swap (rolled back on error) ---------------
        let mut journal = Journal::default();
        let mut owned_dirs: Vec<String> = Vec::new();
        if let Err(e) = self.stage_and_swap(&entries, &old_dirs, &mut journal, &mut owned_dirs) {
            journal.rollback();
            return Err(e);
        }

        // ---- Phase 4: commit to the database ------------------------------------
        let new_paths: HashSet<String> = entries
            .iter()
            .filter(|e| e.kind != EntryKind::Dir)
            .map(|e| path_str(&e.dest))
            .collect::<Result<_, _>>()?;

        let commit = async {
            let mut tx = self.db.begin().await?;
            sqlx::query("DELETE FROM package_files WHERE package_name = ?")
                .bind(name)
                .execute(&mut *tx)
                .await?;
            for e in entries.iter().filter(|e| e.kind != EntryKind::Dir) {
                sqlx::query(
                    "INSERT OR REPLACE INTO package_files (package_name, filepath, kind) VALUES (?, ?, ?)",
                )
                .bind(name)
                .bind(path_str(&e.dest)?)
                .bind(e.kind.code())
                .execute(&mut *tx)
                .await?;
            }
            for d in &owned_dirs {
                sqlx::query(
                    "INSERT OR REPLACE INTO package_files (package_name, filepath, kind) VALUES (?, ?, 'd')",
                )
                .bind(name)
                .bind(d)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("DELETE FROM dependencies WHERE package = ?")
                .bind(name)
                .execute(&mut *tx)
                .await?;
            for d in &deps {
                sqlx::query(
                    "INSERT OR REPLACE INTO dependencies (package, depends_on, req) VALUES (?, ?, ?)",
                )
                .bind(name)
                .bind(&d.name.0)
                .bind(d.req.to_string())
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "INSERT OR REPLACE INTO packages (name, version, hash, installed_at) VALUES (?, ?, ?, ?)",
            )
            .bind(name)
            .bind(&recipe.version)
            .bind(recipe.sha256_sum.0.trim().to_lowercase())
            .bind(now_unix())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<(), RavenError>(())
        };
        if let Err(e) = commit.await {
            journal.rollback();
            return Err(e);
        }

        // ---- Phase 5: cleanup -----------------------------------------------------
        for s in &journal.swapped {
            if let Some(b) = &s.backup {
                let _ = fs::remove_file(b);
            }
        }

        // Files from the previous version that the new version no longer ships.
        let owned_set: HashSet<&str> = owned_dirs.iter().map(String::as_str).collect();
        for (path, kind) in &old_records {
            if kind == "d" || new_paths.contains(path) {
                continue;
            }
            if self.other_owners(path, name).await?.is_empty() {
                remove_file_if_exists(Path::new(path))?;
            }
        }
        let mut stale_dirs: Vec<&str> = old_records
            .iter()
            .filter(|(p, k)| k == "d" && !owned_set.contains(p.as_str()))
            .map(|(p, _)| p.as_str())
            .collect();
        stale_dirs.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for d in stale_dirs {
            if !self.dir_has_other_owner(d, name).await? {
                let _ = fs::remove_dir(d);
            }
        }

        Ok(())
    }

    fn stage_and_swap(
        &self,
        entries: &[ArtifactEntry],
        old_dirs: &HashSet<&str>,
        journal: &mut Journal,
        owned_dirs: &mut Vec<String>,
    ) -> Result<(), RavenError> {
        // Directories (entries are sorted, parents come first).
        for e in entries.iter().filter(|e| e.kind == EntryKind::Dir) {
            let dest_s = path_str(&e.dest)?;
            if !e.dest.is_dir() {
                fs::create_dir(&e.dest)?;
                journal.created_dirs.push(e.dest.clone());
                let mode = fs::metadata(&e.src)?.permissions().mode();
                fs::set_permissions(&e.dest, fs::Permissions::from_mode(mode))?;
                owned_dirs.push(dest_s);
            } else if old_dirs.contains(dest_s.as_str()) {
                owned_dirs.push(dest_s);
            }
        }

        // Stage files and symlinks next to their destinations.
        for e in entries.iter().filter(|e| e.kind != EntryKind::Dir) {
            if let Some(parent) = e.dest.parent() {
                if !parent.is_dir() {
                    fs::create_dir_all(parent)?;
                }
            }
            let temp = sibling(&e.dest, "new");
            let _ = fs::remove_file(&temp);
            match e.kind {
                EntryKind::File => {
                    fs::copy(&e.src, &temp)?;
                    journal.staged.push(Staged {
                        temp: temp.clone(),
                        dest: e.dest.clone(),
                    });
                    fs::File::open(&temp)?.sync_all()?;
                }
                EntryKind::Symlink => {
                    let target = fs::read_link(&e.src)?;
                    std::os::unix::fs::symlink(&target, &temp)?;
                    journal.staged.push(Staged {
                        temp,
                        dest: e.dest.clone(),
                    });
                }
                EntryKind::Dir => unreachable!(),
            }
        }

        // Swap: move existing files aside, rename staged files into place.
        journal.staged.reverse();
        while let Some(s) = journal.staged.pop() {
            let backup = if exists_no_follow(&s.dest) {
                let b = sibling(&s.dest, "old");
                let _ = fs::remove_file(&b);
                if let Err(err) = fs::rename(&s.dest, &b) {
                    journal.staged.push(s);
                    return Err(err.into());
                }
                Some(b)
            } else {
                None
            };
            if let Err(err) = fs::rename(&s.temp, &s.dest) {
                if let Some(b) = &backup {
                    let _ = fs::rename(b, &s.dest);
                }
                journal.staged.push(s);
                return Err(err.into());
            }
            journal.swapped.push(Swapped {
                dest: s.dest,
                backup,
            });
        }
        Ok(())
    }

    /// Removes a package. Packages in `removing_together` are ignored when
    /// checking reverse dependencies (they are being removed in the same command).
    pub async fn remove_package(
        &self,
        pkg: &PackageName,
        removing_together: &HashSet<PackageName>,
    ) -> Result<(), RavenError> {
        if self.installed_version(pkg).await?.is_none() {
            return Err(RavenError::NotInstalled(pkg.0.clone()));
        }

        let blockers: Vec<String> = self
            .reverse_dependencies(pkg)
            .await?
            .into_iter()
            .filter(|p| !removing_together.contains(&PackageName(p.clone())))
            .collect();
        if !blockers.is_empty() {
            return Err(RavenError::DependencyError(format!(
                "cannot remove '{}', it is required by: {}",
                pkg,
                blockers.join(", ")
            )));
        }

        let records = self.files_of(pkg).await?;
        for (path, _) in records.iter().filter(|(_, k)| k != "d") {
            if self.other_owners(path, &pkg.0).await?.is_empty() {
                remove_file_if_exists(Path::new(path))?;
            }
        }
        let mut dirs: Vec<&str> = records
            .iter()
            .filter(|(_, k)| k == "d")
            .map(|(p, _)| p.as_str())
            .collect();
        dirs.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for d in dirs {
            if !self.dir_has_other_owner(d, &pkg.0).await? {
                // Only succeeds if empty; non-empty dirs (user files) are kept.
                let _ = fs::remove_dir(d);
            }
        }

        let mut tx = self.db.begin().await?;
        for table_query in [
            "DELETE FROM package_files WHERE package_name = ?",
            "DELETE FROM dependencies WHERE package = ?",
            "DELETE FROM packages WHERE name = ?",
        ] {
            sqlx::query(table_query)
                .bind(&pkg.0)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Orders a set of packages so that dependents are removed before their
    /// dependencies.
    pub async fn removal_order(
        &self,
        pkgs: &[PackageName],
    ) -> Result<Vec<PackageName>, RavenError> {
        let deps = self.all_dependencies().await?;
        let set: HashSet<&PackageName> = pkgs.iter().collect();
        let mut needed_by: HashMap<&PackageName, usize> = HashMap::new();
        for d in &deps {
            if set.contains(&d.package) && set.contains(&d.depends_on) {
                *needed_by.entry(&d.depends_on).or_default() += 1;
            }
        }
        let mut remaining: Vec<&PackageName> = Vec::new();
        for p in pkgs {
            if !remaining.contains(&p) {
                remaining.push(p);
            }
        }
        let mut order = Vec::new();
        while !remaining.is_empty() {
            let idx = remaining
                .iter()
                .position(|p| needed_by.get(p).copied().unwrap_or(0) == 0)
                .unwrap_or(0); // dependency cycle inside the set: any order works
            let p = remaining.remove(idx);
            for d in &deps {
                if &d.package == p && set.contains(&d.depends_on) {
                    if let Some(n) = needed_by.get_mut(&d.depends_on) {
                        *n = n.saturating_sub(1);
                    }
                }
            }
            order.push(p.clone());
        }
        Ok(order)
    }
}

fn remove_file_if_exists(p: &Path) -> Result<(), RavenError> {
    match fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RavenError::IoError(std::io::Error::new(
            e.kind(),
            format!("cannot remove {}: {}", p.display(), e),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::HashSum;

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        work: PathBuf,
        tm: TransactionManager,
    }

    async fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let work = tmp.path().join("work");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let tm = TransactionManager::open(&tmp.path().join("db/metadata.db"), root.clone())
            .await
            .unwrap();
        Env {
            _tmp: tmp,
            root,
            work,
            tm,
        }
    }

    fn recipe(name: &str, version: &str, deps: &[&str]) -> Recipe {
        Recipe {
            name: name.into(),
            version: version.into(),
            description: String::new(),
            target_arch: None,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            source_url: "file:///dev/null".into(),
            sha256_sum: HashSum("b".repeat(64)),
            build_commands: vec![],
            install_commands: vec![],
        }
    }

    /// Creates an artifact directory with the given files (path, content).
    fn artifact(env: &Env, id: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = env.work.join(id);
        let _ = fs::remove_dir_all(&dir);
        for (p, content) in files {
            let full = dir.join(p);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, content).unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn install_and_remove_roundtrip() {
        let env = env().await;
        let art = artifact(
            &env,
            "a",
            &[("usr/bin/hello", "v1"), ("usr/share/hello/data", "x")],
        );
        std::os::unix::fs::symlink("hello", art.join("usr/bin/hi")).unwrap();
        fs::set_permissions(art.join("usr/bin/hello"), fs::Permissions::from_mode(0o755)).unwrap();

        env.tm
            .install_package(&recipe("hello", "1.0.0", &[]), &art)
            .await
            .unwrap();

        let bin = env.root.join("usr/bin/hello");
        assert_eq!(fs::read_to_string(&bin).unwrap(), "v1");
        assert_eq!(
            fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_link(env.root.join("usr/bin/hi")).unwrap(),
            PathBuf::from("hello")
        );
        assert_eq!(env.tm.list_installed().await.unwrap().len(), 1);

        env.tm
            .remove_package(&"hello".into(), &HashSet::new())
            .await
            .unwrap();
        assert!(!bin.exists());
        assert!(!exists_no_follow(&env.root.join("usr/bin/hi")));
        // Directories created by the package are cleaned up.
        assert!(!env.root.join("usr").exists());
        assert!(env.tm.list_installed().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn preexisting_directories_are_kept_on_remove() {
        let env = env().await;
        fs::create_dir_all(env.root.join("usr/bin")).unwrap();
        let art = artifact(&env, "a", &[("usr/bin/tool", "x")]);
        env.tm
            .install_package(&recipe("tool", "1.0", &[]), &art)
            .await
            .unwrap();
        env.tm
            .remove_package(&"tool".into(), &HashSet::new())
            .await
            .unwrap();
        assert!(env.root.join("usr/bin").is_dir());
    }

    #[tokio::test]
    async fn upgrade_replaces_files_and_drops_stale_ones() {
        let env = env().await;
        let v1 = artifact(
            &env,
            "v1",
            &[("usr/lib/libd.so.1", "one"), ("usr/lib/common", "1")],
        );
        env.tm
            .install_package(&recipe("libd", "1.0.0", &[]), &v1)
            .await
            .unwrap();

        let v2 = artifact(
            &env,
            "v2",
            &[("usr/lib/libd.so.2", "two"), ("usr/lib/common", "2")],
        );
        env.tm
            .install_package(&recipe("libd", "2.0.0", &[]), &v2)
            .await
            .unwrap();

        assert!(!env.root.join("usr/lib/libd.so.1").exists());
        assert_eq!(
            fs::read_to_string(env.root.join("usr/lib/libd.so.2")).unwrap(),
            "two"
        );
        assert_eq!(
            fs::read_to_string(env.root.join("usr/lib/common")).unwrap(),
            "2"
        );
        assert_eq!(
            env.tm
                .installed_version(&"libd".into())
                .await
                .unwrap()
                .as_deref(),
            Some("2.0.0")
        );
        // No leftover staging/backup files.
        let leftovers: Vec<_> = fs::read_dir(env.root.join("usr/lib"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".raven-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn conflicting_files_are_rejected_without_changes() {
        let env = env().await;
        let a = artifact(&env, "a", &[("usr/bin/same", "a")]);
        env.tm
            .install_package(&recipe("a", "1.0", &[]), &a)
            .await
            .unwrap();

        let b = artifact(&env, "b", &[("usr/bin/same", "b"), ("usr/bin/other", "b")]);
        let err = env
            .tm
            .install_package(&recipe("b", "1.0", &[]), &b)
            .await
            .unwrap_err();
        assert!(matches!(err, RavenError::FileConflict(_)), "{err}");
        assert_eq!(
            fs::read_to_string(env.root.join("usr/bin/same")).unwrap(),
            "a"
        );
        assert!(!env.root.join("usr/bin/other").exists());
        assert!(env
            .tm
            .installed_version(&"b".into())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn file_over_directory_is_rejected() {
        let env = env().await;
        fs::create_dir_all(env.root.join("etc/thing")).unwrap();
        let a = artifact(&env, "a", &[("etc/thing", "file")]);
        let err = env
            .tm
            .install_package(&recipe("a", "1.0", &[]), &a)
            .await
            .unwrap_err();
        assert!(matches!(err, RavenError::FileConflict(_)));
        assert!(env.root.join("etc/thing").is_dir());
    }

    #[tokio::test]
    async fn empty_artifact_is_rejected() {
        let env = env().await;
        let a = env.work.join("empty");
        fs::create_dir_all(a.join("usr")).unwrap();
        assert!(env
            .tm
            .install_package(&recipe("a", "1.0", &[]), &a)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn dependency_guard_and_ordering() {
        let env = env().await;
        let lib = artifact(&env, "lib", &[("usr/lib/libx", "x")]);
        let app = artifact(&env, "app", &[("usr/bin/app", "x")]);
        env.tm
            .install_package(&recipe("libx", "1.0", &[]), &lib)
            .await
            .unwrap();
        env.tm
            .install_package(&recipe("app", "1.0", &["libx >= 1.0"]), &app)
            .await
            .unwrap();

        let err = env
            .tm
            .remove_package(&"libx".into(), &HashSet::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("required by: app"), "{err}");

        let deps = env.tm.all_dependencies().await.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].req, ">=1.0");

        let order = env
            .tm
            .removal_order(&["libx".into(), "app".into()])
            .await
            .unwrap();
        assert_eq!(
            order,
            vec![PackageName::from("app"), PackageName::from("libx")]
        );
        let set: HashSet<PackageName> = order.iter().cloned().collect();
        for p in &order {
            env.tm.remove_package(p, &set).await.unwrap();
        }
        assert!(env.tm.list_installed().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_unknown_package_fails() {
        let env = env().await;
        let err = env
            .tm
            .remove_package(&"ghost".into(), &HashSet::new())
            .await
            .unwrap_err();
        assert!(matches!(err, RavenError::NotInstalled(_)));
    }

    #[tokio::test]
    async fn migrates_legacy_database() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("legacy.db");
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true);
            let pool = SqlitePool::connect_with(opts).await.unwrap();
            for stmt in [
                "CREATE TABLE packages (name TEXT PRIMARY KEY, version TEXT NOT NULL, hash TEXT NOT NULL)",
                "CREATE TABLE package_files (package_name TEXT NOT NULL, filepath TEXT NOT NULL, PRIMARY KEY (package_name, filepath))",
                "CREATE TABLE dependencies (package TEXT NOT NULL, depends_on TEXT NOT NULL, PRIMARY KEY (package, depends_on))",
                "INSERT INTO packages VALUES ('old', '1.0.0', 'abc')",
            ] {
                sqlx::query(stmt).execute(&pool).await.unwrap();
            }
            pool.close().await;
        }
        let tm = TransactionManager::open(&db, tmp.path().to_path_buf())
            .await
            .unwrap();
        let installed = tm.list_installed().await.unwrap();
        assert_eq!(installed[0].name.0, "old");
        // Re-opening an up-to-date database is a no-op.
        drop(tm);
        TransactionManager::open(&db, tmp.path().to_path_buf())
            .await
            .unwrap();
    }
}
