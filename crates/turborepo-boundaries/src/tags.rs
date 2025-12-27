use std::collections::{HashMap, HashSet};

use miette::NamedSource;
use turborepo_errors::Spanned;
use turborepo_repository::{
    package_graph::{PackageName, PackageNode},
    package_json::PackageJson,
};

use crate::{
    BoundariesContext, BoundariesDiagnostic, Error, PackageGraphProvider, SecondaryDiagnostic,
    TurboJsonProvider,
    config::{Permissions, Rule},
};

pub type ProcessedRulesMap = HashMap<String, ProcessedRule>;

pub struct ProcessedRule {
    span: Spanned<()>,
    pub dependencies: Option<ProcessedPermissions>,
    pub dependents: Option<ProcessedPermissions>,
}

impl From<Spanned<Rule>> for ProcessedRule {
    fn from(rule: Spanned<Rule>) -> Self {
        let (rule, span) = rule.split();
        Self {
            span,
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
    pub allow: Option<Spanned<HashSet<String>>>,
    pub deny: Option<Spanned<HashSet<String>>>,
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

/// Loops through the tags of a package that is related to `package_name`
/// (i.e. either a dependency or a dependent) and checks if the tag is
/// allowed or denied by the rules in `allow_list` and `deny_list`.
fn validate_relation<G, T>(
    _ctx: &BoundariesContext<'_, G, T>,
    package_name: &PackageName,
    package_json: &PackageJson,
    relation_package_name: &PackageName,
    tags: Option<&Spanned<Vec<Spanned<String>>>>,
    allow_list: Option<&Spanned<HashSet<String>>>,
    deny_list: Option<&Spanned<HashSet<String>>>,
) -> Result<Option<BoundariesDiagnostic>, Error>
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    // We allow "punning" the package name as a tag, so if the allow list contains
    // the package name, then we have a tag in the allow list
    // Likewise, if the allow list is empty, then we vacuously have a tag in the
    // allow list
    let mut has_tag_in_allowlist =
        allow_list.is_none_or(|allow_list| allow_list.contains(relation_package_name.as_str()));
    let tags_span = tags.map(|tags| tags.to(())).unwrap_or_default();
    if let Some(deny_list) = deny_list {
        if deny_list.contains(relation_package_name.as_str()) {
            let (span, text) = package_json
                .name
                .as_ref()
                .map(|name| name.span_and_text("turbo.json"))
                .unwrap_or_else(|| (None, NamedSource::new("package.json", String::new())));
            let deny_list_spanned = deny_list.to(());
            let (deny_list_span, deny_list_text) = deny_list_spanned.span_and_text("turbo.json");

            return Ok(Some(BoundariesDiagnostic::DeniedTag {
                source_package_name: package_name.clone(),
                package_name: relation_package_name.clone(),
                tag: relation_package_name.to_string(),
                span,
                text,
                secondary: [SecondaryDiagnostic::Denylist {
                    span: deny_list_span,
                    text: deny_list_text,
                }],
            }));
        }
    }

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
            format!("`{relation_package_name}` doesn't any tags defined in its `turbo.json` file")
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

pub(crate) fn check_tag<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    diagnostics: &mut Vec<BoundariesDiagnostic>,
    dependencies: Option<&ProcessedPermissions>,
    dependents: Option<&ProcessedPermissions>,
    pkg: &PackageNode,
    package_json: &PackageJson,
) -> Result<(), Error>
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    if let Some(dependency_permissions) = dependencies {
        for dependency in ctx.pkg_dep_graph.dependencies(pkg) {
            if matches!(dependency, PackageNode::Root) {
                continue;
            }

            let dependency_tags = ctx
                .turbo_json_provider
                .package_tags(dependency.as_package_name());

            diagnostics.extend(validate_relation(
                ctx,
                pkg.as_package_name(),
                package_json,
                dependency.as_package_name(),
                dependency_tags,
                dependency_permissions.allow.as_ref(),
                dependency_permissions.deny.as_ref(),
            )?);
        }
    }

    if let Some(dependent_permissions) = dependents {
        for dependent in ctx.pkg_dep_graph.ancestors(pkg) {
            if matches!(dependent, PackageNode::Root) {
                continue;
            }
            let dependent_tags = ctx
                .turbo_json_provider
                .package_tags(dependent.as_package_name());
            diagnostics.extend(validate_relation(
                ctx,
                pkg.as_package_name(),
                package_json,
                dependent.as_package_name(),
                dependent_tags,
                dependent_permissions.allow.as_ref(),
                dependent_permissions.deny.as_ref(),
            )?)
        }
    }

    Ok(())
}

fn check_if_package_name_is_tag(
    tags_rules: &ProcessedRulesMap,
    pkg: &PackageNode,
    package_json: &PackageJson,
) -> Option<BoundariesDiagnostic> {
    let rule = tags_rules.get(pkg.as_package_name().as_str())?;
    let (tag_span, tag_text) = rule.span.span_and_text("turbo.json");
    let (package_span, package_text) = package_json
        .name
        .as_ref()
        .map(|name| name.span_and_text("package.json"))
        .unwrap_or_else(|| (None, NamedSource::new("package.json", "".into())));
    Some(BoundariesDiagnostic::TagSharesPackageName {
        tag: pkg.as_package_name().to_string(),
        package: pkg.as_package_name().to_string(),
        tag_span,
        tag_text,
        secondary: [SecondaryDiagnostic::PackageDefinedHere {
            package: pkg.as_package_name().to_string(),
            package_span,
            package_text,
        }],
    })
}

pub(crate) fn check_package_tags<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    pkg: PackageNode,
    package_json: &PackageJson,
    current_package_tags: Option<&Spanned<Vec<Spanned<String>>>>,
    tags_rules: Option<&ProcessedRulesMap>,
) -> Result<Vec<BoundariesDiagnostic>, Error>
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    let mut diagnostics = Vec::new();

    // Load boundaries config for this package (matches original behavior)
    let package_boundaries = ctx
        .turbo_json_provider
        .boundaries_config(pkg.as_package_name());

    if let Some(boundaries) = package_boundaries {
        if let Some(tags) = &boundaries.tags {
            let (span, text) = tags.span_and_text("turbo.json");
            diagnostics.push(BoundariesDiagnostic::PackageBoundariesHasTags { span, text });
        }
        let dependencies = boundaries
            .dependencies
            .clone()
            .map(|deps| deps.into_inner().into());
        let dependents = boundaries
            .dependents
            .clone()
            .map(|deps| deps.into_inner().into());

        check_tag(
            ctx,
            &mut diagnostics,
            dependencies.as_ref(),
            dependents.as_ref(),
            &pkg,
            package_json,
        )?;
    }

    if let Some(tags_rules) = tags_rules {
        // We don't allow tags to share the same name as the package
        // because we allow package names to be used as a tag
        diagnostics.extend(check_if_package_name_is_tag(tags_rules, &pkg, package_json));

        for tag in current_package_tags.into_iter().flatten().flatten() {
            if let Some(rule) = tags_rules.get(tag.as_inner()) {
                check_tag(
                    ctx,
                    &mut diagnostics,
                    rule.dependencies.as_ref(),
                    rule.dependents.as_ref(),
                    &pkg,
                    package_json,
                )?;
            }
        }
    }

    Ok(diagnostics)
}
