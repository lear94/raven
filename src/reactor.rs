use crate::builder::Builder;
use crate::core::{parse_version, PackageName, RavenError, Recipe};
use crate::signals::CriticalSection;
use crate::transaction::{InstalledDependency, TransactionManager};
use crate::ui::{log_info, log_success, log_warn};
use console::style;
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Reactor {
    tm: Arc<TransactionManager>,
    builder: Arc<Builder>,
}

/// Resolves the build order for `targets`.
///
/// * Targets are always (re)built.
/// * A dependency already installed at a version satisfying the requirement is
///   left untouched; otherwise it is built from its recipe, provided the recipe
///   version satisfies the requirement.
/// * Packages are returned in dependency order (dependencies first).
pub fn resolve(
    targets: &[PackageName],
    recipes: &HashMap<PackageName, Recipe>,
    installed: &HashMap<PackageName, Version>,
) -> Result<Vec<PackageName>, RavenError> {
    struct Ctx<'a> {
        recipes: &'a HashMap<PackageName, Recipe>,
        installed: &'a HashMap<PackageName, Version>,
        targets: HashSet<&'a PackageName>,
        visited: HashSet<PackageName>,
        stack: Vec<PackageName>,
        order: Vec<PackageName>,
    }

    fn visit(ctx: &mut Ctx, node: &PackageName) -> Result<(), RavenError> {
        if ctx.visited.contains(node) {
            return Ok(());
        }
        if let Some(pos) = ctx.stack.iter().position(|n| n == node) {
            let mut cycle: Vec<String> = ctx.stack[pos..].iter().map(|n| n.0.clone()).collect();
            cycle.push(node.0.clone());
            return Err(RavenError::DependencyError(format!(
                "Circular dependency detected: {}",
                cycle.join(" -> ")
            )));
        }

        let recipe = ctx
            .recipes
            .get(node)
            .ok_or_else(|| match ctx.stack.last() {
                Some(parent) => RavenError::DependencyError(format!(
                    "Missing dependency: '{}' (required by '{}') has no recipe",
                    node, parent
                )),
                None => RavenError::DependencyError(format!("Package not found: {}", node)),
            })?;

        ctx.stack.push(node.clone());
        for dep in recipe.parse_dependencies()? {
            let installed_ok = ctx
                .installed
                .get(&dep.name)
                .is_some_and(|v| dep.req.matches(v));
            if installed_ok && !ctx.targets.contains(&dep.name) {
                continue;
            }

            match ctx.recipes.get(&dep.name) {
                Some(candidate) => {
                    let cv = candidate.parsed_version()?;
                    if !dep.req.matches(&cv) {
                        let installed_note = ctx
                            .installed
                            .get(&dep.name)
                            .map(|v| format!(", installed '{}'", v))
                            .unwrap_or_default();
                        return Err(RavenError::DependencyError(format!(
                            "Version mismatch for '{}': '{}' requires '{}', but the recipe provides '{}'{}",
                            dep.name, node, dep.req, candidate.version, installed_note
                        )));
                    }
                }
                None if ctx.installed.contains_key(&dep.name) => {
                    return Err(RavenError::DependencyError(format!(
                        "Version mismatch for '{}': '{}' requires '{}', installed '{}' and no recipe available",
                        dep.name, node, dep.req, ctx.installed[&dep.name]
                    )));
                }
                None => {}
            }
            visit(ctx, &dep.name)?;
        }
        ctx.stack.pop();

        ctx.visited.insert(node.clone());
        ctx.order.push(node.clone());
        Ok(())
    }

    let mut ctx = Ctx {
        recipes,
        installed,
        targets: targets.iter().collect(),
        visited: HashSet::new(),
        stack: Vec::new(),
        order: Vec::new(),
    };
    for t in targets {
        visit(&mut ctx, t)?;
    }
    Ok(ctx.order)
}

/// Ensures that installing the planned versions doesn't break installed
/// packages that depend on them (and are not themselves being rebuilt).
pub fn check_reverse_dependencies(
    plan: &[PackageName],
    recipes: &HashMap<PackageName, Recipe>,
    installed_deps: &[InstalledDependency],
) -> Result<(), RavenError> {
    let in_plan: HashSet<&PackageName> = plan.iter().collect();
    for pkg in plan {
        let new_version = recipes[pkg].parsed_version()?;
        for d in installed_deps
            .iter()
            .filter(|d| &d.depends_on == pkg && !in_plan.contains(&d.package))
        {
            let Ok(req) = VersionReq::parse(&d.req) else {
                continue;
            };
            if !req.matches(&new_version) {
                return Err(RavenError::DependencyError(format!(
                    "Version mismatch: installing '{}' {} would break installed package '{}' (requires '{} {}')",
                    pkg, new_version, d.package, pkg, d.req
                )));
            }
        }
    }
    Ok(())
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
        let installed_list = self.tm.list_installed().await?;
        let mut installed = HashMap::new();
        for p in &installed_list {
            match parse_version(&p.version) {
                Ok(v) => {
                    installed.insert(p.name.clone(), v);
                }
                Err(_) => log_warn(&format!(
                    "Installed package '{}' has unparseable version '{}'; it will be rebuilt if needed",
                    p.name, p.version
                )),
            }
        }

        let mut unique_targets = Vec::new();
        for t in targets {
            if !unique_targets.contains(&t) {
                unique_targets.push(t);
            }
        }

        let plan = resolve(&unique_targets, &recipes, &installed)?;
        check_reverse_dependencies(&plan, &recipes, &self.tm.all_dependencies().await?)?;

        println!("{}", style("Transaction plan:").bold());
        for name in &plan {
            let r = &recipes[name];
            let action = match installed_list.iter().find(|p| &p.name == name) {
                Some(p) if p.version == r.version => format!("reinstall {}", r.version),
                Some(p) => format!("{} -> {}", p.version, r.version),
                None => format!("install {}", r.version),
            };
            println!("   {} {}", style(&name.0).cyan(), action);
        }
        println!();

        let total = plan.len();
        let mut done: Vec<String> = Vec::new();
        for (i, name) in plan.iter().enumerate() {
            let recipe = &recipes[name];
            log_info(&format!(
                "[{}/{}] {} {}",
                i + 1,
                total,
                name,
                recipe.version
            ));

            let result = async {
                let output = self.builder.build(recipe).await?;
                {
                    let _guard = CriticalSection::enter();
                    self.tm.install_package(recipe, &output.out_dir).await?;
                }
                self.builder.cleanup(&output);
                crate::signals::check_interrupted()
            }
            .await;

            if let Err(e) = result {
                if !done.is_empty() {
                    log_warn(&format!(
                        "Packages installed before the failure remain installed: {}",
                        done.join(", ")
                    ));
                }
                return Err(e);
            }

            log_success(&format!("Installed {} v{}", name, recipe.version));
            done.push(name.0.clone());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::HashSum;

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

    fn repo(rs: Vec<Recipe>) -> HashMap<PackageName, Recipe> {
        rs.into_iter().map(|r| (r.name.clone(), r)).collect()
    }

    fn names(v: &[PackageName]) -> Vec<&str> {
        v.iter().map(|n| n.0.as_str()).collect()
    }

    #[test]
    fn resolves_dependencies_first() {
        let r = repo(vec![
            recipe("app", "1.0.0", &["lib >= 1.0.0", "util"]),
            recipe("lib", "1.2.0", &["util"]),
            recipe("util", "0.1.0", &[]),
        ]);
        let plan = resolve(&["app".into()], &r, &HashMap::new()).unwrap();
        assert_eq!(names(&plan), vec!["util", "lib", "app"]);
    }

    #[test]
    fn detects_cycles() {
        let r = repo(vec![
            recipe("a", "1.0.0", &["b"]),
            recipe("b", "1.0.0", &["a"]),
        ]);
        let err = resolve(&["a".into()], &r, &HashMap::new()).unwrap_err();
        assert!(err.to_string().contains("a -> b -> a"), "{err}");
    }

    #[test]
    fn reports_missing_packages() {
        let r = repo(vec![recipe("a", "1.0.0", &["ghost"])]);
        assert!(resolve(&["a".into()], &r, &HashMap::new())
            .unwrap_err()
            .to_string()
            .contains("Missing dependency"));
        assert!(resolve(&["nope".into()], &r, &HashMap::new())
            .unwrap_err()
            .to_string()
            .contains("Package not found"));
    }

    #[test]
    fn rejects_version_mismatch() {
        let r = repo(vec![
            recipe("app", "1.0.0", &["lib >= 2.0.0"]),
            recipe("lib", "1.0.0", &[]),
        ]);
        let installed = HashMap::from([("lib".into(), Version::new(1, 0, 0))]);
        let err = resolve(&["app".into()], &r, &installed).unwrap_err();
        assert!(err.to_string().contains("Version mismatch"), "{err}");
    }

    #[test]
    fn skips_satisfied_installed_dependencies() {
        let r = repo(vec![
            recipe("app", "1.0.0", &["lib >= 1.0.0"]),
            recipe("lib", "1.5.0", &[]),
        ]);
        let installed = HashMap::from([("lib".into(), Version::new(1, 0, 0))]);
        let plan = resolve(&["app".into()], &r, &installed).unwrap();
        assert_eq!(names(&plan), vec!["app"]);
        // Explicit targets are always rebuilt.
        let plan = resolve(&["lib".into(), "app".into()], &r, &installed).unwrap();
        assert_eq!(names(&plan), vec!["lib", "app"]);
    }

    #[test]
    fn upgrades_unsatisfied_installed_dependencies() {
        let r = repo(vec![
            recipe("app", "1.0.0", &["lib >= 2.0.0"]),
            recipe("lib", "2.1.0", &[]),
        ]);
        let installed = HashMap::from([("lib".into(), Version::new(1, 0, 0))]);
        let plan = resolve(&["app".into()], &r, &installed).unwrap();
        assert_eq!(names(&plan), vec!["lib", "app"]);
    }

    #[test]
    fn reverse_dependency_check() {
        let r = repo(vec![recipe("lib", "2.0.0", &[])]);
        let deps = vec![InstalledDependency {
            package: "app".into(),
            depends_on: "lib".into(),
            req: "<2.0.0".into(),
        }];
        let err = check_reverse_dependencies(&["lib".into()], &r, &deps).unwrap_err();
        assert!(err.to_string().contains("would break"), "{err}");

        // Fine if the dependent is rebuilt in the same transaction.
        let r = repo(vec![
            recipe("lib", "2.0.0", &[]),
            recipe("app", "1.0.0", &["lib"]),
        ]);
        check_reverse_dependencies(&["lib".into(), "app".into()], &r, &deps).unwrap();
    }
}
