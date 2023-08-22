use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_scm::SCM;
use wax::Pattern;

use super::{
    change_detector::{ChangeDetectError, PackageChangeDetector, SCMChangeDetector},
    simple_glob::{Match, SimpleGlob},
    target_selector::{InvalidSelectorError, TargetSelector},
};
use crate::{
    package_graph::{self, PackageGraph, WorkspaceName, WorkspaceNode},
    run::task_id::ROOT_PKG_NAME,
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
        pkg_graph: &package_graph::PackageGraph,
    ) -> Self {
        debug!(
            "Using {} as a basis for selecting packages",
            pkg_inference_path
        );
        let full_inference_path = turbo_root.resolve(pkg_inference_path);
        for (workspace_name, workspace_entry) in pkg_graph.workspaces() {
            let pkg_path = turbo_root.resolve(workspace_entry.package_json_path());
            let inferred_path_is_below = pkg_path.contains(&full_inference_path);
            // We skip over the root package as the inferred path will always be below it
            if inferred_path_is_below && (&pkg_path as &AbsoluteSystemPath) != turbo_root {
                // set both. The user might have set a parent directory filter,
                // in which case we *should* fail to find any packages, but we should
                // do so in a consistent manner
                return Self {
                    package_name: Some(workspace_name.to_string()),
                    directory_root: workspace_entry.package_json_path().to_owned(),
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
            selector.name_pattern = name.to_owned();
        }

        if selector.parent_dir != turbopath::AnchoredSystemPathBuf::default() {
            selector.parent_dir = self.directory_root.join(&selector.parent_dir);
        } else if self.package_name.is_none() {
            // fallback: the user didn't set a parent directory and we didn't find a single
            // package, so use the directory we inferred and select all subdirectories
            let mut parent_dir = self.directory_root.clone();
            parent_dir.push("**");
            selector.parent_dir = parent_dir;
        }
    }
}

pub struct FilterResolver<'a, T: PackageChangeDetector> {
    pkg_graph: &'a PackageGraph,
    turbo_root: &'a AbsoluteSystemPath,
    inference: Option<PackageInference>,
    scm: &'a SCM,
    change_detector: T,
}

impl<'a> FilterResolver<'a, SCMChangeDetector<'a>> {
    pub(crate) fn new(
        opts: &'a super::ScopeOpts,
        pkg_graph: &'a PackageGraph,
        turbo_root: &'a AbsoluteSystemPath,
        inference: Option<PackageInference>,
        scm: &'a SCM,
    ) -> Self {
        let change_detector = SCMChangeDetector::new(
            turbo_root,
            scm,
            pkg_graph,
            opts.global_deps.clone(),
            opts.ignore_patterns.clone(),
        );
        Self::new_with_change_detector(pkg_graph, turbo_root, inference, scm, change_detector)
    }
}

impl<'a, T: PackageChangeDetector> FilterResolver<'a, T> {
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
        patterns: &Vec<String>,
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        // inference is None only if we are in the root
        let is_all_packages = patterns.is_empty() && self.inference.is_none();

        let mut filter_patterns = if is_all_packages {
            // return all packages in the workspace
            self.pkg_graph
                .workspaces()
                .map(|(name, _)| name.to_owned())
                .collect()
        } else {
            self.get_packages_from_patterns(patterns)?
        };

        // if the root package is in the filtered packages, remove it
        if let Some(pkg_name) = &self.pkg_graph.root_package_json().name {
            let name = WorkspaceName::Other(pkg_name.to_owned());
            filter_patterns.remove(&name);
        }

        Ok(filter_patterns)
    }

    fn get_packages_from_patterns(
        &self,
        patterns: &[String],
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        let selectors = patterns
            .iter()
            .map(|pattern| TargetSelector::from_str(pattern))
            .collect::<Result<Vec<_>, _>>()?;

        self.get_filtered_packages(selectors)
    }

    fn get_filtered_packages(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
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
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        let (include_selectors, exclude_selectors) =
            selectors.into_iter().partition::<Vec<_>, _>(|t| !t.exclude);

        let mut include = if !include_selectors.is_empty() {
            self.filter_graph_with_selectors(include_selectors)?
        } else {
            self.pkg_graph
                .workspaces()
                // todo: a type-level way of dealing with non-root packages
                .filter(|(name, _)| !WorkspaceName::Root.eq(name)) // the root package has to be explicitly included
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
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
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
                let node = package_graph::WorkspaceNode::Workspace(package.clone());

                if selector.include_dependencies {
                    let dependencies = self.pkg_graph.immediate_dependencies(&node);
                    let dependencies = dependencies
                        .iter()
                        .flatten()
                        .map(|i| i.as_workspace().to_owned())
                        .collect::<Vec<_>>();

                    // flatmap through the option, the set, and then the optional package name
                    walked_dependencies.extend(dependencies);
                }

                if selector.include_dependents {
                    let dependents = self.pkg_graph.ancestors(&node);
                    for dependent in dependents.iter().map(|i| i.as_workspace()) {
                        walked_dependents.insert(dependent.clone());

                        // get the dependent's dependencies
                        if selector.include_dependencies {
                            let dependent_node =
                                package_graph::WorkspaceNode::Workspace(dependent.to_owned());

                            let dependent_dependencies =
                                self.pkg_graph.immediate_dependencies(&dependent_node);

                            let dependent_dependencies = dependent_dependencies
                                .iter()
                                .flatten()
                                .map(|i| i.as_workspace().to_owned())
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
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
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
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        let mut entry_packages = HashSet::new();

        for (name, info) in self.pkg_graph.workspaces() {
            if selector.parent_dir == AnchoredSystemPathBuf::default() {
                entry_packages.insert(name.to_owned());
            } else {
                let path = self
                    .turbo_root
                    .join_unix_path(selector.parent_dir.to_unix())
                    .unwrap();
                let matcher = wax::Glob::new(path.as_str())?;
                let p = self.turbo_root.resolve(&info.package_json_path);
                let matches = matcher.is_match(p.as_std_path());

                if matches {
                    entry_packages.insert(name.to_owned());
                }
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
        let changed_packages =
            self.packages_changed_in_range(&selector.from_ref, selector.to_ref())?;

        for package in filtered_entry_packages {
            if matched.contains(&package) {
                roots.insert(package);
                continue;
            }

            let workspace_node = package_graph::WorkspaceNode::Workspace(package.clone());
            let dependencies = self.pkg_graph.immediate_dependencies(&workspace_node);

            for changed_package in &changed_packages {
                if !selector.exclude_self && package.eq(changed_package) {
                    roots.insert(package);
                    break;
                }

                let changed_node =
                    package_graph::WorkspaceNode::Workspace(changed_package.to_owned());

                if dependencies
                    .as_ref()
                    .map(|d| d.contains(&changed_node))
                    .unwrap_or_default()
                {
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
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        let mut entry_packages = HashSet::new();
        let mut selector_valid = false;

        let path = self
            .turbo_root
            .join_unix_path(selector.parent_dir.to_unix())
            .unwrap();
        let globber = wax::Glob::new(path.as_str()).unwrap();

        if !selector.from_ref.is_empty() {
            selector_valid = true;
            let changed_packages =
                self.packages_changed_in_range(&selector.from_ref, selector.to_ref())?;
            let package_path_lookup = self
                .pkg_graph
                .workspaces()
                .map(|(name, entry)| (name, entry.package_json_path()))
                .collect::<HashMap<_, _>>();

            for package in changed_packages {
                if selector.parent_dir == AnchoredSystemPathBuf::default() {
                    entry_packages.insert(package);
                    continue;
                };

                if package == WorkspaceName::Root {
                    // The root package changed, only add it if
                    // the parentDir is equivalent to the root
                    if globber.matched(&self.turbo_root.into()).is_some() {
                        entry_packages.insert(package);
                    }
                } else {
                    let path = package_path_lookup
                        .get(&package)
                        .ok_or(ResolutionError::MissingPackageInfo(package.to_string()))?;

                    let path = self.turbo_root.resolve(path);
                    if globber.is_match(path.as_std_path()) {
                        entry_packages.insert(package);
                    }
                }
            }
        } else if selector.parent_dir != AnchoredSystemPathBuf::default() {
            selector_valid = true;
            if selector.parent_dir == AnchoredSystemPathBuf::from_raw(".").expect("valid anchored")
            {
                entry_packages.insert(WorkspaceName::Root);
            } else {
                let path = self
                    .turbo_root
                    .join_unix_path(selector.parent_dir.to_unix())
                    .unwrap();
                let globber = wax::Glob::new(path.as_str())?;
                let packages = self.pkg_graph.workspaces();
                for (name, _) in packages.filter(|(_name, info)| {
                    let path = self.turbo_root.resolve(&info.package_json_path);
                    globber.is_match(path.as_std_path())
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
                        .workspaces()
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
        from_ref: &str,
        to_ref: &str,
    ) -> Result<HashSet<WorkspaceName>, ChangeDetectError> {
        self.change_detector.changed_packages(from_ref, to_ref)
    }

    fn match_package_names_to_vertices(
        &self,
        name_pattern: &str,
        mut entry_packages: HashSet<WorkspaceName>,
    ) -> Result<HashSet<WorkspaceName>, ResolutionError> {
        // add the root package to the entry packages
        entry_packages.insert(WorkspaceName::Root);

        Ok(match_package_names(name_pattern, entry_packages)?)
    }
}

/// match the provided name pattern against the provided set of packages
/// and return the set of packages that match the pattern
///
/// the pattern is normalized, replacing `\*` with `.*`
fn match_package_names(
    name_pattern: &str,
    mut entry_packages: HashSet<WorkspaceName>,
) -> Result<HashSet<WorkspaceName>, regex::Error> {
    let matcher = SimpleGlob::new(name_pattern)?;
    let matched_packages = entry_packages
        .extract_if(|e| matcher.is_match(e.as_ref()))
        .collect::<HashSet<_>>();

    // if we got no matches and the pattern is not scoped
    // check if we have exactly one scoped package that does match
    if matched_packages.is_empty()
        && !name_pattern.starts_with('@')
        && !name_pattern.starts_with('/')
    {
        let scoped_pattern = format!("@*/{}", name_pattern);
        let scoped_matcher = SimpleGlob::new(&scoped_pattern)?;

        let (first_item, multiple_matches) = {
            let mut scoped_matched_packages =
                entry_packages.extract_if(|e| scoped_matcher.is_match(e.as_ref())); // we can extract again since the original set is untouched
            let first_item = scoped_matched_packages.next();
            (first_item, scoped_matched_packages.count() > 0)
        };

        if multiple_matches {
            // if we have more than one, we can't disambiguate
            Ok(Default::default())
        } else {
            // otherwise we either have a match or no match
            Ok(first_item.into_iter().collect())
        }
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
    #[error("selector not used: {0}")]
    InvalidSelector(#[from] InvalidSelectorError),
    #[error("Invalid regex pattern")]
    InvalidRegex(#[from] regex::Error),
    #[error("Invalid glob pattern")]
    InvalidGlob(#[from] wax::BuildError),
    #[error("Unable to query SCM: {0}")]
    Scm(#[from] turborepo_scm::Error),
    #[error("Unable to calculate changes: {0}")]
    ChangeDetectError(#[from] ChangeDetectError),
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};

    use super::{FilterResolver, PackageInference, TargetSelector};
    use crate::{
        package_graph::{PackageGraph, WorkspaceName},
        package_json::PackageJson,
        package_manager::PackageManager,
        run::{
            scope::change_detector::{ChangeDetectError, PackageChangeDetector},
            task_id::ROOT_PKG_NAME,
        },
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

    /// Make a project resolver with the provided dependencies. Extras is for
    /// packages that are not dependencies of any other package.
    fn make_project<T: PackageChangeDetector>(
        dependencies: &[(&str, &str)],
        extras: &[&str],
        package_inference: Option<PackageInference>,
        change_detector: T,
    ) -> super::FilterResolver<'static, T> {
        let temp_folder = tempfile::tempdir().unwrap();
        let turbo_root = Box::leak(Box::new(
            AbsoluteSystemPathBuf::new(temp_folder.path().as_os_str().to_str().unwrap()).unwrap(),
        ));

        let packages = dependencies
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

        let package_jsons = packages
            .iter()
            .map(|package_path| {
                let (_, name) = get_name(package_path);
                (
                    turbo_root
                        .join_unix_path(RelativeUnixPathBuf::new(**package_path).unwrap())
                        .unwrap(),
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
            .collect();

        let pkg_graph = Box::leak(Box::new(
            PackageGraph::builder(turbo_root, Default::default())
                .with_package_jsons(Some(package_jsons))
                .with_package_manger(Some(PackageManager::Pnpm6))
                .build()
                .unwrap(),
        ));

        let scm = Box::leak(Box::new(turborepo_scm::SCM::new(turbo_root)));

        let resolver = FilterResolver::<'static>::new_with_change_detector(
            pkg_graph,
            turbo_root,
            package_inference,
            scm,
            change_detector,
        );

        resolver
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
    AnchoredSystemPathBuf::try_from("packages/*").unwrap(),
    ..Default::default()         }
        ],
        None,
        &["project-0", "project-1"] ;
        "select by parentDir using glob"
    )]
    #[test_case(
        vec![
            TargetSelector {
                parent_dir:
    AnchoredSystemPathBuf::try_from("project-5/**").unwrap(),
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
    AnchoredSystemPathBuf::try_from("project-5").unwrap(),
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
                parent_dir: AnchoredSystemPathBuf::try_from("packages/*").unwrap(),
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
                parent_dir: AnchoredSystemPathBuf::try_from(".").unwrap(),
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
        let resolver = make_project(
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
            expected
                .iter()
                .map(|s| if ROOT_PKG_NAME.eq(*s) {
                    WorkspaceName::Root
                } else {
                    WorkspaceName::Other(s.to_string())
                })
                .collect()
        );
    }

    #[test]
    fn match_exact() {
        let resolver = make_project(
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
            vec![WorkspaceName::Other("bar".to_string())]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn match_scoped_package() {
        let resolver = make_project(
            &[],
            &["packages/bar/@foo/bar"],
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
            vec![WorkspaceName::Other("@foo/bar".to_string())]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn match_multiple_scoped() {
        let resolver = make_project(
            &[],
            &["packages/@foo/bar", "packages/@types/bar"],
            None,
            TestChangeDetector::new(&[]),
        );
        let packages = resolver
            .get_filtered_packages(vec![TargetSelector {
                name_pattern: "bar".to_string(),
                ..Default::default()
            }])
            .unwrap();

        assert_eq!(packages, HashSet::new());
    }

    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~1".to_string(),
                ..Default::default()
            }
        ],
        &["package-1", "package-2", ROOT_PKG_NAME] ;
        "all changed packages"
    )]
    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~1".to_string(),
                parent_dir: AnchoredSystemPathBuf::try_from(".").unwrap(),
                ..Default::default()
            }
        ],
        &[ROOT_PKG_NAME] ;
        "all changed packages with parent dir exact match"
    )]
    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~1".to_string(),
                parent_dir: AnchoredSystemPathBuf::try_from("package-2").unwrap(),
                ..Default::default()
            }
        ],
        &["package-2"] ;
        "changed packages in directory"
    )]
    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~1".to_string(),
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
                from_ref: "HEAD~1".to_string(),
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
                from_ref: "HEAD~2".to_string(),
                ..Default::default()
            }
        ],
        &["package-1", "package-2", "package-3", ROOT_PKG_NAME] ;
        "older commit"
    )]
    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~2".to_string(),
                to_ref_override: "HEAD~1".to_string(),
                ..Default::default()
            }
        ],
        &["package-3"] ;
        "commit range"
    )]
    #[test_case(
        vec![
            TargetSelector {
                from_ref: "HEAD~1".to_string(),
                parent_dir:
    AnchoredSystemPathBuf::try_from("package-*").unwrap(),
    match_dependencies: true,             ..Default::default()
            }
        ],
        &["package-1", "package-2"] ;
        "match dependency subtree"
    )]
    fn scm(selectors: Vec<TargetSelector>, expected: &[&str]) {
        let scm_resolver = TestChangeDetector::new(&[
            ("HEAD~1", "HEAD", &["package-1", "package-2", ROOT_PKG_NAME]),
            ("HEAD~2", "HEAD~1", &["package-3"]),
            (
                "HEAD~2",
                "HEAD",
                &["package-1", "package-2", "package-3", ROOT_PKG_NAME],
            ),
        ]);

        let resolver = make_project(
            &[("package-3", "package-20")],
            &["package-1", "package-2"],
            None,
            scm_resolver,
        );

        let packages = resolver.get_filtered_packages(selectors).unwrap();
        assert_eq!(
            packages,
            expected
                .iter()
                .map(|s| if ROOT_PKG_NAME.eq(*s) {
                    WorkspaceName::Root
                } else {
                    WorkspaceName::Other(s.to_string())
                })
                .collect()
        );
    }

    struct TestChangeDetector<'a>(HashMap<(&'a str, &'a str), HashSet<WorkspaceName>>);

    impl<'a> TestChangeDetector<'a> {
        fn new(pairs: &[(&'a str, &'a str, &[&'a str])]) -> Self {
            let mut map = HashMap::new();
            for (from, to, changed) in pairs {
                map.insert(
                    (*from, *to),
                    changed
                        .iter()
                        .map(|s| {
                            if ROOT_PKG_NAME.eq(*s) {
                                WorkspaceName::Root
                            } else {
                                WorkspaceName::Other(s.to_string())
                            }
                        })
                        .collect(),
                );
            }

            Self(map)
        }
    }

    impl<'a> PackageChangeDetector for TestChangeDetector<'a> {
        fn changed_packages(
            &self,
            from: &str,
            to: &str,
        ) -> Result<HashSet<WorkspaceName>, ChangeDetectError> {
            Ok(self
                .0
                .get(&(from, to))
                .map(|h| h.to_owned())
                .expect("unsupported range"))
        }
    }
}
