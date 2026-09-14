//! Framework detection and configuration inference for Turborepo.
//! Automatically identifies JavaScript frameworks and what environment
//! variables impact it.

use std::{collections::HashMap, sync::OnceLock};

use semver::Version;
use serde::Deserialize;
use turborepo_repository::{
    external_resolution::PackageExternalDeclarations, relationships::DependencyKind,
};

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
    /// When `true`, the conditional matches only if `key` is absent from the
    /// environment. Mutually exclusive with `value` in practice.
    #[serde(default)]
    absent: Option<bool>,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EnvConditional {
    when: EnvConditionKey,
    include: Vec<String>,
    /// Inclusive floor on the resolved version of the framework's gated
    /// dependency. A conditional with a floor only applies when the exact
    /// resolved version is known and at least this version.
    #[serde(default)]
    from_version: Option<String>,
    /// Exclusive ceiling on the resolved version of the framework's gated
    /// dependency. Unknown versions satisfy the ceiling so they keep legacy
    /// behavior.
    #[serde(default)]
    until_version: Option<String>,
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

    /// Environment variables this framework's tasks depend on, given the
    /// package's external declarations.
    ///
    /// Declaration versions gate the version-sensitive conditionals: a
    /// conditional with a `fromVersion` applies only once the framework's
    /// gated dependency is known to resolve to at least that version, while an
    /// `untilVersion` ceiling also applies when the version is unknown so
    /// legacy behavior is preserved.
    pub fn env(
        &self,
        env_at_execution_start: &HashMap<String, String>,
        declarations: PackageExternalDeclarations<'_>,
    ) -> Vec<String> {
        let mut env_vars = self.env_wildcards.clone();

        if let Some(env_conditionals) = &self.env_conditionals {
            let dependency_version = self.resolved_dependency_version(declarations);

            for conditional in env_conditionals {
                if conditional.matches(env_at_execution_start, dependency_version.as_ref()) {
                    env_vars.extend(conditional.include.iter().cloned());
                }
            }
        }

        env_vars
    }

    /// Resolved version of the first gated dependency this framework declares,
    /// when the package locks it to a parseable exact version.
    fn resolved_dependency_version(
        &self,
        declarations: PackageExternalDeclarations<'_>,
    ) -> Option<Version> {
        self.dependency_match
            .dependencies
            .iter()
            .find_map(|dependency| {
                declarations
                    .iter()
                    .find(|declaration| declaration.package_name() == dependency)
                    .and_then(|declaration| declaration.resolved_version())
                    .and_then(|version| Version::parse(version).ok())
            })
    }
}

impl EnvConditional {
    fn matches(
        &self,
        env_at_execution_start: &HashMap<String, String>,
        dependency_version: Option<&Version>,
    ) -> bool {
        self.when.matches(env_at_execution_start) && self.matches_version(dependency_version)
    }

    fn matches_version(&self, dependency_version: Option<&Version>) -> bool {
        // Malformed bounds are inert: they are rejected at test time rather
        // than changing which variables a release hashes.
        if let Some(floor) = self
            .from_version
            .as_ref()
            .and_then(|floor| Version::parse(floor).ok())
            && !dependency_version.is_some_and(|version| version >= &floor)
        {
            // A floor requires a known version at or above it.
            return false;
        }

        if let Some(ceiling) = self
            .until_version
            .as_ref()
            .and_then(|ceiling| Version::parse(ceiling).ok())
            && dependency_version.is_some_and(|version| version >= &ceiling)
        {
            // An unknown version satisfies the ceiling.
            return false;
        }

        true
    }
}

impl EnvConditionKey {
    fn matches(&self, env_at_execution_start: &HashMap<String, String>) -> bool {
        if self.absent == Some(true) {
            return !env_at_execution_start.contains_key(&self.key);
        }

        env_at_execution_start.get(&self.key).is_some_and(|actual| {
            self.value
                .as_ref()
                .is_none_or(|expected| expected == actual)
        })
    }
}

static FRAMEWORKS: OnceLock<Result<Vec<Framework>, serde_json::Error>> = OnceLock::new();

const FRAMEWORKS_JSON: &str =
    include_str!("../../../packages/turbo-types/src/json/frameworks.json");

fn get_frameworks() -> Result<&'static [Framework], &'static serde_json::Error> {
    FRAMEWORKS
        .get_or_init(|| serde_json::from_str(FRAMEWORKS_JSON))
        .as_ref()
        .map(Vec::as_slice)
}

impl Matcher {
    pub fn test(&self, declarations: PackageExternalDeclarations<'_>, is_monorepo: bool) -> bool {
        let has_dep = |dep: &str| -> bool {
            declarations.iter().any(|declaration| {
                let kind_matches = if is_monorepo {
                    !matches!(declaration.kind(), DependencyKind::Peer { .. })
                } else {
                    matches!(
                        declaration.kind(),
                        DependencyKind::Production | DependencyKind::Development
                    )
                };
                let name = if is_monorepo {
                    declaration.package_name()
                } else {
                    declaration.declaration_name()
                };
                kind_matches && name == dep
            })
        };

        match self.strategy {
            Strategy::All => self.dependencies.iter().all(|dep| has_dep(dep)),
            Strategy::Some => self.dependencies.iter().any(|dep| has_dep(dep)),
        }
    }
}

impl Slug {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn framework(&self) -> Option<&Framework> {
        let frameworks = get_frameworks().ok()?;
        frameworks
            .iter()
            .find(|framework| framework.slug.as_str() == self.as_str())
    }
}

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn infer_framework(
    declarations: PackageExternalDeclarations<'_>,
    is_monorepo: bool,
) -> Option<&'static Framework> {
    let frameworks = get_frameworks().ok()?;

    frameworks
        .iter()
        .find(|framework| framework.dependency_match.test(declarations, is_monorepo))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use test_case::test_case;
    use turborepo_repository::{
        external_resolution::{ExternalDeclaration, PackageExternalDeclarations},
        package_json::PackageJson,
    };

    use super::*;

    fn get_framework_by_slug(slug: &str) -> &Framework {
        get_frameworks()
            .expect("framework JSON failed to parse")
            .iter()
            .find(|framework| framework.slug.as_str() == slug)
            .expect("framework not found")
    }

    fn deps(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, String>> {
        Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// A declaration view for a package that depends on `next`, optionally
    /// locked to an exact resolved version.
    fn next_declarations(version: Option<&str>) -> Vec<ExternalDeclaration> {
        let declaration = ExternalDeclaration::new(
            "workspace",
            "next",
            "next",
            "^16.0.0",
            DependencyKind::Production,
        );
        vec![match version {
            Some(version) => declaration.with_resolved_version(version),
            None => declaration,
        }]
    }

    /// A declaration view for a package whose resolved versions are unknowable.
    fn unknown_version() -> PackageExternalDeclarations<'static> {
        PackageExternalDeclarations::new(&[], "workspace")
    }

    #[test_case(PackageJson::default(), None, true; "empty dependencies")]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("blitz", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("blitzjs")),
        true;
        "blitz"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("blitz", "*"), ("next", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("blitzjs")),
        true;
        "Order is preserved (returns blitz, not next)"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("next", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("nextjs")),
        true;
        "Finds next without blitz"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("solid-js", "*"), ("solid-start", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("solidstart")),
        true;
        "match all strategy works (solid)"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("nuxt", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("nuxtjs")),
        true;
        "match some strategy works (nuxt)"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("@remix-run/react", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("remix")),
        true;
        "match some strategy works (remix)"
    )]
    #[test_case(
        PackageJson {
                dependencies: deps(&[("react-scripts", "*")]),
                ..Default::default()
        },
        Some(get_framework_by_slug("create-react-app")),
        true;
        "match some strategy works (create-react-app)"
    )]
    #[test_case(
        PackageJson {
                            dependencies: Some(
                vec![("next", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
              ),
                            ..Default::default()
        },
        Some(get_framework_by_slug("nextjs")),
        false;
        "Finds next in non-monorepo"
    )]
    #[test_case(
        PackageJson {
                            dev_dependencies: Some(
                vec![("vite", "*")]
                    .into_iter()
                    .map(|(s1, s2)| (s1.to_string(), s2.to_string()))
                    .collect()
              ),
                            ..Default::default()
        },
        Some(get_framework_by_slug("vite")),
        false;
        "Finds vite in devDependencies in non-monorepo"
    )]
    #[test_case(PackageJson::default(), None, false; "empty dependencies in non-monorepo")]
    #[test_case(
        PackageJson {
                                dev_dependencies: deps(&[("vite", "*")]),
                                ..Default::default()
        },
        None,
        true;
        "devDependencies in package_json ignored in monorepo mode"
    )]
    #[test_case(
        PackageJson {
                                dependencies: deps(&[("solid-js", "*")]),
                dev_dependencies: deps(&[("solid-start", "*")]),
                                ..Default::default()
        },
        Some(get_framework_by_slug("solidstart")),
        false;
        "Strategy::All matches deps split across dependencies and devDependencies"
    )]
    #[test_case(
        PackageJson {
                                dev_dependencies: deps(&[("react-scripts", "*")]),
                                ..Default::default()
        },
        Some(get_framework_by_slug("create-react-app")),
        false;
        "Strategy::Some matches devDependency in non-monorepo"
    )]
    fn test_infer_framework(
        workspace_info: PackageJson,
        expected: Option<&Framework>,
        is_monorepo: bool,
    ) {
        let declarations =
            if is_monorepo {
                workspace_info
                    .dependencies
                    .iter()
                    .flatten()
                    .map(|(name, specifier)| {
                        ExternalDeclaration::new(
                            "workspace",
                            name,
                            name,
                            specifier,
                            DependencyKind::Production,
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                workspace_info
                    .dependencies
                    .iter()
                    .flatten()
                    .map(|(name, specifier)| (name, specifier, DependencyKind::Production))
                    .chain(
                        workspace_info.dev_dependencies.iter().flatten().map(
                            |(name, specifier)| (name, specifier, DependencyKind::Development),
                        ),
                    )
                    .map(|(name, specifier, kind)| {
                        ExternalDeclaration::new("workspace", name, name, specifier, kind)
                    })
                    .collect::<Vec<_>>()
            };
        let framework = infer_framework(
            PackageExternalDeclarations::new(&declarations, "workspace"),
            is_monorepo,
        );
        assert_eq!(framework, expected);
    }

    #[test]
    fn aliases_and_peer_declarations_preserve_framework_behavior() {
        let declarations = vec![
            ExternalDeclaration::new(
                "workspace",
                "next-alias",
                "next",
                "npm:next@latest",
                DependencyKind::Production,
            ),
            ExternalDeclaration::new(
                "workspace",
                "blitz",
                "blitz",
                "*",
                DependencyKind::Peer { optional: false },
            ),
        ];
        let view = PackageExternalDeclarations::new(&declarations, "workspace");

        assert_eq!(
            infer_framework(view, true),
            Some(get_framework_by_slug("nextjs"))
        );
        assert_eq!(infer_framework(view, false), None);
    }

    #[test]
    fn optional_declarations_preserve_single_package_behavior() {
        let declarations = vec![ExternalDeclaration::new(
            "workspace",
            "next",
            "next",
            "latest",
            DependencyKind::Optional,
        )];
        let view = PackageExternalDeclarations::new(&declarations, "workspace");

        assert_eq!(
            infer_framework(view, true),
            Some(get_framework_by_slug("nextjs"))
        );
        assert_eq!(infer_framework(view, false), None);
    }

    #[test]
    fn test_env_with_no_conditions() {
        let framework = get_framework_by_slug("nextjs");

        let env_at_execution_start = HashMap::new();
        let env_vars = framework.env(&env_at_execution_start, unknown_version());

        assert_eq!(
            env_vars,
            framework.env_wildcards.clone(),
            "Expected env_wildcards when no conditionals exist"
        );
    }

    #[test]
    fn test_env_with_legacy_deployment_id_below_version_floor() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        let declarations = next_declarations(Some("16.0.9"));
        let env_vars = framework.env(
            &env_at_execution_start,
            PackageExternalDeclarations::new(&declarations, "workspace"),
        );

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected VERCEL_DEPLOYMENT_ID below the version floor so Next.js 16.0.x keeps \
             hashing the deployment ID it baked into the build"
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

        let env_vars = framework.env(&env_at_execution_start, unknown_version());

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

        let env_vars = framework.env(&env_at_execution_start, unknown_version());

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
                    absent: None,
                },
                include: vec!["ADDITIONAL_ENV_VAR".to_string()],
                from_version: None,
                until_version: None,
            });
        }

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );
        env_at_execution_start.insert("ANOTHER_CONDITION".to_string(), "true".to_string());

        let env_vars = framework.env(&env_at_execution_start, unknown_version());

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
    fn test_env_unknown_version_keeps_legacy_deployment_id() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        // Without a resolved version the floor-gated conditional cannot be
        // proven applicable, while the ceiling still admits the legacy one.
        let env_vars = framework.env(&env_at_execution_start, unknown_version());

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected an unknown Next.js version to preserve legacy behavior"
        );
    }

    #[test]
    fn test_env_unknown_version_does_not_assume_modern_nextjs() {
        let framework = get_framework_by_slug("nextjs");

        // No NOW_BUILDER and no skew-protection flag: only a known version at
        // or above the floor may pull in NEXT_DEPLOYMENT_ID.
        let env_vars = framework.env(&HashMap::new(), unknown_version());

        assert_eq!(
            env_vars,
            framework.env_wildcards.clone(),
            "Expected unknown versions to stay out of the version-gated conditional"
        );
    }

    #[test]
    fn test_env_hashes_next_deployment_id_above_version_floor() {
        let framework = get_framework_by_slug("nextjs");

        let declarations = next_declarations(Some("16.1.0"));
        let env_vars = framework.env(
            &HashMap::new(),
            PackageExternalDeclarations::new(&declarations, "workspace"),
        );

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("NEXT_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected NEXT_DEPLOYMENT_ID outside Vercel's builder, where Next.js still reads it \
             at build time"
        );
    }

    #[test]
    fn test_env_replaces_legacy_deployment_id_above_version_floor() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        let declarations = next_declarations(Some("16.1.0"));
        let env_vars = framework.env(
            &env_at_execution_start,
            PackageExternalDeclarations::new(&declarations, "workspace"),
        );

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("NEXT_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected NEXT_DEPLOYMENT_ID to replace VERCEL_DEPLOYMENT_ID at and above the floor"
        );
    }

    #[test]
    fn test_env_skips_deployment_ids_in_vercel_builder() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert("NOW_BUILDER".to_string(), "1".to_string());
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        let declarations = next_declarations(Some("16.1.0"));
        let env_vars = framework.env(
            &env_at_execution_start,
            PackageExternalDeclarations::new(&declarations, "workspace"),
        );

        assert_eq!(
            env_vars,
            framework.env_wildcards.clone(),
            "Expected Vercel's builder to hash neither deployment ID once Next.js supplies it at \
             runtime"
        );
    }

    #[test]
    fn test_env_builder_flag_does_not_affect_legacy_deployment_id() {
        let framework = get_framework_by_slug("nextjs");

        let mut env_at_execution_start = HashMap::new();
        env_at_execution_start.insert("NOW_BUILDER".to_string(), "1".to_string());
        env_at_execution_start.insert(
            "VERCEL_SKEW_PROTECTION_ENABLED".to_string(),
            "1".to_string(),
        );

        let declarations = next_declarations(Some("16.0.9"));
        let env_vars = framework.env(
            &env_at_execution_start,
            PackageExternalDeclarations::new(&declarations, "workspace"),
        );

        let mut expected_vars = framework.env_wildcards.clone();
        expected_vars.push("VERCEL_DEPLOYMENT_ID".to_string());

        assert_eq!(
            env_vars, expected_vars,
            "Expected the builder flag to leave pre-floor behavior untouched"
        );
    }

    #[test]
    fn test_framework_version_bounds_are_valid_semver() {
        for framework in get_frameworks().expect("framework JSON failed to parse") {
            for conditional in framework.env_conditionals.iter().flatten() {
                for bound in [&conditional.from_version, &conditional.until_version]
                    .into_iter()
                    .flatten()
                {
                    assert!(
                        Version::parse(bound).is_ok(),
                        "{} has an unparseable version bound: {bound}",
                        framework.slug()
                    );
                }
            }
        }
    }

    #[test]
    fn test_framework_slug_roundtrip() {
        for framework in get_frameworks().expect("framework JSON failed to parse") {
            assert_eq!(Some(framework), framework.slug().framework());
        }
    }
}
