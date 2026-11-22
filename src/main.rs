mod builder;
mod config;
mod core;
mod lock;
mod reactor;
mod sandbox;
mod search;
mod signals;
mod sources;
mod transaction;
mod ui;

use crate::builder::Builder;
use crate::config::{validate_repo_url, ConfigManager, LOCAL_MODE};
use crate::core::{parse_version, PackageName, Recipe};
use crate::lock::ProcessLock;
use crate::reactor::Reactor;
use crate::search::SearchEngine;
use crate::signals::CriticalSection;
use crate::sources::SourceManager;
use crate::transaction::TransactionManager;
use crate::ui::{log_error, log_info, log_success, log_warn, print_banner};
use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use console::style;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

const DEFAULT_HOME: &str = "/var/lib/raven";

#[derive(Parser)]
#[command(
    name = "raven",
    version,
    about = "Transactional, sandboxed, source-based package manager",
    after_help = "Data directory: /var/lib/raven (override with the RAVEN_HOME environment variable)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and install packages (and their missing dependencies)
    Install {
        #[arg(required = true, value_name = "PACKAGE")]
        packages: Vec<String>,
    },
    /// Remove installed packages (refused if other packages depend on them)
    Remove {
        #[arg(required = true, value_name = "PACKAGE")]
        packages: Vec<String>,
    },
    /// Sync recipes from the configured repository
    Update,
    /// Rebuild installed packages that have a newer recipe version
    Upgrade,
    /// Fuzzy search available recipes
    Search { query: String },
    /// List installed packages
    List,
    /// Show or change configuration
    Config {
        /// Recipe repository: git URL, local path, or "local-mode"
        #[arg(long, value_name = "URL")]
        set_repo: Option<String>,
        /// Show current configuration (default)
        #[arg(long)]
        show: bool,
    },
}

struct Paths {
    home: PathBuf,
    db: PathBuf,
    recipes: PathBuf,
    build: PathBuf,
    cache: PathBuf,
    lock: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let home = std::env::var_os("RAVEN_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_HOME));
        Self {
            db: home.join("metadata.db"),
            recipes: home.join("recipes"),
            build: home.join("build"),
            cache: home.join("cache"),
            lock: home.join("raven.lock"),
            home,
        }
    }

    /// Creates the data directory and takes the global lock.
    fn prepare(&self) -> anyhow::Result<ProcessLock> {
        std::fs::create_dir_all(&self.home).map_err(|e| permission_hint(e, &self.home))?;
        ProcessLock::acquire(&self.lock).map_err(|e| match e {
            core::RavenError::IoError(io) => permission_hint(io, &self.lock),
            other => other.into(),
        })
    }
}

fn permission_hint(e: std::io::Error, path: &Path) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        anyhow::anyhow!(
            "permission denied accessing {} (run with sudo, or set RAVEN_HOME)",
            path.display()
        )
    } else {
        anyhow::Error::new(e).context(format!("cannot access {}", path.display()))
    }
}

fn require_root(action: &str) -> anyhow::Result<()> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        bail!(
            "'{}' modifies the system and must be run as root (use sudo)",
            action
        );
    }
    Ok(())
}

fn validate_names(packages: &[String]) -> anyhow::Result<Vec<PackageName>> {
    packages
        .iter()
        .map(|p| {
            if PackageName::is_valid(p) {
                Ok(PackageName(p.clone()))
            } else {
                bail!("invalid package name '{}'", p)
            }
        })
        .collect()
}

fn load_recipes(sm: &SourceManager) -> anyhow::Result<HashMap<PackageName, Recipe>> {
    let loaded = sm.load()?;
    for w in &loaded.warnings {
        log_warn(&format!("Skipping recipe {}", w));
    }
    if loaded.recipes.is_empty() {
        let hint = if sm.is_local_mode() {
            "Add recipe .toml files there (local mode)."
        } else {
            "Run 'raven update' to fetch them."
        };
        log_warn(&format!(
            "No recipes found in {}. {}",
            sm.recipes_dir().display(),
            hint
        ));
    }
    Ok(loaded.recipes)
}

async fn open_db(paths: &Paths) -> anyhow::Result<Arc<TransactionManager>> {
    let tm = TransactionManager::open(&paths.db, PathBuf::from("/"))
        .await
        .with_context(|| format!("cannot open package database {}", paths.db.display()))?;
    Ok(Arc::new(tm))
}

fn make_reactor(paths: &Paths, tm: Arc<TransactionManager>) -> anyhow::Result<Reactor> {
    let builder = Builder::new(paths.build.clone(), paths.cache.clone())?;
    Ok(Reactor::new(tm, Arc::new(builder)))
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    print_banner();
    signals::install_handler();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log_error(&format!("{:#}", e));
            if matches!(
                e.downcast_ref::<core::RavenError>(),
                Some(core::RavenError::Interrupted)
            ) {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let paths = Paths::new();
    let config_manager = ConfigManager::new(&paths.home);
    let config = config_manager.load()?;
    let sm = SourceManager::new(paths.recipes.clone(), config.repo_url.clone());

    match cli.command {
        Commands::Install { packages } => {
            require_root("install")?;
            let targets = validate_names(&packages)?;
            let _lock = paths.prepare()?;
            let recipes = load_recipes(&sm)?;
            let tm = open_db(&paths).await?;
            make_reactor(&paths, tm)?.execute(targets, recipes).await?;
        }

        Commands::Remove { packages } => {
            require_root("remove")?;
            let targets = validate_names(&packages)?;
            let _lock = paths.prepare()?;
            let tm = open_db(&paths).await?;
            let order = tm.removal_order(&targets).await?;
            let set: HashSet<PackageName> = order.iter().cloned().collect();
            // Validate everything before removing anything.
            for p in &order {
                if tm.installed_version(p).await?.is_none() {
                    return Err(core::RavenError::NotInstalled(p.0.clone()).into());
                }
            }
            for p in &order {
                {
                    let _guard = CriticalSection::enter();
                    tm.remove_package(p, &set).await?;
                }
                log_success(&format!("Removed {}", p));
                signals::check_interrupted()?;
            }
        }

        Commands::Update => {
            let _lock = paths.prepare()?;
            if sm.is_local_mode() {
                log_info(&format!(
                    "Local mode: recipes are managed manually in {}",
                    paths.recipes.display()
                ));
                sm.sync()?;
            } else {
                log_info(&format!("Syncing recipes from {}", config.repo_url));
                sm.sync()?;
            }
            let count = load_recipes(&sm)?.len();
            log_success(&format!(
                "{} recipes available. Run 'raven upgrade' to apply updates.",
                count
            ));
        }

        Commands::Upgrade => {
            require_root("upgrade")?;
            let _lock = paths.prepare()?;
            let tm = open_db(&paths).await?;
            let recipes = load_recipes(&sm)?;
            println!("{}", style("Checking for updates...").bold());

            let mut to_upgrade = Vec::new();
            for pkg in tm.list_installed().await? {
                let Some(recipe) = recipes.get(&pkg.name) else {
                    continue;
                };
                let installed = match parse_version(&pkg.version) {
                    Ok(v) => v,
                    Err(e) => {
                        log_warn(&format!("Skipping {}: {}", pkg.name, e));
                        continue;
                    }
                };
                let available = recipe.parsed_version()?;
                if available > installed {
                    println!(
                        "   ➜ {} {} -> {}",
                        style(&pkg.name.0).cyan(),
                        style(&pkg.version).red(),
                        style(&recipe.version).green()
                    );
                    to_upgrade.push(pkg.name);
                }
            }

            if to_upgrade.is_empty() {
                log_success("System is up to date.");
            } else {
                println!(
                    "\nStarting upgrade transaction for {} package(s)...",
                    to_upgrade.len()
                );
                make_reactor(&paths, tm)?
                    .execute(to_upgrade, recipes)
                    .await?;
                log_success("System upgrade completed successfully.");
            }
        }

        Commands::Search { query } => {
            let recipes = load_recipes(&sm)?;
            let mut list: Vec<Recipe> = recipes.into_values().collect();
            list.sort_by(|a, b| a.name.cmp(&b.name));
            let results = SearchEngine::search(&query, &list);
            println!("{}", style("SEARCH RESULTS").bold().underlined());
            if results.is_empty() {
                println!("No packages found matching '{}'", query);
            } else {
                for r in results {
                    println!(
                        "{} {} - {}",
                        style(&r.name.0).green().bold(),
                        style(&r.version).dim(),
                        r.description
                    );
                }
            }
        }

        Commands::List => {
            if !paths.db.exists() {
                println!("No packages installed.");
                return Ok(());
            }
            let tm = open_db(&paths).await?;
            let installed = tm.list_installed().await?;
            if installed.is_empty() {
                println!("No packages installed.");
            }
            for p in installed {
                println!("{} {}", style(&p.name.0).green().bold(), p.version);
            }
        }

        Commands::Config { set_repo, show } => {
            if let Some(url) = set_repo {
                validate_repo_url(&url)?;
                let _lock = paths.prepare()?;
                let mut new_config = config.clone();
                new_config.repo_url = url.clone();
                config_manager.save(&new_config).map_err(|e| match e {
                    core::RavenError::IoError(io) => permission_hint(io, config_manager.path()),
                    other => other.into(),
                })?;
                log_success(&format!("Repository URL updated to: {}", url));
                if url != LOCAL_MODE {
                    println!("Run 'raven update' to fetch recipes from the new repository.");
                }
            } else {
                let _ = show;
                println!("Current Configuration:");
                println!("   Repo URL:   {}", config.repo_url);
                println!("   Data dir:   {}", paths.home.display());
                println!("   Recipes:    {}", paths.recipes.display());
            }
        }
    }

    Ok(())
}
