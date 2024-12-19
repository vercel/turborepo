use std::sync::OnceLock;

use serde::Deserialize;
use turborepo_repository::package_graph::PackageInfo;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Strategy {
    All,
    Some,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Matcher {
    strategy: Strategy,
    dependencies: Vec<String>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Framework {
    slug: String,
    env_wildcards: Vec<String>,
    dependency_match: Matcher,
}

impl Framework {
    pub fn slug(&self) -> String {
        self.slug.clone()
    }

    pub fn env_wildcards(&self) -> &[String] {
        &self.env_wildcards
    }
}

static FRAMEWORKS: OnceLock<Vec<Framework>> = OnceLock::new();

const FRAMEWORKS_JSON: &str =
    include_str!("../../../packages/turbo-types/src/json/frameworks.json");

fn get_frameworks() -> &'static Vec<Framework> {
    FRAMEWORKS.get_or_init(|| {
        serde_json::from_str(FRAMEWORKS_JSON).expect("Unable to parse embedded JSON")
    })
}

impl Matcher {
    pub fn test(&self, workspace: &PackageInfo, is_monorepo: bool) -> bool {
        // In the case where we're not in a monorepo, i.e. single package mode
        // `unresolved_external_dependencies` is not populated. In which
        // case we should check `dependencies` instead.
        let deps = if is_monorepo {
            workspace.unresolved_external_dependencies.as_ref()
        } else {
            workspace.package_json.dependencies.as_ref()
        };

        match self.strategy {
            Strategy::All => self
                .dependencies
                .iter()
                .all(|dep| deps.map_or(false, |deps| deps.contains_key(dep))),
            Strategy::Some => self
                .dependencies
                .iter()
                .any(|dep| deps.map_or(false, |deps| deps.contains_key(dep))),
        }
    }
}

pub fn infer_framework(workspace: &PackageInfo, is_monorepo: bool) -> Option<&Framework> {
    let frameworks = get_frameworks();

    frameworks
        .iter()
        .find(|framework| framework.dependency_match.test(workspace, is_monorepo))
}

#[cfg(test)]
mod tests {
    use test_case::test_case;
    use turborepo_repository::{package_graph::PackageInfo, package_json::PackageJson};

    use crate::framework::{get_frameworks, infer_framework, Framework};

    fn get_framework_by_slug(slug: &str) -> &Framework {
        get_frameworks()
            .iter()
            .find(|framework| framework.slug == slug)
            .expect("framework not found")
    }

    #[test_case(PackageInfo::default(), None, true; "empty dependencies")]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("blitz".to_string(), "*".to_string())].into_iter().collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("blitzjs")),
        true;
        "blitz"
    )]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("blitz", "*"), ("next", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("blitzjs")),
        true;
        "Order is preserved (returns blitz, not next)"
    )]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("next", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("nextjs")),
        true;
        "Finds next without blitz"
    )]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("solid-js", "*"), ("solid-start", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("solidstart")),
        true;
        "match all strategy works (solid)"
    )]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("nuxt3", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("nuxtjs")),
        true;
        "match some strategy works (nuxt)"
    )]
    #[test_case(
        PackageInfo {
            unresolved_external_dependencies: Some(
                vec![("react-scripts", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
            ),
            ..Default::default()
        },
        Some(get_framework_by_slug("create-react-app")),
        true;
        "match some strategy works (create-react-app)"
    )]
    #[test_case(
        PackageInfo {
            package_json: PackageJson {
              dependencies: Some(
                vec![("next", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
              ),
              ..Default::default()
            },
            ..Default::default()
        },
        Some(get_framework_by_slug("nextjs")),
        false;
        "Finds next in non-monorepo"
    )]
    fn test_infer_framework(
        workspace_info: PackageInfo,
        expected: Option<&Framework>,
        is_monorepo: bool,
    ) {
        let framework = infer_framework(&workspace_info, is_monorepo);
        assert_eq!(framework, expected);
    }
}
