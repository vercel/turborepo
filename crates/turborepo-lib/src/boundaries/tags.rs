use std::collections::{HashMap, HashSet};

use turborepo_errors::Spanned;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{
    boundaries::{config::Rule, BoundariesDiagnostic, Error, Permissions, SecondaryDiagnostic},
    run::Run,
    turbo_json::TurboJson,
};

pub type ProcessedRulesMap = HashMap<String, ProcessedRule>;

pub struct ProcessedRule {
    dependencies: Option<ProcessedPermissions>,
    dependents: Option<ProcessedPermissions>,
}

impl From<Rule> for ProcessedRule {
    fn from(rule: Rule) -> Self {
        Self {
            dependencies: rule
                .dependencies
                .map(|dependencies| dependencies.into_inner().into()),
            dependents: rule
                .dependents
                .map(|dependents| dependents.into_inner().into()),
        }
    }
}

pub struct ProcessedPermissions {
    allow: Option<Spanned<HashSet<String>>>,
    deny: Option<Spanned<HashSet<String>>>,
}

impl From<Permissions> for ProcessedPermissions {
    fn from(permissions: Permissions) -> Self {
        Self {
            allow: permissions
                .allow
                .map(|allow| allow.map(|allow| allow.into_iter().flatten().collect())),
            deny: permissions
                .deny
                .map(|deny| deny.map(|deny| deny.into_iter().flatten().collect())),
        }
    }
}

impl Run {
    pub(crate) fn get_package_tags(&self) -> HashMap<PackageName, Spanned<Vec<Spanned<String>>>> {
        let mut package_tags = HashMap::new();
        let mut turbo_json_loader = self.turbo_json_loader();
        for (package, _) in self.pkg_dep_graph().packages() {
            if let Ok(TurboJson {
                tags: Some(tags), ..
            }) = turbo_json_loader.load(package)
            {
                package_tags.insert(package.clone(), tags.clone());
            }
        }

        package_tags
    }

    pub(crate) fn get_processed_rules_map(&self) -> Option<ProcessedRulesMap> {
        self.root_turbo_json()
            .boundaries
            .as_ref()
            .and_then(|boundaries| boundaries.tags.as_ref())
            .map(|tags| {
                tags.as_inner()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_inner().clone().into()))
                    .collect()
            })
    }

    /// Loops through the tags of a package that is related to `package_name`
    /// (i.e. either a dependency or a dependent) and checks if the tag is
    /// allowed or denied by the rules in `allow_list` and `deny_list`.
    fn validate_relation(
        &self,
        package_name: &PackageName,
        relation_package_name: &PackageName,
        tags: Option<&Spanned<Vec<Spanned<String>>>>,
        allow_list: Option<&Spanned<HashSet<String>>>,
        deny_list: Option<&Spanned<HashSet<String>>>,
    ) -> Result<Option<BoundariesDiagnostic>, Error> {
        // If there is no allow list, then we vacuously have a tag in the allow list
        let mut has_tag_in_allowlist = allow_list.is_none();
        let tags_span = tags.map(|tags| tags.to(())).unwrap_or_default();

        for tag in tags.into_iter().flatten().flatten() {
            if let Some(allow_list) = allow_list {
                if allow_list.contains(tag.as_inner()) {
                    has_tag_in_allowlist = true;
                }
            }

            if let Some(deny_list) = deny_list {
                if deny_list.contains(tag.as_inner()) {
                    let (span, text) = tag.span_and_text("turbo.json");
                    let deny_list_spanned = deny_list.to(());
                    let (deny_list_span, deny_list_text) =
                        deny_list_spanned.span_and_text("turbo.json");

                    return Ok(Some(BoundariesDiagnostic::DeniedTag {
                        source_package_name: package_name.clone(),
                        package_name: relation_package_name.clone(),
                        tag: tag.as_inner().to_string(),
                        span,
                        text,
                        secondary: [SecondaryDiagnostic::Denylist {
                            span: deny_list_span,
                            text: deny_list_text,
                        }],
                    }));
                }
            }
        }

        if !has_tag_in_allowlist {
            let (span, text) = tags_span.span_and_text("turbo.json");
            let help = span.is_none().then(|| {
                format!(
                    "`{}` doesn't any tags defined in its `turbo.json` file",
                    relation_package_name
                )
            });

            let allow_list_spanned = allow_list
                .map(|allow_list| allow_list.to(()))
                .unwrap_or_default();
            let (allow_list_span, allow_list_text) = allow_list_spanned.span_and_text("turbo.json");

            return Ok(Some(BoundariesDiagnostic::NoTagInAllowlist {
                source_package_name: package_name.clone(),
                package_name: relation_package_name.clone(),
                help,
                span,
                text,
                secondary: [SecondaryDiagnostic::Allowlist {
                    span: allow_list_span,
                    text: allow_list_text,
                }],
            }));
        }

        Ok(None)
    }

    pub(crate) fn check_package_tags(
        &self,
        pkg: PackageNode,
        current_package_tags: &Spanned<Vec<Spanned<String>>>,
        all_package_tags: &HashMap<PackageName, Spanned<Vec<Spanned<String>>>>,
        tags_rules: &ProcessedRulesMap,
    ) -> Result<Vec<BoundariesDiagnostic>, Error> {
        let mut diagnostics = Vec::new();
        for tag in current_package_tags.iter() {
            if let Some(rule) = tags_rules.get(tag.as_inner()) {
                if let Some(dependency_permissions) = &rule.dependencies {
                    for dependency in self.pkg_dep_graph().dependencies(&pkg) {
                        if matches!(dependency, PackageNode::Root) {
                            continue;
                        }
                        let dependency_tags = all_package_tags.get(dependency.as_package_name());
                        diagnostics.extend(self.validate_relation(
                            pkg.as_package_name(),
                            dependency.as_package_name(),
                            dependency_tags,
                            dependency_permissions.allow.as_ref(),
                            dependency_permissions.deny.as_ref(),
                        )?);
                    }
                }

                if let Some(dependent_permissions) = &rule.dependents {
                    for dependent in self.pkg_dep_graph().ancestors(&pkg) {
                        if matches!(dependent, PackageNode::Root) {
                            continue;
                        }
                        let dependent_tags = all_package_tags.get(dependent.as_package_name());
                        diagnostics.extend(self.validate_relation(
                            pkg.as_package_name(),
                            dependent.as_package_name(),
                            dependent_tags,
                            dependent_permissions.allow.as_ref(),
                            dependent_permissions.deny.as_ref(),
                        )?)
                    }
                }
            }
        }

        Ok(diagnostics)
    }
}
