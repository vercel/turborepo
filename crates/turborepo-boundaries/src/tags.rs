use std::collections::{HashMap, HashSet};

use miette::NamedSource;
use tracing::info_span;
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
    if let Some(deny_list) = deny_list
        && deny_list.contains(relation_package_name.as_str())
    {
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

    for tag in tags.into_iter().flatten().flatten() {
        if let Some(allow_list) = allow_list
            && allow_list.contains(tag.as_inner())
        {
            has_tag_in_allowlist = true;
        }

        if let Some(deny_list) = deny_list
            && deny_list.contains(tag.as_inner())
        {
            let (span, text) = tag.span_and_text("turbo.json");
            let deny_list_spanned = deny_list.to(());
            let (deny_list_span, deny_list_text) = deny_list_spanned.span_and_text("turbo.json");

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

/// Check tag rules against precomputed dependency/ancestor sets.
///
/// Unlike the previous version that called `ctx.pkg_dep_graph.dependencies()`
/// per invocation (triggering a full DFS each time), this takes the already-
/// computed sets to avoid redundant graph traversals when multiple tags share
/// the same package.
pub(crate) fn check_tag_with_cache<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    diagnostics: &mut Vec<BoundariesDiagnostic>,
    dependencies: Option<&ProcessedPermissions>,
    dependents: Option<&ProcessedPermissions>,
    pkg: &PackageNode,
    package_json: &PackageJson,
    cached_deps: &[&PackageNode],
    cached_ancestors: &[&PackageNode],
) -> Result<(), Error>
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    if let Some(dependency_permissions) = dependencies {
        for dependency in cached_deps {
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
        for dependent in cached_ancestors {
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

/// Returns true if any tag rule (either from the package's boundaries config
/// or from the global tag rules) needs dependency checking.
fn needs_dependencies<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    pkg: &PackageNode,
    current_package_tags: Option<&Spanned<Vec<Spanned<String>>>>,
    tags_rules: Option<&ProcessedRulesMap>,
) -> bool
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    let pkg_boundaries = ctx
        .turbo_json_provider
        .boundaries_config(pkg.as_package_name());
    if let Some(b) = pkg_boundaries
        && b.dependencies.is_some()
    {
        return true;
    }
    if let Some(rules) = tags_rules {
        for tag in current_package_tags.into_iter().flatten().flatten() {
            if let Some(rule) = rules.get(tag.as_inner())
                && rule.dependencies.is_some()
            {
                return true;
            }
        }
    }
    false
}

/// Returns true if any tag rule needs ancestor (dependent) checking.
fn needs_ancestors<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    pkg: &PackageNode,
    current_package_tags: Option<&Spanned<Vec<Spanned<String>>>>,
    tags_rules: Option<&ProcessedRulesMap>,
) -> bool
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    let pkg_boundaries = ctx
        .turbo_json_provider
        .boundaries_config(pkg.as_package_name());
    if let Some(b) = pkg_boundaries
        && b.dependents.is_some()
    {
        return true;
    }
    if let Some(rules) = tags_rules {
        for tag in current_package_tags.into_iter().flatten().flatten() {
            if let Some(rule) = rules.get(tag.as_inner())
                && rule.dependents.is_some()
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
fn needs_graph_traversal_for<G, T>(
    ctx: &BoundariesContext<'_, G, T>,
    pkg: &PackageNode,
    current_package_tags: Option<&Spanned<Vec<Spanned<String>>>>,
    tags_rules: Option<&ProcessedRulesMap>,
) -> (bool, bool)
where
    G: PackageGraphProvider,
    T: TurboJsonProvider,
{
    (
        needs_dependencies(ctx, pkg, current_package_tags, tags_rules),
        needs_ancestors(ctx, pkg, current_package_tags, tags_rules),
    )
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
    let _span = info_span!("check_package_tags", package = %pkg.as_package_name()).entered();
    let mut diagnostics = Vec::new();

    // Compute transitive dependencies and ancestors once, then reuse across
    // all tag rules for this package. Each call to `dependencies()` /
    // `ancestors()` does a full DFS — caching them here avoids O(tags * (V+E))
    // redundant traversals.
    let cached_deps: Vec<&PackageNode> =
        if needs_dependencies(ctx, &pkg, current_package_tags, tags_rules) {
            let _span =
                info_span!("compute_dependencies", package = %pkg.as_package_name()).entered();
            ctx.pkg_dep_graph.dependencies(&pkg).collect()
        } else {
            Vec::new()
        };

    let cached_ancestors: Vec<&PackageNode> =
        if needs_ancestors(ctx, &pkg, current_package_tags, tags_rules) {
            let _span = info_span!("compute_ancestors", package = %pkg.as_package_name()).entered();
            ctx.pkg_dep_graph.ancestors(&pkg).collect()
        } else {
            Vec::new()
        };

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

        check_tag_with_cache(
            ctx,
            &mut diagnostics,
            dependencies.as_ref(),
            dependents.as_ref(),
            &pkg,
            package_json,
            &cached_deps,
            &cached_ancestors,
        )?;
    }

    if let Some(tags_rules) = tags_rules {
        // We don't allow tags to share the same name as the package
        // because we allow package names to be used as a tag
        diagnostics.extend(check_if_package_name_is_tag(tags_rules, &pkg, package_json));

        for tag in current_package_tags.into_iter().flatten().flatten() {
            if let Some(rule) = tags_rules.get(tag.as_inner()) {
                check_tag_with_cache(
                    ctx,
                    &mut diagnostics,
                    rule.dependencies.as_ref(),
                    rule.dependents.as_ref(),
                    &pkg,
                    package_json,
                    &cached_deps,
                    &cached_ancestors,
                )?;
            }
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use turborepo_repository::{
        package_graph::{PackageInfo, PackageName, PackageNode},
        package_json::PackageJson,
    };

    use super::*;
    use crate::{BoundariesConfig, BoundariesContext, PackageGraphProvider, TurboJsonProvider};

    // Minimal mock graph that tracks packages, dependencies, and ancestors.
    struct MockGraph {
        packages: Vec<(PackageName, PackageInfo)>,
        deps: HashMap<PackageNode, Vec<PackageNode>>,
        ancestors: HashMap<PackageNode, Vec<PackageNode>>,
    }

    impl MockGraph {
        fn new() -> Self {
            Self {
                packages: Vec::new(),
                deps: HashMap::new(),
                ancestors: HashMap::new(),
            }
        }

        fn add_package(&mut self, name: &str) {
            let pkg_name = PackageName::Other(name.into());
            self.packages.push((
                pkg_name,
                PackageInfo {
                    package_json: PackageJson::default(),
                    package_json_path: turbopath::AnchoredSystemPathBuf::from_raw(format!(
                        "packages/{name}/package.json"
                    ))
                    .unwrap(),
                    unresolved_external_dependencies: None,
                    transitive_dependencies: None,
                },
            ));
        }

        fn add_dep(&mut self, from: &str, to: &str) {
            let from_node = PackageNode::Workspace(PackageName::Other(from.into()));
            let to_node = PackageNode::Workspace(PackageName::Other(to.into()));
            self.deps
                .entry(from_node.clone())
                .or_default()
                .push(to_node.clone());
            self.ancestors.entry(to_node).or_default().push(from_node);
        }
    }

    impl PackageGraphProvider for MockGraph {
        fn packages(&self) -> Box<dyn Iterator<Item = (&PackageName, &PackageInfo)> + '_> {
            Box::new(self.packages.iter().map(|(n, i)| (n, i)))
        }

        fn immediate_dependencies(&self, _node: &PackageNode) -> Option<HashSet<&PackageNode>> {
            None
        }

        fn dependencies(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
            match self.deps.get(node) {
                Some(deps) => Box::new(deps.iter()),
                None => Box::new(std::iter::empty()),
            }
        }

        fn ancestors(&self, node: &PackageNode) -> Box<dyn Iterator<Item = &PackageNode> + '_> {
            match self.ancestors.get(node) {
                Some(anc) => Box::new(anc.iter()),
                None => Box::new(std::iter::empty()),
            }
        }

        fn find_cycles(&self) -> Vec<Vec<PackageName>> {
            Vec::new()
        }
    }

    struct MockTurboJson {
        configs: HashMap<PackageName, BoundariesConfig>,
        tags: HashMap<PackageName, Spanned<Vec<Spanned<String>>>>,
    }

    impl MockTurboJson {
        fn new() -> Self {
            Self {
                configs: HashMap::new(),
                tags: HashMap::new(),
            }
        }

        fn set_boundaries(&mut self, pkg: &str, config: BoundariesConfig) {
            self.configs.insert(PackageName::Other(pkg.into()), config);
        }

        fn set_tags(&mut self, pkg: &str, tags: Vec<&str>) {
            let spanned_tags: Vec<Spanned<String>> =
                tags.into_iter().map(|t| Spanned::new(t.into())).collect();
            self.tags
                .insert(PackageName::Other(pkg.into()), Spanned::new(spanned_tags));
        }
    }

    impl TurboJsonProvider for MockTurboJson {
        fn has_turbo_json(&self, pkg: &PackageName) -> bool {
            self.configs.contains_key(pkg) || self.tags.contains_key(pkg)
        }

        fn boundaries_config(&self, pkg: &PackageName) -> Option<&BoundariesConfig> {
            self.configs.get(pkg)
        }

        fn package_tags(&self, pkg: &PackageName) -> Option<&Spanned<Vec<Spanned<String>>>> {
            self.tags.get(pkg)
        }

        fn implicit_dependencies(&self, _pkg: &PackageName) -> HashMap<String, Spanned<()>> {
            HashMap::new()
        }
    }

    fn make_permissions(allow: Option<Vec<&str>>, deny: Option<Vec<&str>>) -> Permissions {
        Permissions {
            allow: allow.map(|tags| {
                Spanned::new(tags.into_iter().map(|t| Spanned::new(t.into())).collect())
            }),
            deny: deny.map(|tags| {
                Spanned::new(tags.into_iter().map(|t| Spanned::new(t.into())).collect())
            }),
        }
    }

    fn make_repo_root() -> turbopath::AbsoluteSystemPathBuf {
        #[cfg(unix)]
        {
            turbopath::AbsoluteSystemPathBuf::new("/tmp/test-repo").unwrap()
        }
        #[cfg(windows)]
        {
            turbopath::AbsoluteSystemPathBuf::new("C:\\tmp\\test-repo").unwrap()
        }
    }

    // -- needs_dependencies / needs_ancestors tests --

    #[test]
    fn needs_traversal_false_when_no_rules() {
        let graph = MockGraph::new();
        let turbo_json = MockTurboJson::new();
        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };
        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));

        let (need_deps, need_anc) = needs_graph_traversal_for(&ctx, &pkg, None, None);
        assert!(!need_deps);
        assert!(!need_anc);
    }

    #[test]
    fn needs_traversal_true_from_package_boundaries_config() {
        let graph = MockGraph::new();
        let mut turbo_json = MockTurboJson::new();
        turbo_json.set_boundaries(
            "pkg-a",
            BoundariesConfig {
                dependencies: Some(Spanned::new(make_permissions(Some(vec!["allowed"]), None))),
                dependents: Some(Spanned::new(make_permissions(None, Some(vec!["denied"])))),
                ..Default::default()
            },
        );
        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };
        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));

        let (need_deps, need_anc) = needs_graph_traversal_for(&ctx, &pkg, None, None);
        assert!(
            need_deps,
            "should need deps when boundaries config has dependencies"
        );
        assert!(
            need_anc,
            "should need ancestors when boundaries config has dependents"
        );
    }

    #[test]
    fn needs_traversal_true_from_tag_rules() {
        let graph = MockGraph::new();
        let turbo_json = MockTurboJson::new();
        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };
        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));

        let tag_rules: ProcessedRulesMap = [(
            "my-tag".into(),
            ProcessedRule {
                span: Spanned::new(()),
                dependencies: Some(ProcessedPermissions {
                    allow: None,
                    deny: None,
                }),
                dependents: None,
            },
        )]
        .into();

        let tags = Spanned::new(vec![Spanned::new("my-tag".into())]);

        let (need_deps, need_anc) =
            needs_graph_traversal_for(&ctx, &pkg, Some(&tags), Some(&tag_rules));
        assert!(need_deps, "should need deps when tag rule has dependencies");
        assert!(
            !need_anc,
            "should not need ancestors when no dependents rule"
        );
    }

    #[test]
    fn needs_traversal_only_dependents_from_tag_rules() {
        let graph = MockGraph::new();
        let turbo_json = MockTurboJson::new();
        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };
        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));

        let tag_rules: ProcessedRulesMap = [(
            "my-tag".into(),
            ProcessedRule {
                span: Spanned::new(()),
                dependencies: None,
                dependents: Some(ProcessedPermissions {
                    allow: None,
                    deny: None,
                }),
            },
        )]
        .into();

        let tags = Spanned::new(vec![Spanned::new("my-tag".into())]);

        let (need_deps, need_anc) =
            needs_graph_traversal_for(&ctx, &pkg, Some(&tags), Some(&tag_rules));
        assert!(!need_deps);
        assert!(need_anc);
    }

    // -- DFS caching / check_package_tags tests --

    #[test]
    fn cached_deps_used_across_multiple_tag_rules() {
        // Graph: pkg-a -> pkg-b, pkg-a -> pkg-c
        // pkg-b has tag "lib", pkg-c has tag "util"
        // pkg-a has two tags: "tag1" (allow deps with "lib") and "tag2" (allow deps
        // with "util") Both rules should see the same cached dependency set.
        let mut graph = MockGraph::new();
        graph.add_package("pkg-a");
        graph.add_package("pkg-b");
        graph.add_package("pkg-c");
        graph.add_dep("pkg-a", "pkg-b");
        graph.add_dep("pkg-a", "pkg-c");

        let mut turbo_json = MockTurboJson::new();
        turbo_json.set_tags("pkg-a", vec!["tag1", "tag2"]);
        turbo_json.set_tags("pkg-b", vec!["lib"]);
        turbo_json.set_tags("pkg-c", vec!["util"]);

        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };

        // tag1 allows only "lib", tag2 allows only "util"
        let tag_rules: ProcessedRulesMap = [
            (
                "tag1".into(),
                ProcessedRule {
                    span: Spanned::new(()),
                    dependencies: Some(ProcessedPermissions {
                        allow: Some(Spanned::new(["lib".into()].into())),
                        deny: None,
                    }),
                    dependents: None,
                },
            ),
            (
                "tag2".into(),
                ProcessedRule {
                    span: Spanned::new(()),
                    dependencies: Some(ProcessedPermissions {
                        allow: Some(Spanned::new(["util".into()].into())),
                        deny: None,
                    }),
                    dependents: None,
                },
            ),
        ]
        .into();

        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));
        let pkg_json = PackageJson::default();
        let tags = turbo_json.package_tags(&PackageName::Other("pkg-a".into()));

        let diagnostics = check_package_tags(&ctx, pkg, &pkg_json, tags, Some(&tag_rules)).unwrap();

        // tag1 allows "lib": pkg-b has "lib" (ok), pkg-c has "util" (violation)
        // tag2 allows "util": pkg-c has "util" (ok), pkg-b has "lib" (violation)
        // So we expect 2 NoTagInAllowlist diagnostics
        let allowlist_violations: Vec<_> = diagnostics
            .iter()
            .filter(|d| matches!(d, BoundariesDiagnostic::NoTagInAllowlist { .. }))
            .collect();
        assert_eq!(
            allowlist_violations.len(),
            2,
            "expected 2 allowlist violations from 2 tag rules seeing same cached deps, got: {}",
            allowlist_violations.len()
        );
    }

    #[test]
    fn no_dfs_when_no_rules_need_it() {
        // If no rules require dependency/ancestor checks, the DFS should be skipped.
        // We verify by giving pkg-a dependencies but no rules that would trigger
        // checking.
        let mut graph = MockGraph::new();
        graph.add_package("pkg-a");
        graph.add_package("pkg-b");
        graph.add_dep("pkg-a", "pkg-b");

        let mut turbo_json = MockTurboJson::new();
        turbo_json.set_tags("pkg-a", vec!["my-tag"]);

        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };

        // Tag rule has no dependencies or dependents fields
        let tag_rules: ProcessedRulesMap = [(
            "my-tag".into(),
            ProcessedRule {
                span: Spanned::new(()),
                dependencies: None,
                dependents: None,
            },
        )]
        .into();

        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));
        let pkg_json = PackageJson::default();
        let tags = turbo_json.package_tags(&PackageName::Other("pkg-a".into()));

        let diagnostics = check_package_tags(&ctx, pkg, &pkg_json, tags, Some(&tag_rules)).unwrap();

        // No dependency/dependent rules → no violations possible from tag checking
        let tag_violations: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    BoundariesDiagnostic::NoTagInAllowlist { .. }
                        | BoundariesDiagnostic::DeniedTag { .. }
                )
            })
            .collect();
        assert!(
            tag_violations.is_empty(),
            "expected no tag violations when rules don't check deps/dependents"
        );
    }

    #[test]
    fn check_tag_with_cache_skips_root_nodes() {
        let graph = MockGraph::new();
        let turbo_json = MockTurboJson::new();
        let repo_root = make_repo_root();
        let filtered = HashSet::new();
        let ctx = BoundariesContext {
            repo_root: &repo_root,
            pkg_dep_graph: &graph,
            turbo_json_provider: &turbo_json,
            root_boundaries_config: None,
            filtered_pkgs: &filtered,
        };

        let pkg = PackageNode::Workspace(PackageName::Other("pkg-a".into()));
        let pkg_json = PackageJson::default();

        let perms = ProcessedPermissions {
            allow: Some(Spanned::new(["allowed-tag".into()].into())),
            deny: None,
        };

        // Include Root in the cached deps — it should be skipped
        let root = PackageNode::Root;
        let cached_deps = vec![&root];
        let mut diagnostics = Vec::new();

        check_tag_with_cache(
            &ctx,
            &mut diagnostics,
            Some(&perms),
            None,
            &pkg,
            &pkg_json,
            &cached_deps,
            &[],
        )
        .unwrap();

        assert!(
            diagnostics.is_empty(),
            "Root node should be skipped, producing no diagnostics"
        );
    }
}
