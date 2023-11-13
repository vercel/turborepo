use std::sync::OnceLock;

use turborepo_repository::package_graph::WorkspaceInfo;

#[derive(Debug, PartialEq)]
enum Strategy {
    All,
    Some,
}

#[derive(Debug, PartialEq)]
struct Matcher {
    strategy: Strategy,
    dependencies: Vec<&'static str>,
}

#[derive(Debug, PartialEq)]
pub struct Framework {
    slug: &'static str,
    env_wildcards: Vec<&'static str>,
    dependency_match: Matcher,
}

impl Framework {
    pub fn slug(&self) -> &'static str {
        self.slug
    }

    pub fn env_wildcards(&self) -> &[&'static str] {
        &self.env_wildcards
    }
}

static FRAMEWORKS: OnceLock<[Framework; 12]> = OnceLock::new();

fn get_frameworks() -> &'static [Framework] {
    FRAMEWORKS.get_or_init(|| {
        [
            Framework {
                slug: "blitzjs",
                env_wildcards: vec!["NEXT_PUBLIC_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["blitz"],
                },
            },
            Framework {
                slug: "nextjs",
                env_wildcards: vec!["NEXT_PUBLIC_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["next"],
                },
            },
            Framework {
                slug: "gatsby",
                env_wildcards: vec!["GATSBY_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["gatsby"],
                },
            },
            Framework {
                slug: "astro",
                env_wildcards: vec!["PUBLIC_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["astro"],
                },
            },
            Framework {
                slug: "solidstart",
                env_wildcards: vec!["VITE_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["solid-js", "solid-start"],
                },
            },
            Framework {
                slug: "vue",
                env_wildcards: vec!["VUE_APP_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["@vue/cli-service"],
                },
            },
            Framework {
                slug: "sveltekit",
                env_wildcards: vec!["VITE_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["@sveltejs/kit"],
                },
            },
            Framework {
                slug: "create-react-app",
                env_wildcards: vec!["REACT_APP_*"],
                dependency_match: Matcher {
                    strategy: Strategy::Some,
                    dependencies: vec!["react-scripts", "react-dev-utils"],
                },
            },
            Framework {
                slug: "nuxtjs",
                env_wildcards: vec!["NUXT_ENV_*"],
                dependency_match: Matcher {
                    strategy: Strategy::Some,
                    dependencies: vec!["nuxt", "nuxt-edge", "nuxt3", "nuxt3-edge"],
                },
            },
            Framework {
                slug: "redwoodjs",
                env_wildcards: vec!["REDWOOD_ENV_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["@redwoodjs/core"],
                },
            },
            Framework {
                slug: "vite",
                env_wildcards: vec!["VITE_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["vite"],
                },
            },
            Framework {
                slug: "sanity",
                env_wildcards: vec!["SANITY_STUDIO_*"],
                dependency_match: Matcher {
                    strategy: Strategy::All,
                    dependencies: vec!["@sanity/cli"],
                },
            },
        ]
    })
}

impl Matcher {
    pub fn test(&self, workspace: &WorkspaceInfo, is_monorepo: bool) -> bool {
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
                .all(|dep| deps.map_or(false, |deps| deps.contains_key(*dep))),
            Strategy::Some => self
                .dependencies
                .iter()
                .any(|dep| deps.map_or(false, |deps| deps.contains_key(*dep))),
        }
    }
}

pub fn infer_framework(workspace: &WorkspaceInfo, is_monorepo: bool) -> Option<&'static Framework> {
    let frameworks = get_frameworks();

    frameworks
        .iter()
        .find(|framework| framework.dependency_match.test(workspace, is_monorepo))
}

#[cfg(test)]
mod tests {
    use test_case::test_case;
    use turborepo_repository::{package_graph::WorkspaceInfo, package_json::PackageJson};

    use crate::framework::{get_frameworks, infer_framework, Framework};

    fn get_framework_by_slug(slug: &str) -> &'static Framework {
        get_frameworks()
            .iter()
            .find(|framework| framework.slug == slug)
            .expect("framework not found")
    }

    #[test_case(WorkspaceInfo::default(), None, true; "empty dependencies")]
    #[test_case(
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        WorkspaceInfo {
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
        workspace_info: WorkspaceInfo,
        expected: Option<&'static Framework>,
        is_monorepo: bool,
    ) {
        let framework = infer_framework(&workspace_info, is_monorepo);
        assert_eq!(framework, expected);
    }
}
