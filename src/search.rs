use crate::core::Recipe;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use owo_colors::OwoColorize;

pub struct SearchEngine;

impl SearchEngine {
    pub fn search(query: &str, recipes: &[Recipe]) {
        let matcher = SkimMatcherV2::default();
        let mut results = Vec::new();

        for recipe in recipes {
            // Calculate relevance score
            if let Some(score) = matcher.fuzzy_match(&recipe.name.0, query) {
                results.push((score, recipe));
            }
        }

        // Sort by relevance (descending)
        results.sort_by(|a, b| b.0.cmp(&a.0));

        println!("{}", "SEARCH RESULTS".bold().underline());
        if results.is_empty() {
            println!("No packages found matching '{}'", query);
        } else {
            for (_, recipe) in results {
                println!("{} - {}", recipe.name.0.green().bold(), recipe.description);
            }
        }
        println!();
    }
}
