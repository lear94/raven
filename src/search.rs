use crate::core::Recipe;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

pub struct SearchEngine;

impl SearchEngine {
    /// Fuzzy-matches `query` against package names (and, with lower weight,
    /// descriptions). Results are sorted by relevance, then by name.
    pub fn search<'a>(query: &str, recipes: &'a [Recipe]) -> Vec<&'a Recipe> {
        let matcher = SkimMatcherV2::default().ignore_case();
        let mut results: Vec<(i64, &Recipe)> = recipes
            .iter()
            .filter_map(|r| {
                let name = matcher.fuzzy_match(&r.name.0, query).map(|s| s * 2);
                let desc = matcher.fuzzy_match(&r.description, query);
                name.max(desc).map(|s| (s, r))
            })
            .collect();
        results.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        results.into_iter().map(|(_, r)| r).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::HashSum;

    fn recipe(name: &str, desc: &str) -> Recipe {
        Recipe {
            name: name.into(),
            version: "1.0.0".into(),
            description: desc.into(),
            target_arch: None,
            dependencies: vec![],
            source_url: "x".into(),
            sha256_sum: HashSum(String::new()),
            extra_sources: vec![],
            build_commands: vec![],
            install_commands: vec![],
            post_install_commands: vec![],
            overwrites: vec![],
        }
    }

    #[test]
    fn ranks_name_matches_first() {
        let list = vec![
            recipe("zlib", "compression library"),
            recipe("libdummy", "a dummy library"),
            recipe("hello", "GNU Hello"),
        ];
        let res = SearchEngine::search("dummy", &list);
        assert_eq!(res[0].name.0, "libdummy");
        assert!(SearchEngine::search("HELLO", &list)
            .iter()
            .any(|r| r.name.0 == "hello"));
        assert!(SearchEngine::search("qqqq", &list).is_empty());
    }
}
