use crate::builder::Builder;
use crate::core::TransactionManager;
use crate::core::{PackageName, RavenError, Recipe};
use crate::ui::{create_spinner, log_success};
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Reactor {
    tm: Arc<TransactionManager>,
    builder: Arc<Builder>,
}

impl Reactor {
    pub fn new(tm: Arc<TransactionManager>, builder: Arc<Builder>) -> Self {
        Self { tm, builder }
    }

    pub async fn execute(
        &self,
        targets: Vec<PackageName>,
        recipes: HashMap<PackageName, Recipe>,
    ) -> Result<(), RavenError> {
        // 1. Resolve DAG (Directed Acyclic Graph)
        let mut build_order = Vec::new();
        let mut visited = HashSet::new();
        let mut temp_visited = HashSet::new();

        for target in targets {
            self.visit(
                &target,
                &recipes,
                &mut visited,
                &mut temp_visited,
                &mut build_order,
            )?;
        }

        // 2. Execute Build & Install
        for pkg_name in build_order {
            let recipe = recipes.get(&pkg_name).unwrap();

            let spinner = create_spinner(&format!("Processing {}...", pkg_name.0));

            // Compile
            let artifact = self.builder.build(recipe).await?;

            // ACID Install
            self.tm.install_package(recipe, &artifact).await?;

            spinner.finish_and_clear();
            log_success(&format!("Installed {} v{}", pkg_name.0, recipe.version));
        }

        Ok(())
    }

    fn visit(
        &self,
        node: &PackageName,
        recipes: &HashMap<PackageName, Recipe>,
        visited: &mut HashSet<PackageName>,
        temp_visited: &mut HashSet<PackageName>,
        order: &mut Vec<PackageName>,
    ) -> Result<(), RavenError> {
        // Cycle detection
        if temp_visited.contains(node) {
            return Err(RavenError::DependencyError(format!(
                "Circular dependency detected involving {}",
                node.0
            )));
        }
        if visited.contains(node) {
            return Ok(());
        }

        temp_visited.insert(node.clone());

        let recipe = recipes
            .get(node)
            .ok_or_else(|| RavenError::DependencyError(format!("Package not found: {}", node.0)))?;

        // --- SEMVER VALIDATION ---
        let deps = recipe.parse_dependencies()?;

        for dep_req in deps {
            let candidate = recipes.get(&dep_req.name).ok_or_else(|| {
                RavenError::DependencyError(format!("Missing dependency: {}", dep_req.name.0))
            })?;

            let candidate_version = Version::parse(&candidate.version).map_err(|e| {
                RavenError::DependencyError(format!(
                    "Package {} has invalid semver: {}",
                    candidate.name.0, e
                ))
            })?;

            // Mathematical check (e.g., 1.5.0 >= 1.0.0)
            if !dep_req.req.matches(&candidate_version) {
                return Err(RavenError::DependencyError(format!(
                    "Version mismatch for '{}': required '{}', but found '{}'",
                    dep_req.name.0, dep_req.req, candidate.version
                )));
            }

            self.visit(&dep_req.name, recipes, visited, temp_visited, order)?;
        }
        // -------------------------

        temp_visited.remove(node);
        visited.insert(node.clone());
        order.push(node.clone());

        Ok(())
    }
}
