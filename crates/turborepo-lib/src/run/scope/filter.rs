use std::{
    collections::{HashMap, HashSet},
    path::Path,
    str::FromStr,
};

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::ChangeMapError,
    package_graph::{self, PackageGraph, PackageName},
};
use turborepo_scm::SCM;
use wax::Program;

use super::{
    change_detector::GitChangeDetector,
    simple_glob::{Match, SimpleGlob},
    target_selector::{GitRange, InvalidSelectorError, TargetSelector},
};
use crate::{
    global_deps_package_change_mapper, run::scope::change_detector::ScopeChangeDetector,
    turbo_json::TurboJson,
};

pub struct PackageInference {
    package_name: Option<String>,
    directory_root: AnchoredSystemPathBuf,
}

impl PackageInference {
    // calculate, based on the directory that global turbo was invoked in,
    // the pieces of a filter spec that we will infer. If turbo was invoked
    // somewhere between the root and packages, scope turbo invocations to the
    // packages below where turbo was invoked. If turbo was invoked at or within
    // a particular package, scope the turbo invocation to just that package.
    pub fn calculate(
        turbo_root: &AbsoluteSystemPath,
        pkg_inference_path: &AnchoredSystemPathBuf,
        pkg_graph: &PackageGraph,
    ) -> Self {
        debug!(
            "Using {} as a basis for selecting packages",
            pkg_inference_path
        );
        let full_inference_path = turbo_root.resolve(pkg_inference_path);
        for (workspace_name, workspace_entry) in pkg_graph.packages() {
            let pkg_path = turbo_root.resolve(workspace_entry.package_path());
            let inferred_path_is_below = pkg_path.contains(&full_inference_path);
            // We skip over the root package as the inferred path will always be below it
            if inferred_path_is_below && (&pkg_path as &AbsoluteSystemPath) != turbo_root {
                // set both. The user might have set a parent directory filter,
                // in which case we *should* fail to find any packages, but we should
                // do so in a consistent manner
                return Self {
                    package_name: Some(workspace_name.to_string()),
                    directory_root: workspace_entry.package_path().to_owned(),
                };
            }
            let inferred_path_is_between_root_and_pkg = full_inference_path.contains(&pkg_path);
            if inferred_path_is_between_root_and_pkg {
                // we've found *some* package below our inference directory. We can stop now and
                // conclude that we're looking for all packages in a
                // subdirectory
                break;
            }
        }
        Self {
            package_name: None,
            directory_root: pkg_inference_path.to_owned(),
        }
    }

    pub fn apply(&self, selector: &mut TargetSelector) {
        // if the name pattern is provided, do not attempt inference
        if !selector.name_pattern.is_empty() {
            return;
        };

        if let Some(name) = &self.package_name {
            selector.name_pattern.clone_from(name);
        }

        if let Some(parent_dir) = selector.parent_dir.as_deref() {
            let repo_relative_parent_dir = self.directory_root.join(parent_dir);
            let clean_parent_dir = path_clean::clean(Path::new(repo_relative_parent_dir.as_path()))
                .into_os_string()
                .into_string()
                .expect("path was valid utf8 before cleaning");
            selector.parent_dir = Some(
                AnchoredSystemPathBuf::try_from(clean_parent_dir.as_str())
                    .expect("path wasn't absolute before cleaning"),
            );
        } else if self.package_name.is_none() {
            // fallback: the user didn't set a parent directory and we didn't find a single
            // package, so use the directory we inferred and select all subdirectories
            let mut parent_dir = self.directory_root.clone();
            parent_dir.push("**");
            selector.parent_dir = Some(parent_dir);
        }
    }
}

pub struct FilterResolver<'a, T: GitChangeDetector> {
    pkg_graph: &'a PackageGraph,
    turbo_root: &'a AbsoluteSystemPath,
    inference: Option<PackageInference>,
    scm: &'a SCM,
    change_detector: T,
}

impl<'a> FilterResolver<'a, ScopeChangeDetector<'a>> {
    pub(crate) fn new(
        opts: &'a super::ScopeOpts,
        pkg_graph: &'a PackageGraph,
        turbo_root: &'a AbsoluteSystemPath,
        inference: Option<PackageInference>,
        scm: &'a SCM,
        root_turbo_json: &'a TurboJson,
    ) -> Result<Self, ResolutionError> {
        let global_deps = opts
            .global_deps
            .iter()
            .map(|s| s.as_str())
            .chain(root_turbo_json.global_deps.iter().map(|s| s.as_str()));

        let change_detector =
            ScopeChangeDetector::new(turbo_root, scm, pkg_graph, global_deps, vec![])?;

        Ok(Self::new_with_change_detector(
            pkg_graph,
            turbo_root,
            inference,
            scm,
            change_detector,
        ))
    }
}

impl<'a, T: GitChangeDetector> FilterResolver<'a, T> {
    pub(crate) fn new_with_change_detector(
        pkg_graph: &'a PackageGraph,
        turbo_root: &'a AbsoluteSystemPath,
        inference: Option<PackageInference>,
        scm: &'a SCM,
        change_detector: T,
    ) -> Self {
        Self {
            pkg_graph,
            turbo_root,
            inference,
            scm,
            change_detector,
        }
    }

    /// Resolves a set of filter patterns into a set of packages,
    /// based on the current state of the workspace. The result is
    /// guaranteed to be a subset of the packages in the workspace,
    /// and non-empty. If the filter is empty, none of the packages
    /// in the workspace will be returned.
    ///
    /// It applies the following rules:
    pub(crate) fn resolve(
        &self,
        affected: &Option<(Option<String>, String)>,
        patterns: &[String],
    ) -> Result<(HashSet<PackageName>, bool), ResolutionError> {
        // inference is None only if we are in the root
        let is_all_packages = patterns.is_empty() && self.inference.is_none() && affected.is_none();

        let filter_patterns = if is_all_packages {
            // return all packages in the workspace
            self.pkg_graph
                .packages()
                .filter(|(name, _)| matches!(name, PackageName::Other(_)))
                .map(|(name, _)| name.to_owned())
                .collect()
        } else {
            self.get_packages_from_patterns(affected, patterns)?
        };

        Ok((filter_patterns, is_all_packages))
    }

    fn get_packages_from_patterns(
        &self,
        affected: &Option<(Option<String>, String)>,
        patterns: &[String],
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let mut selectors = patterns
            .iter()
            .map(|pattern| TargetSelector::from_str(pattern))
            .collect::<Result<Vec<_>, _>>()?;

        if let Some((from_ref, to_ref)) = affected {
            selectors.push(TargetSelector {
                git_range: Some(GitRange {
                    from_ref: from_ref.clone(),
                    to_ref: Some(to_ref.to_string()),
                    include_uncommitted: true,
                    allow_unknown_objects: true,
                }),
                ..Default::default()
            });
        }

        self.get_filtered_packages(selectors)
    }

    fn get_filtered_packages(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let (_prod_selectors, all_selectors) = self
            .apply_inference(selectors)
            .into_iter()
            .partition::<Vec<_>, _>(|t| t.follow_prod_deps_only);

        if !all_selectors.is_empty() {
            self.filter_graph(all_selectors)
        } else {
            Ok(Default::default())
        }
    }

    fn apply_inference(&self, selectors: Vec<TargetSelector>) -> Vec<TargetSelector> {
        let inference = match self.inference {
            Some(ref inference) => inference,
            None => return selectors,
        };

        // if there is no selector provided, synthesize one
        let mut selectors = if selectors.is_empty() {
            vec![Default::default()]
        } else {
            selectors
        };

        for selector in &mut selectors {
            inference.apply(selector);
        }

        selectors
    }

    fn filter_graph(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let (include_selectors, exclude_selectors) =
            selectors.into_iter().partition::<Vec<_>, _>(|t| !t.exclude);

        let mut include = if !include_selectors.is_empty() {
            // TODO: add telemetry for each selector
            self.filter_graph_with_selectors(include_selectors)?
        } else {
            self.pkg_graph
                .packages()
                // todo: a type-level way of dealing with non-root packages
                .filter(|(name, _)| !PackageName::Root.eq(name)) // the root package has to be explicitly included
                .map(|(name, _)| name.to_owned())
                .collect()
        };

        let exclude = self.filter_graph_with_selectors(exclude_selectors)?;

        include.retain(|i| !exclude.contains(i));

        Ok(include)
    }

    fn filter_graph_with_selectors(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let mut unmatched_selectors = Vec::new();
        let mut walked_dependencies = HashSet::new();
        let mut walked_dependents = HashSet::new();
        let mut walked_dependent_dependencies = HashSet::new();
        let mut cherry_picked_packages = HashSet::new();

        for selector in selectors {
            let selector_packages = self.filter_graph_with_selector(&selector)?;

            if selector_packages.is_empty() {
                unmatched_selectors.push(selector);
                continue;
            }

            for package in selector_packages {
                let node = package_graph::PackageNode::Workspace(package.clone());

                if selector.include_dependencies {
                    let dependencies = self.pkg_graph.dependencies(&node);
                    let dependencies = dependencies
                        .iter()
                        .filter(|node| !matches!(node, package_graph::PackageNode::Root))
                        .map(|i| i.as_package_name().to_owned())
                        .collect::<Vec<_>>();

                    // flatmap through the option, the set, and then the optional package name
                    walked_dependencies.extend(dependencies);
                }

                if selector.include_dependents {
                    let dependents = self.pkg_graph.ancestors(&node);
                    for dependent in dependents.iter().map(|i| i.as_package_name()) {
                        walked_dependents.insert(dependent.clone());

                        // get the dependent's dependencies
                        if selector.include_dependencies {
                            let dependent_node =
                                package_graph::PackageNode::Workspace(dependent.to_owned());

                            let dependent_dependencies =
                                self.pkg_graph.dependencies(&dependent_node);

                            let dependent_dependencies = dependent_dependencies
                                .iter()
                                .filter(|node| !matches!(node, package_graph::PackageNode::Root))
                                .map(|i| i.as_package_name().to_owned())
                                .collect::<HashSet<_>>();

                            walked_dependent_dependencies.extend(dependent_dependencies);
                        }
                    }
                }

                if (selector.include_dependents || selector.include_dependencies)
                    && !selector.exclude_self
                {
                    // if we are including dependents or dependencies, and we are not excluding
                    // ourselves, then we should add ourselves to the list of packages
                    walked_dependencies.insert(package);
                } else if !selector.include_dependencies && !selector.include_dependents {
                    // if we are neither including dependents or dependencies, then
                    // add  to the list of cherry picked packages
                    cherry_picked_packages.insert(package);
                }
            }
        }

        let mut all_packages = HashSet::new();
        all_packages.extend(walked_dependencies);
        all_packages.extend(walked_dependents);
        all_packages.extend(walked_dependent_dependencies);
        all_packages.extend(cherry_picked_packages);

        Ok(all_packages)
    }

    fn filter_graph_with_selector(
        &self,
        selector: &TargetSelector,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        if selector.match_dependencies {
            self.filter_subtrees_with_selector(selector)
        } else {
            self.filter_nodes_with_selector(selector)
        }
    }

    /// returns the set of nodes where the node or any of its dependencies match
    /// the selector.
    ///
    /// Example:
    /// a -> b -> c
    /// a -> d
    ///
    /// filter(b) = {a, b, c}
    /// filter(d) = {a, d}
    fn filter_subtrees_with_selector(
        &self,
        selector: &TargetSelector,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let mut entry_packages = HashSet::new();

        for (name, info) in self.pkg_graph.packages() {
            if let Some(parent_dir) = selector.parent_dir.as_deref() {
                let path = parent_dir.to_unix();
                let parent_dir_matcher = wax::Glob::new(path.as_str())?;
                let matches = parent_dir_matcher.is_match(info.package_path().as_path());

                if matches {
                    entry_packages.insert(name.to_owned());
                }
            } else {
                entry_packages.insert(name.to_owned());
            }
        }

        // if we have a filter, use it to filter the entry packages
        let filtered_entry_packages = if !selector.name_pattern.is_empty() {
            match_package_names(&selector.name_pattern, entry_packages)?
        } else {
            entry_packages
        };

        let mut roots = HashSet::new();
        let mut matched = HashSet::new();
        let changed_packages = if let Some(git_range) = selector.git_range.as_ref() {
            self.packages_changed_in_range(git_range)?
        } else {
            HashSet::new()
        };

        for package in filtered_entry_packages {
            if matched.contains(&package) {
                roots.insert(package);
                continue;
            }

            let workspace_node = package_graph::PackageNode::Workspace(package.clone());
            let dependencies = self.pkg_graph.dependencies(&workspace_node);

            for changed_package in &changed_packages {
                if !selector.exclude_self && package.eq(changed_package) {
                    roots.insert(package);
                    break;
                }

                let changed_node =
                    package_graph::PackageNode::Workspace(changed_package.to_owned());

                if dependencies.contains(&changed_node) {
                    roots.insert(package.clone());
                    matched.insert(package);
                    break;
                }
            }
        }

        Ok(roots)
    }

    fn filter_nodes_with_selector(
        &self,
        selector: &TargetSelector,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let mut entry_packages = HashSet::new();
        let mut selector_valid = false;

        let parent_dir_unix = selector.parent_dir.as_deref().map(|path| path.to_unix());
        let parent_dir_globber = parent_dir_unix
            .as_deref()
            .map(|path| {
                wax::Glob::new(path.as_str()).map_err(|err| ResolutionError::InvalidDirectoryGlob {
                    glob: path.as_str().to_string(),
                    err: Box::new(err),
                })
            })
            .transpose()?;

        if let Some(globber) = parent_dir_globber.clone() {
            let (base, _) = globber.partition();
            // wax takes a unix-like glob, but partition will return a system path
            // TODO: it would be more proper to use
            // `AnchoredSystemPathBuf::from_system_path` but that function
            // doesn't allow leading `.` or `..`.
            let base = AnchoredSystemPathBuf::from_raw(
                base.to_str().expect("glob base should be valid utf8"),
            )
            .expect("partitioned glob gave absolute path");
            // need to join this with globbing's current dir :)
            let path = self.turbo_root.resolve(&base);
            if !path.exists() {
                return Err(ResolutionError::DirectoryDoesNotExist(path));
            }
        }

        if let Some(git_range) = selector.git_range.as_ref() {
            selector_valid = true;
            let changed_packages = self.packages_changed_in_range(git_range)?;
            let package_path_lookup = self
                .pkg_graph
                .packages()
                .map(|(name, entry)| (name, entry.package_path()))
                .collect::<HashMap<_, _>>();

            for package in changed_packages {
                if let Some(parent_dir_globber) = parent_dir_globber.as_ref() {
                    if package == PackageName::Root {
                        // The root package changed, only add it if
                        // the parentDir is equivalent to the root
                        if parent_dir_globber.matched(&Path::new(".").into()).is_some() {
                            entry_packages.insert(package);
                        }
                    } else {
                        let path = package_path_lookup
                            .get(&package)
                            .ok_or(ResolutionError::MissingPackageInfo(package.to_string()))?;

                        if parent_dir_globber.is_match(path.as_path()) {
                            entry_packages.insert(package);
                        }
                    }
                } else {
                    entry_packages.insert(package);
                }
            }
        } else if let Some((parent_dir, parent_dir_globber)) = selector
            .parent_dir
            .as_deref()
            .zip(parent_dir_globber.as_ref())
        {
            selector_valid = true;
            if parent_dir == &*AnchoredSystemPathBuf::from_raw(".").expect("valid anchored") {
                entry_packages.insert(PackageName::Root);
            } else {
                let packages = self.pkg_graph.packages();
                for (name, _) in packages.filter(|(_name, info)| {
                    let path = info.package_path().as_path();
                    parent_dir_globber.is_match(path)
                }) {
                    entry_packages.insert(name.to_owned());
                }
            }
        }

        if !selector.name_pattern.is_empty() {
            if !selector_valid {
                entry_packages = self.match_package_names_to_vertices(
                    &selector.name_pattern,
                    self.pkg_graph
                        .packages()
                        .map(|(name, _)| name.to_owned())
                        .collect(),
                )?;
                selector_valid = true;
            } else {
                entry_packages = match_package_names(&selector.name_pattern, entry_packages)?;
            }
        }

        // if neither a name pattern, parent dir, or from ref is provided, then
        // the selector is invalid
        if !selector_valid {
            Err(ResolutionError::InvalidSelector(
                InvalidSelectorError::InvalidSelector(selector.raw.clone()),
            ))
        } else {
            Ok(entry_packages)
        }
    }

    fn packages_changed_in_range(
        &self,
        git_range: &GitRange,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        self.change_detector.changed_packages(
            git_range.from_ref.as_deref(),
            git_range.to_ref.as_deref(),
            git_range.include_uncommitted,
            git_range.allow_unknown_objects,
        )
    }

    fn match_package_names_to_vertices(
        &self,
        name_pattern: &str,
        mut entry_packages: HashSet<PackageName>,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        // add the root package to the entry packages
        entry_packages.insert(PackageName::Root);

        match_package_names(name_pattern, entry_packages)
    }
}

/// match the provided name pattern against the provided set of packages
/// and return the set of packages that match the pattern
///
/// the pattern is normalized, replacing `\*` with `.*`
fn match_package_names(
    name_pattern: &str,
    mut entry_packages: HashSet<PackageName>,
) -> Result<HashSet<PackageName>, ResolutionError> {
    let matcher = SimpleGlob::new(name_pattern)?;
    let matched_packages = entry_packages
        .extract_if(|e| matcher.is_match(e.as_ref()))
        .collect::<HashSet<_>>();

    // If the pattern was an exact name and it matched no packages, then error
    if matcher.is_exact() && matched_packages.is_empty() {
        Err(ResolutionError::NoPackagesMatchedWithName(
            name_pattern.to_owned(),
        ))
    } else {
        Ok(matched_packages)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolutionError {
    #[error("missing info for package")]
    MissingPackageInfo(String),
    #[error("No packages matched the provided filter")]
    NoPackagesMatched,
    #[error("Multiple packages matched the provided filter")]
    MultiplePackagesMatched,
    #[error("The provided filter matched a package that is not in the workspace")]
    PackageNotInWorkspace,
    #[error("No package found with name '{0}' in workspace")]
    NoPackagesMatchedWithName(String),
    #[error("selector not used: {0}")]
    InvalidSelector(#[from] InvalidSelectorError),
    #[error("Invalid regex pattern")]
    InvalidRegex(#[from] regex::Error),
    #[error("Invalid glob pattern")]
    InvalidGlob(#[from] wax::BuildError),
    #[error("Unable to query SCM: {0}")]
    Scm(#[from] turborepo_scm::Error),
    #[error("Unable to calculate changes: {0}")]
    ChangeDetectError(#[from] ChangeMapError),
    #[error("'Invalid directory filter '{glob}': {err}")]
    InvalidDirectoryGlob {
        glob: String,
        err: Box<wax::BuildError>,
    },
    #[error("Directory '{0}' specified in filter does not exist")]
    DirectoryDoesNotExist(AbsoluteSystemPathBuf),
    #[error("failed to construct glob for globalDependencies")]
    GlobalDependenciesGlob(#[from] global_deps_package_change_mapper::Error),
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use tempfile::TempDir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};
    use turborepo_repository::{
        discovery::PackageDiscovery,
        package_graph::{PackageGraph, PackageName, ROOT_PKG_NAME},
        package_json::PackageJson,
        package_manager::PackageManager,
    };

    use super::{FilterResolver, PackageInference, TargetSelector};
    use crate::run::scope::{
        change_detector::GitChangeDetector, target_selector::GitRange, ResolutionError,
    };

    fn get_name(name: &str) -> (Option<&str>, &str) {
        if let Some(idx) = name.rfind('/') {
            // check if the rightmost slash has an @
            if let Some(idx) = name[..idx].find('@') {
                return (Some(&name[..idx - 1]), &name[idx..]);
            }

            return (Some(&name[..idx]), &name[idx + 1..]);
        }

        (None, name)
    }

    fn reverse<T, U>(tuple: (T, U)) -> (U, T) {
        let (a, b) = tuple;
        (b, a)
    }

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(turborepo_repository::discovery::DiscoveryResponse {
                package_manager: PackageManager::Pnpm6,
                workspaces: vec![], // we don't care about this
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            self.discover_packages().await
        }
    }

    /// Make a project resolver with the provided dependencies. Extras is for
    /// packages that are not dependencies of any other package.
    fn make_project<T: GitChangeDetector>(
        dependencies: &[(&str, &str)],
        extras: &[&str],
        package_inference: Option<PackageInference>,
        change_detector: T,
    ) -> (TempDir, super::FilterResolver<'static, T>) {
        let temp_folder = tempfile::tempdir().unwrap();
        let turbo_root = Box::leak(Box::new(
            AbsoluteSystemPathBuf::new(temp_folder.path().as_os_str().to_str().unwrap()).unwrap(),
        ));

        let package_dirs = dependencies
            .iter()
            .flat_map(|(a, b)| vec![a, b])
            .chain(extras.iter())
            .collect::<HashSet<_>>();

        let dependencies =
            dependencies
                .iter()
                .fold(HashMap::<&str, Vec<&str>>::new(), |mut acc, (k, v)| {
                    let k = get_name(k).1;
                    let v = get_name(v).1;
                    acc.entry(k).or_default().push(v);
                    acc
                });

        let package_jsons = package_dirs
            .iter()
            .map(|package_path| {
                let (_, name) = get_name(package_path);
                (
                    turbo_root.join_unix_path(
                        RelativeUnixPathBuf::new(format!("{package_path}/package.json")).unwrap(),
                    ),
                    PackageJson {
                        name: Some(name.to_string()),
                        dependencies: dependencies.get(name).map(|v| {
                            v.iter()
                                .map(|name| (name.to_string(), "*".to_string()))
                                .collect()
                        }),
                        ..Default::default()
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        for package_dir in package_jsons.keys() {
            package_dir.ensure_dir().unwrap();
        }

        let graph = {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(
                PackageGraph::builder(turbo_root, Default::default())
                    .with_package_discovery(MockDiscovery)
                    .with_package_jsons(Some(package_jsons))
                    .build(),
            )
            .unwrap()
        };

        let pkg_graph = Box::leak(Box::new(graph));

        let scm = Box::leak(Box::new(turborepo_scm::SCM::new(turbo_root)));

        let resolver = FilterResolver::<'static>::new_with_change_detector(
            pkg_graph,
            turbo_root,
            package_inference,
            scm,
            change_detector,
        );

        // TempDir's drop implementation will mark the folder as ready for cleanup
        // which can lead to non-deterministic test results if the folder is removed
        // before the test finishes.
        (temp_folder, resolver)
    }

    #[test_case(
        vec![
            TargetSelector {
                name_pattern: ROOT_PKG_NAME.to_string(),
                ..Default::default()
            }
        ],
        None,
        &[ROOT_PKG_NAME] ;
        "select root package"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: true,
                include_dependencies: true,
                name_pattern: "project-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-2", "project-4"] ;
        "select only package dependencies (excluding the package itself)"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: false,
                include_dependencies: true,
                name_pattern: "project-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-1", "project-2", "project-4"] ;
        "select package with dependencies"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: false,
                include_dependencies: true,
                name_pattern: "project-0".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0", "project-1", "project-2", "project-4", "project-5"] ;
        "select package with transitive dependencies"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: true,
                include_dependencies: true,
                include_dependents: true,
                name_pattern: "project-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0", "project-1", "project-2", "project-4", "project-5"] ;
        "select package with dependencies and dependents, including dependent
    dependencies" )]
    #[test_case(
        vec![
            TargetSelector {
                include_dependents: true,
                name_pattern: "project-2".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-1", "project-2", "project-0"] ;
        "select package with dependents"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: true,
                include_dependents: true,
                name_pattern: "project-2".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0", "project-1"] ;
        "select dependents excluding package itself"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude_self: true,
                include_dependents: true,
                name_pattern: "project-2".to_string(),
                ..Default::default()
            },
            TargetSelector {
                include_dependencies: true,
                exclude_self: true,
                name_pattern: "project-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0", "project-1", "project-2", "project-4"] ;
        "filter using two selectors: one selects dependencies another selects
    dependents" )]
    #[test_case(
        vec![
            TargetSelector {
                name_pattern: "project-2".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-2"] ;
        "select just a package by name"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir:
    Some(AnchoredSystemPathBuf::try_from("packages/*").unwrap()),
    ..Default::default()         }
        ],
        None,
        &["project-0", "project-1"] ;
        "select by parentDir using glob"
    )]
    #[test_case(
        vec![TargetSelector {
            parent_dir: Some(AnchoredSystemPathBuf::try_from(if cfg!(windows) { "..\\packages\\*" } else { "../packages/*" }).unwrap()),
            ..Default::default()
        }],
        Some(PackageInference{
            package_name: None,
            directory_root: AnchoredSystemPathBuf::try_from("project-5").unwrap(),
        }),
        &["project-0", "project-1"] ;
        "select sibling directory"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir:
    Some(AnchoredSystemPathBuf::try_from("project-5/**").unwrap()),
    ..Default::default()         }
        ],
        None,
        &["project-5", "project-6"] ;
        "select by parentDir using globstar"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir:
    Some(AnchoredSystemPathBuf::try_from("project-5").unwrap()),
    ..Default::default()         }
        ],
        None,
        &["project-5"] ;
        "select by parentDir with no glob"
    )]
    #[test_case(
        vec![
            TargetSelector {
                exclude: true,
                name_pattern: "project-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0", "project-2", "project-3", "project-4", "project-5", "project-6"] ;
        "select all packages except one"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir: Some(AnchoredSystemPathBuf::try_from("packages/*").unwrap()),
                ..Default::default()
            },
            TargetSelector {
                exclude: true,
                name_pattern: "*-1".to_string(),
                ..Default::default()
            }
        ],
        None,
        &["project-0"] ;
        "select by parentDir and exclude one package by pattern"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir: Some(AnchoredSystemPathBuf::try_from(".").unwrap()),
                ..Default::default()
            }
        ],
        None,
        &[ROOT_PKG_NAME] ;
        "select root package by directory"
    )]
    #[test_case(
        vec![],
        Some(PackageInference{
            package_name: None,
            directory_root: AnchoredSystemPathBuf::try_from("packages").unwrap(),
        }),
        &["project-0", "project-1"] ;
        "select packages directory"
    )]
    #[test_case(
        vec![],
        Some(PackageInference{
            package_name: Some("project-0".to_string()),
            directory_root: AnchoredSystemPathBuf::try_from("packages/project-0").unwrap(),
        }),
        &["project-0"] ;
        "infer single package"
    )]
    #[test_case(
        vec![],
        Some(PackageInference{
            package_name: Some("project-0".to_string()),
            directory_root: AnchoredSystemPathBuf::try_from("packages/project-0/src").unwrap(),
        }),
        &["project-0"] ;
        "infer single package from subdirectory"
    )]
    fn filter(
        selectors: Vec<TargetSelector>,
        package_inference: Option<PackageInference>,
        expected: &[&str],
    ) {
        let (_tempdir, resolver) = make_project(
            &[
                ("packages/project-0", "packages/project-1"),
                ("packages/project-0", "project-5"),
                ("packages/project-1", "project-2"),
                ("packages/project-1", "project-4"),
            ],
            &["project-3", "project-5/packages/project-6"],
            package_inference,
            TestChangeDetector::new(&[]),
        );

        let packages = resolver.get_filtered_packages(selectors).unwrap();

        assert_eq!(
            packages,
            expected.iter().map(|s| PackageName::from(*s)).collect()
        );
    }

    #[test]
    fn match_exact() {
        let (_tempdir, resolver) = make_project(
            &[],
            &["packages/@foo/bar", "packages/bar"],
            None,
            TestChangeDetector::new(&[]),
        );
        let packages = resolver
            .get_filtered_packages(vec![TargetSelector {
                name_pattern: "bar".to_string(),
                ..Default::default()
            }])
            .unwrap();

        assert_eq!(
            packages,
            vec![PackageName::Other("bar".to_string())]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn match_scoped_package() {
        let (_tempdir, resolver) = make_project(
            &[],
            &["packages/bar/@foo/bar"],
            None,
            TestChangeDetector::new(&[]),
        );
        let packages = resolver.get_filtered_packages(vec![TargetSelector {
            name_pattern: "bar".to_string(),
            ..Default::default()
        }]);

        assert!(packages.is_err(), "non existing package name should error",);

        let packages = resolver
            .get_filtered_packages(vec![TargetSelector {
                name_pattern: "@foo/bar".to_string(),
                ..Default::default()
            }])
            .unwrap();
        assert_eq!(
            packages,
            vec![PackageName::from("@foo/bar")].into_iter().collect()
        );
    }

    #[test]
    fn test_no_matching_name() {
        let (_tempdir, resolver) = make_project(
            &[],
            &["packages/bar/@foo/bar"],
            None,
            TestChangeDetector::new(&[]),
        );
        let packages = resolver.get_filtered_packages(vec![TargetSelector {
            name_pattern: "bar".to_string(),
            ..Default::default()
        }]);

        assert!(packages.is_err(), "non existing package name should error",);

        let packages = resolver
            .get_filtered_packages(vec![TargetSelector {
                name_pattern: "baz*".to_string(),
                ..Default::default()
            }])
            .unwrap();
        assert!(
            packages.is_empty(),
            "expected no matches, got {:?}",
            packages
        );
    }

    #[test]
    fn test_no_directory() {
        let (_tempdir, resolver) = make_project(
            &[("packages/foo", "packages/bar")],
            &[],
            None,
            TestChangeDetector::new(&[]),
        );
        let packages = resolver.get_filtered_packages(vec![TargetSelector {
            parent_dir: Some(AnchoredSystemPathBuf::try_from("pakcages/*").unwrap()),
            ..Default::default()
        }]);

        assert!(packages.is_err(), "non existing dir should error",);
    }

    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                ..Default::default()
            }
        ],
        &["package-1", "package-2", ROOT_PKG_NAME] ;
        "all changed packages"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                parent_dir: Some(AnchoredSystemPathBuf::try_from(".").unwrap()),
                ..Default::default()
            }
        ],
        &[ROOT_PKG_NAME] ;
        "all changed packages with parent dir exact match"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                parent_dir: Some(AnchoredSystemPathBuf::try_from("package-2").unwrap()),
                ..Default::default()
            }
        ],
        &["package-2"] ;
        "changed packages in directory"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                name_pattern: "package-2*".to_string(),
                ..Default::default()
            }
        ],
        &["package-2"] ;
        "changed packages matching pattern"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                name_pattern: "package-1".to_string(),
                match_dependencies: true,
                ..Default::default()
            }
        ],
        &["package-1"] ;
        "changed package was requested scope, and we're matching dependencies"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~2".to_string()), to_ref: None, ..Default::default() }),
                ..Default::default()
            }
        ],
        &["package-1", "package-2", "package-3", ROOT_PKG_NAME] ;
        "older commit"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~2".to_string()), to_ref: Some("HEAD~1".to_string()), ..Default::default() }),
                ..Default::default()
            }
        ],
        &["package-3"] ;
        "commit range"
    )]
    #[test_case(
        vec![
            TargetSelector {
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                parent_dir: Some(AnchoredSystemPathBuf::try_from("package-*").unwrap()),
                match_dependencies: true,             ..Default::default()
            }
        ],
        &["package-1", "package-2"] ;
        "match dependency subtree"
    )]
    fn scm(selectors: Vec<TargetSelector>, expected: &[&str]) {
        let scm_resolver = TestChangeDetector::new(&[
            ("HEAD~1", None, &["package-1", "package-2", ROOT_PKG_NAME]),
            ("HEAD~2", Some("HEAD~1"), &["package-3"]),
            (
                "HEAD~2",
                None,
                &["package-1", "package-2", "package-3", ROOT_PKG_NAME],
            ),
        ]);

        let (_tempdir, resolver) = make_project(
            &[("package-3", "package-20")],
            &["package-1", "package-2"],
            None,
            scm_resolver,
        );

        let packages = resolver.get_filtered_packages(selectors).unwrap();
        assert_eq!(
            packages,
            expected.iter().map(|s| PackageName::from(*s)).collect()
        );
    }

    struct TestChangeDetector<'a>(HashMap<(&'a str, Option<&'a str>), HashSet<PackageName>>);

    impl<'a> TestChangeDetector<'a> {
        fn new(pairs: &[(&'a str, Option<&'a str>, &[&'a str])]) -> Self {
            let mut map = HashMap::new();
            for (from, to, changed) in pairs {
                map.insert(
                    (*from, *to),
                    changed.iter().map(|s| PackageName::from(*s)).collect(),
                );
            }

            Self(map)
        }
    }

    impl<'a> GitChangeDetector for TestChangeDetector<'a> {
        fn changed_packages(
            &self,
            from: Option<&str>,
            to: Option<&str>,
            _include_uncommitted: bool,
            _allow_unknown_objects: bool,
        ) -> Result<HashSet<PackageName>, ResolutionError> {
            Ok(self
                .0
                .get(&(from.expect("expected base branch"), to))
                .map(|h| h.to_owned())
                .expect("unsupported range"))
        }
    }
}
