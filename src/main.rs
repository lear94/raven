mod builder;
mod config;
mod core;
mod reactor;
mod sandbox;
mod search;
mod sources;
mod ui;

use crate::builder::Builder;
use crate::config::ConfigManager;
use crate::core::{PackageName, TransactionManager};
use crate::reactor::Reactor;
use crate::search::SearchEngine;
use crate::sources::SourceManager;
use crate::ui::{log_error, log_success, print_banner};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use semver::Version;
use std::process::exit;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "raven")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Install {
        packages: Vec<String>,
    },
    Remove {
        packages: Vec<String>,
    },
    Update,
    Upgrade, // NEW: Checks available versions against installed ones
    Search {
        query: String,
    },
    Config {
        #[arg(long)]
        set_repo: Option<String>,
        #[arg(long, action)]
        show: bool,
    },
}

#[tokio::main]
async fn main() {
    print_banner();
    if let Err(e) = run().await {
        log_error(&e.to_string());
        exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let raven_root = std::path::Path::new("/var/lib/raven");
    if !raven_root.exists() {
        std::fs::create_dir_all(raven_root)?;
    }

    let config_manager = ConfigManager::new(raven_root);
    let mut config = config_manager.load().await?;

    let cli = Cli::parse();

    let tm = Arc::new(
        TransactionManager::new(
            &format!("sqlite://{}/metadata.db?mode=rwc", raven_root.display()),
            "/tmp/raven_stage".into(),
        )
        .await?,
    );

    let builder = Arc::new(Builder::new("/tmp/raven_build".into()));
    let reactor = Reactor::new(tm.clone(), builder.clone());

    let sm = SourceManager::new(raven_root.join("recipes"), config.repo_url.clone());

    match cli.command {
        Commands::Install { packages } => {
            let recipes = sm.load()?;
            let targets = packages.into_iter().map(PackageName).collect();
            reactor.execute(targets, recipes).await?;
        }
        Commands::Remove { packages } => {
            for p in packages {
                tm.remove_package(&PackageName(p.clone())).await?;
                log_success(&format!("Removed {}", p));
            }
        }
        Commands::Update => {
            println!("Syncing recipes from: {}", config.repo_url);
            sm.sync()?;
            log_success("Recipes updated. Run 'raven upgrade' to apply available updates.");
        }
        Commands::Upgrade => {
            // 1. Get installed packages
            let installed = tm.list_installed().await?;
            // 2. Load latest recipes
            let recipes = sm.load()?;

            let mut to_upgrade = Vec::new();
            println!("{}", "Checking for updates...".bold());

            for (pkg_name, installed_ver_str) in installed {
                if let Some(recipe) = recipes.get(&pkg_name) {
                    let installed_ver = Version::parse(&installed_ver_str)?;
                    let recipe_ver = Version::parse(&recipe.version)?;

                    // If remote is newer, mark for upgrade
                    if recipe_ver > installed_ver {
                        println!(
                            "   ➜ {} {} -> {}",
                            pkg_name.0.cyan(),
                            installed_ver.to_string().red(),
                            recipe_ver.to_string().green()
                        );
                        to_upgrade.push(pkg_name);
                    }
                }
            }

            if to_upgrade.is_empty() {
                log_success("System is up to date.");
            } else {
                println!(
                    "\nStarting upgrade transaction for {} packages...",
                    to_upgrade.len()
                );
                reactor.execute(to_upgrade, recipes).await?;
                log_success("System upgrade completed successfully.");
            }
        }
        Commands::Search { query } => {
            let recipes = sm.load()?;
            let list: Vec<_> = recipes.values().cloned().collect();
            SearchEngine::search(&query, &list);
        }
        Commands::Config { set_repo, show } => {
            if let Some(url) = set_repo {
                config.repo_url = url.clone();
                config_manager.save(&config).await?;
                log_success(&format!("Repository URL updated to: {}", url));
            } else if show {
                println!("Current Configuration:");
                println!("   Repo URL: {}", config.repo_url);
            } else {
                println!("Use --show or --set-repo <URL>");
            }
        }
    }

    Ok(())
}
