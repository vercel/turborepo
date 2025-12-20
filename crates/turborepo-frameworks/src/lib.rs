//! Framework detection and configuration inference for Turborepo.
//! Automatically identifies JavaScript frameworks and what environment
//! variables impact it.

use std::{collections::HashMap, sync::OnceLock};

use serde::Deserialize;
use turborepo_repository::package_graph::PackageInfo;

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
enum Strategy {
    All,
    Some,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Matcher {
    strategy: Strategy,
    dependencies: Vec<String>,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EnvConditionKey {
    key: String,
    value: Option<String>,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EnvConditional {
    when: EnvConditionKey,
    include: Vec<String>,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Framework {
    slug: Slug,
    env_wildcards: Vec<String>,
    env_conditionals: Option<Vec<EnvConditional>>,
    dependency_match: Matcher,
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
#[serde(transparent)]
pub struct Slug(String);

impl Framework {
    pub fn slug(&self) -> Slug {
        self.slug.clone()
    }

    pub fn env(&self, env_at_execution_start: &HashMap<String, String>) -> Vec<String> {
        let mut env_vars = self.env_wildcards.clone();

        if let Some(env_conditionals) = &self.env_conditionals {
            for conditional in env_conditionals {
                let (key, expected_value) = (&conditional.when.key, &conditional.when.value);

                if let Some(actual_value) = env_at_execution_start.get(key)
                    && (expected_value.is_none() || expected_value.as_ref() == Some(actual_value))
                {
                    env_vars.extend(conditional.include.iter().cloned());
                }
            }
        }

        env_vars
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
                .all(|dep| deps.is_some_and(|deps| deps.contains_key(dep))),
            Strategy::Some => self
                .dependencies
                .iter()
                .any(|dep| deps.is_some_and(|deps| deps.contains_key(dep))),
        }
    }
}

impl Slug {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn framework(&self) -> &Framework {
        let frameworks = get_frameworks();
        frameworks
            .iter()
            .find(|framework| framework.slug.as_str() == self.as_str())
            .expect("slug is only constructed via deserialization")
    }
}

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    use std::collections::HashMap;

    use test_case::test_case;
    use turborepo_repository::{package_graph::PackageInfo, package_json::PackageJson};

    use super::*;

    fn get_framework_by_slug(slug: &str) -> &Framework {
        get_frameworks()
            .iter()
            .find(|framework| framework.slug.as_str() == slug)
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

    #[test]
    fn test_env_with_no_conditions() {
        let framework = get_framework_by_slug("nextjs");

        let env_at_execution_start = HashMap::new();
        let env_vars = framework.env(&env_at_execution_start);

        assert_eq!(
            env_vars,
            framework.env_wildcards.clone(),
            "Expected env_wildcards when no conditionals exist"
        );
    }

    #[test]
    fn test_env_with_matching_condition() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        let env_vars = framework.env(&env_at_execution_start);

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected VERCEL_DEPLOYMENT_ID to be included when condition is met"
        );
    }

    #[test]
    fn test_env_with_non_matching_condition() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "0".to_string(),
        );

        let env_vars = framework.env(&env_at_execution_start);

        assert_eq!(
            env_vars,
            framework.env_wildcards.clone(),
            "Expected only env_wildcards when condition is not met"
        );
    }

    #[test]
    fn test_env_with_condition_without_value_requirement() {
        let mut framework = get_framework_by_slug("nextjs").clone();

        if let Some(env_conditionals) = framework.env_conditionals.as_mut() {
            env_conditionals[0].when.value = None;
        }

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "random".to_string(),
        );

        let env_vars = framework.env(&env_at_execution_start);

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected VERCEL_DEPLOYMENT_ID to be included when condition key exists, regardless \
             of value"
        );
    }

    #[test]
    fn test_env_with_multiple_conditions() {
        let mut framework = get_framework_by_slug("nextjs").clone();

        if let Some(env_conditionals) = framework.env_conditionals.as_mut() {
            env_conditionals.push(EnvConditional {
                when: EnvConditionKey {
                    key: "ANOTHER_CONDITION".to_string(),
                    value: Some("true".to_string()),
                },
                include: vec!["ADDITIONAL_ENV_VAR".to_string()],
            });
        }

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );
        env_at_execution_start.insert("ANOTHER_CONDITION".to_string(), "true".to_string());

        let env_vars = framework.env(&env_at_execution_start);

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());
        expected_vars.push("ADDITIONAL_ENV_VAR".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected both VERCEL_DEPLOYMENT_ID and ADDITIONAL_ENV_VAR when both conditions are \
             met"
        );
    }

    #[test]
    fn test_framework_slug_roundtrip() {
        for framework in get_frameworks() {
            assert_eq!(framework, framework.slug().framework());
        }
    }
}
