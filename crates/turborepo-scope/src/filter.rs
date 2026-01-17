//! Filter pattern parsing and resolution.
//!
//! This module handles --filter flag parsing and package filtering.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    str::FromStr,
};

use miette::Diagnostic;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{ChangeMapError, PackageInclusionReason, merge_changed_packages},
    package_graph::{self, PackageGraph, PackageName},
};
use turborepo_scm::SCM;
use wax::Program;

use crate::{
    ScopeOpts,
    change_detector::{GitChangeDetector, ScopeChangeDetector},
    simple_glob::{Match, SimpleGlob},
    target_selector::{GitRange, InvalidSelectorError, TargetSelector},
};

/// Package inference for directory-based filtering.
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

        // Track the best matching package (the one whose path is the longest prefix
        // of the inference path, i.e., the most specific package containing our
        // current directory)
        let mut best_match: Option<(String, AnchoredSystemPathBuf)> = None;
        let mut found_package_below = false;

        for (workspace_name, workspace_entry) in pkg_graph.packages() {
            let pkg_path = turbo_root.resolve(workspace_entry.package_path());

            // Check if the inference path is inside this package (pkg_path is a prefix of
            // full_inference_path)
            let inferred_path_is_below = pkg_path.contains(&full_inference_path);

            // We skip over the root package as the inferred path will always be below it
            if inferred_path_is_below && (&pkg_path as &AbsoluteSystemPath) != turbo_root {
                // This package contains the inference path. Track it if it's a better
                // (longer/more specific) match than what we've found so far.
                let pkg_path_len = workspace_entry.package_path().as_str().len();
                let is_better_match = match &best_match {
                    None => true,
                    Some((_, existing_path)) => pkg_path_len > existing_path.as_str().len(),
                };

                if is_better_match {
                    best_match = Some((
                        workspace_name.to_string(),
                        workspace_entry.package_path().to_owned(),
                    ));
                }
            }

            // Check if this package is below the inference path (full_inference_path is a
            // prefix of pkg_path)
            let inferred_path_is_between_root_and_pkg = full_inference_path.contains(&pkg_path);
            if inferred_path_is_between_root_and_pkg {
                // We've found *some* package below our inference directory
                found_package_below = true;
            }
        }

        // If we found a package that contains our inference path, use it
        if let Some((package_name, directory_root)) = best_match {
            return Self {
                package_name: Some(package_name),
                directory_root,
            };
        }

        // If we found packages below the inference path, or no packages matched at all,
        // use the inference path as the directory root
        if found_package_below {
            // We're in a directory that contains packages
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

        // Inject package name based on the directory filter:
        // - No filter: inject name (original behavior)
        // - Filter navigates up (starts with ".."): inject name (backwards compat)
        // - Filter stays within current dir (e.g., "./*"): don't inject name because
        //   user is explicitly selecting child packages
        if let Some(name) = &self.package_name {
            let should_inject_name = match selector.parent_dir.as_deref() {
                None => true,
                Some(parent_dir) => parent_dir.as_str().starts_with(".."),
            };
            if should_inject_name {
                selector.name_pattern.clone_from(name);
            }
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

/// Resolves filter patterns to package sets.
pub struct FilterResolver<'a, T: GitChangeDetector> {
    pkg_graph: &'a PackageGraph,
    turbo_root: &'a AbsoluteSystemPath,
    inference: Option<PackageInference>,
    change_detector: T,
}

impl<'a> FilterResolver<'a, ScopeChangeDetector<'a>> {
    pub fn new(
        opts: &'a ScopeOpts,
        pkg_graph: &'a PackageGraph,
        turbo_root: &'a AbsoluteSystemPath,
        inference: Option<PackageInference>,
        scm: &'a SCM,
        global_deps: &'a [String],
    ) -> Result<Self, ResolutionError> {
        let global_deps_iter = opts
            .global_deps
            .iter()
            .map(|s| s.as_str())
            .chain(global_deps.iter().map(|s| s.as_str()));

        let change_detector =
            ScopeChangeDetector::new(turbo_root, scm, pkg_graph, global_deps_iter, vec![])?;

        Ok(Self::new_with_change_detector(
            pkg_graph,
            turbo_root,
            inference,
            change_detector,
        ))
    }
}

impl<'a, T: GitChangeDetector> FilterResolver<'a, T> {
    pub fn new_with_change_detector(
        pkg_graph: &'a PackageGraph,
        turbo_root: &'a AbsoluteSystemPath,
        inference: Option<PackageInference>,
        change_detector: T,
    ) -> Self {
        Self {
            pkg_graph,
            turbo_root,
            inference,
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
    pub fn resolve(
        &self,
        affected: &Option<(Option<String>, Option<String>)>,
        patterns: &[String],
    ) -> Result<(HashMap<PackageName, PackageInclusionReason>, bool), ResolutionError> {
        // inference is None only if we are in the root
        let is_all_packages = patterns.is_empty() && self.inference.is_none() && affected.is_none();

        let filter_patterns = if is_all_packages {
            // return all packages in the workspace
            self.pkg_graph
                .packages()
                .filter(|(name, _)| matches!(name, PackageName::Other(_)))
                .map(|(name, _)| {
                    (
                        name.to_owned(),
                        PackageInclusionReason::IncludedByFilter {
                            filters: patterns.to_vec(),
                        },
                    )
                })
                .collect()
        } else {
            self.get_packages_from_patterns(affected, patterns)?
        };

        Ok((filter_patterns, is_all_packages))
    }

    fn get_packages_from_patterns(
        &self,
        affected: &Option<(Option<String>, Option<String>)>,
        patterns: &[String],
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        let mut selectors = patterns
            .iter()
            .map(|pattern| TargetSelector::from_str(pattern))
            .collect::<Result<Vec<_>, _>>()?;

        if let Some((from_ref, to_ref)) = affected {
            selectors.push(TargetSelector {
                git_range: Some(GitRange {
                    from_ref: from_ref.clone(),
                    to_ref: to_ref.clone(),
                    include_uncommitted: true,
                    allow_unknown_objects: true,
                    merge_base: true,
                }),
                include_dependents: true,
                ..Default::default()
            });
        }

        self.get_filtered_packages(selectors)
    }

    fn get_filtered_packages(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
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
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
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
                .map(|(name, _)| {
                    (
                        name.to_owned(),
                        PackageInclusionReason::IncludedByFilter {
                            filters: exclude_selectors
                                .iter()
                                .map(|s| s.raw.to_string())
                                .collect(),
                        },
                    )
                })
                .collect()
        };

        // We want to just collect the names, not the reasons, so when we check for
        // inclusion we don't need to check the reason
        let exclude: HashSet<PackageName> = self
            .filter_graph_with_selectors(exclude_selectors)?
            .into_keys()
            .collect();

        include.retain(|i, _| !exclude.contains(i));

        Ok(include)
    }

    fn filter_graph_with_selectors(
        &self,
        selectors: Vec<TargetSelector>,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        let mut unmatched_selectors = Vec::new();
        let mut walked_dependencies = HashMap::new();
        let mut walked_dependents = HashMap::new();
        let mut walked_dependent_dependencies = HashMap::new();
        let mut cherry_picked_packages = HashMap::new();

        for selector in selectors {
            let selector_packages = self.filter_graph_with_selector(&selector)?;

            if selector_packages.is_empty() {
                unmatched_selectors.push(selector);
                continue;
            }

            for (package, reason) in selector_packages {
                let node = package_graph::PackageNode::Workspace(package.clone());

                if selector.include_dependencies {
                    let dependencies = self.pkg_graph.dependencies(&node);
                    let dependencies = dependencies
                        .iter()
                        .filter(|node| !matches!(node, package_graph::PackageNode::Root))
                        .map(|i| {
                            (
                                i.as_package_name().to_owned(),
                                // While we're adding dependencies, from their
                                // perspective, they were changed because
                                // of a *dependent*
                                PackageInclusionReason::DependentChanged {
                                    dependent: package.to_owned(),
                                },
                            )
                        })
                        .collect::<Vec<_>>();

                    // flatmap through the option, the set, and then the optional package name
                    merge_changed_packages(&mut walked_dependencies, dependencies);
                }

                if selector.include_dependents {
                    let dependents = self.pkg_graph.ancestors(&node);
                    for dependent in dependents.iter().map(|i| i.as_package_name()) {
                        walked_dependents.insert(
                            dependent.clone(),
                            // While we're adding dependents, from their
                            // perspective, they were changed because
                            // of a *dependency*
                            PackageInclusionReason::DependencyChanged {
                                dependency: package.to_owned(),
                            },
                        );

                        // get the dependent's dependencies
                        if selector.include_dependencies {
                            let dependent_node =
                                package_graph::PackageNode::Workspace(dependent.to_owned());

                            let dependent_dependencies =
                                self.pkg_graph.dependencies(&dependent_node);

                            let dependent_dependencies = dependent_dependencies
                                .iter()
                                .filter(|node| !matches!(node, package_graph::PackageNode::Root))
                                .map(|i| {
                                    (
                                        i.as_package_name().to_owned(),
                                        PackageInclusionReason::DependencyChanged {
                                            dependency: package.to_owned(),
                                        },
                                    )
                                })
                                .collect::<HashSet<_>>();

                            merge_changed_packages(
                                &mut walked_dependent_dependencies,
                                dependent_dependencies,
                            );
                        }
                    }
                }

                if (selector.include_dependents || selector.include_dependencies)
                    && !selector.exclude_self
                {
                    // if we are including dependents or dependencies, and we are not excluding
                    // ourselves, then we should add ourselves to the list of packages
                    walked_dependencies.insert(package, reason);
                } else if !selector.include_dependencies && !selector.include_dependents {
                    // if we are neither including dependents or dependencies, then
                    // add  to the list of cherry picked packages
                    cherry_picked_packages.insert(package, reason);
                }
            }
        }

        let mut all_packages = HashMap::new();
        merge_changed_packages(&mut all_packages, walked_dependencies);
        merge_changed_packages(&mut all_packages, walked_dependents);
        merge_changed_packages(&mut all_packages, walked_dependent_dependencies);
        merge_changed_packages(&mut all_packages, cherry_picked_packages);

        Ok(all_packages)
    }

    fn filter_graph_with_selector(
        &self,
        selector: &TargetSelector,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
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
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        let mut entry_packages = HashMap::new();

        // Compile glob pattern ONCE outside the loop to avoid O(n) compilation overhead
        let parent_dir_matcher = selector
            .parent_dir
            .as_deref()
            .map(|parent_dir| {
                let path = parent_dir.to_unix();
                wax::Glob::new(path.as_str()).map(wax::Glob::into_owned)
            })
            .transpose()?;

        for (name, info) in self.pkg_graph.packages() {
            if let Some(ref matcher) = parent_dir_matcher {
                let matches = matcher.is_match(info.package_path().as_path());

                if matches {
                    entry_packages.insert(
                        name.to_owned(),
                        PackageInclusionReason::InFilteredDirectory {
                            directory: selector.parent_dir.as_ref().unwrap().to_owned(),
                        },
                    );
                }
            } else {
                entry_packages.insert(
                    name.to_owned(),
                    PackageInclusionReason::IncludedByFilter {
                        filters: vec![selector.raw.to_string()],
                    },
                );
            }
        }

        // if we have a filter, use it to filter the entry packages
        let filtered_entry_packages = if !selector.name_pattern.is_empty() {
            match_package_names(&selector.name_pattern, &self.all_packages(), entry_packages)?
        } else {
            entry_packages
        };

        let mut roots = HashMap::new();
        let mut matched = HashSet::new();
        let changed_packages = if let Some(git_range) = selector.git_range.as_ref() {
            self.packages_changed_in_range(git_range)?
        } else {
            HashMap::default()
        };

        for (package, reason) in filtered_entry_packages {
            if matched.contains(&package) {
                roots.insert(package, reason);
                continue;
            }

            let workspace_node = package_graph::PackageNode::Workspace(package.clone());
            let dependencies = self.pkg_graph.dependencies(&workspace_node);

            for changed_package in changed_packages.keys() {
                if !selector.exclude_self && package.eq(changed_package) {
                    roots.insert(package, reason);
                    break;
                }

                let changed_node =
                    package_graph::PackageNode::Workspace(changed_package.to_owned());

                if dependencies.contains(&changed_node) {
                    roots.insert(package.clone(), reason);
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
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        let mut entry_packages = HashMap::new();
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

            for (package, reason) in changed_packages {
                if let Some(parent_dir_globber) = parent_dir_globber.as_ref() {
                    if package == PackageName::Root {
                        // The root package changed, only add it if
                        // the parentDir is equivalent to the root
                        if parent_dir_globber.matched(&Path::new(".").into()).is_some() {
                            entry_packages.insert(package, reason);
                        }
                    } else {
                        let path = package_path_lookup
                            .get(&package)
                            .ok_or(ResolutionError::MissingPackageInfo(package.to_string()))?;

                        if parent_dir_globber.is_match(path.as_path()) {
                            entry_packages.insert(package, reason);
                        }
                    }
                } else {
                    entry_packages.insert(package, reason);
                }
            }
        } else if let Some((parent_dir, parent_dir_globber)) = selector
            .parent_dir
            .as_deref()
            .zip(parent_dir_globber.as_ref())
        {
            selector_valid = true;
            if parent_dir == &*AnchoredSystemPathBuf::from_raw(".").expect("valid anchored") {
                entry_packages.insert(
                    PackageName::Root,
                    PackageInclusionReason::InFilteredDirectory {
                        directory: parent_dir.to_owned(),
                    },
                );
            } else {
                let packages = self.pkg_graph.packages();
                for (name, _) in packages.filter(|(_name, info)| {
                    let path = info.package_path().as_path();
                    parent_dir_globber.is_match(path)
                }) {
                    entry_packages.insert(
                        name.to_owned(),
                        PackageInclusionReason::InFilteredDirectory {
                            directory: parent_dir.to_owned(),
                        },
                    );
                }
            }
        }

        if !selector.name_pattern.is_empty() {
            if !selector_valid {
                entry_packages = self
                    .all_packages()
                    .into_iter()
                    .map(|name| {
                        (
                            name,
                            PackageInclusionReason::IncludedByFilter {
                                filters: vec![selector.raw.to_string()],
                            },
                        )
                    })
                    .collect();
                selector_valid = true;
            }
            let all_packages = self.all_packages();
            entry_packages =
                match_package_names(&selector.name_pattern, &all_packages, entry_packages)?;
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

    pub fn packages_changed_in_range(
        &self,
        git_range: &GitRange,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        self.change_detector.changed_packages(
            git_range.from_ref.as_deref(),
            git_range.to_ref.as_deref(),
            git_range.include_uncommitted,
            git_range.allow_unknown_objects,
            git_range.merge_base,
        )
    }

    fn all_packages(&self) -> HashSet<PackageName> {
        let mut packages = self
            .pkg_graph
            .packages()
            .map(|(name, _)| name.to_owned())
            .collect::<HashSet<_>>();
        packages.insert(PackageName::Root);
        packages
    }
}

/// match the provided name pattern against the provided set of packages
/// and return the set of packages that match the pattern
///
/// the pattern is normalized, replacing `\*` with `.*`
fn match_package_names(
    name_pattern: &str,
    all_packages: &HashSet<PackageName>,
    mut packages: HashMap<PackageName, PackageInclusionReason>,
) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
    let matcher = SimpleGlob::new(name_pattern)?;
    let matched_packages = all_packages
        .iter()
        .filter(|e| matcher.is_match(e.as_ref()))
        .cloned()
        .collect::<HashSet<_>>();

    // If the pattern was an exact name and it matched no packages, then error
    if matcher.is_exact() && matched_packages.is_empty() {
        return Err(ResolutionError::NoPackagesMatchedWithName(
            name_pattern.to_owned(),
        ));
    }

    packages.retain(|pkg, _| matched_packages.contains(pkg));

    Ok(packages)
}

/// Errors that can occur during scope resolution.
#[derive(Debug, thiserror::Error, Diagnostic)]
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
    GlobalDependenciesGlob(#[from] turborepo_repository::change_mapper::Error),
}

#[cfg(test)]
mod test {
    use std::{
        collections::{HashMap, HashSet},
        str::FromStr,
    };

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        change_mapper::PackageInclusionReason,
        discovery::PackageDiscovery,
        package_graph::{PackageGraph, PackageName, ROOT_PKG_NAME},
        package_json::PackageJson,
        package_manager::PackageManager,
    };

    use super::{FilterResolver, PackageInference};
    use crate::{
        change_detector::GitChangeDetector,
        filter::ResolutionError,
        target_selector::{GitRange, TargetSelector},
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
                        name: Some(Spanned::new(name.to_string())),
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

        let resolver = FilterResolver::<'static>::new_with_change_detector(
            pkg_graph,
            turbo_root,
            package_inference,
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
            packages.into_keys().collect::<HashSet<_>>(),
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
                raw: "bar".to_string(),
                ..Default::default()
            }])
            .unwrap();

        assert_eq!(
            packages,
            vec![(
                PackageName::Other("bar".to_string()),
                PackageInclusionReason::IncludedByFilter {
                    filters: vec!["bar".to_string()]
                }
            )]
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
            raw: "bar".to_string(),
            ..Default::default()
        }]);

        assert!(packages.is_err(), "non existing package name should error",);

        let packages = resolver
            .get_filtered_packages(vec![TargetSelector {
                name_pattern: "@foo/bar".to_string(),
                raw: "@foo/bar".to_string(),
                ..Default::default()
            }])
            .unwrap();

        assert_eq!(
            packages,
            vec![(
                PackageName::from("@foo/bar"),
                PackageInclusionReason::IncludedByFilter {
                    filters: vec!["@foo/bar".to_string()]
                }
            )]
            .into_iter()
            .collect()
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
    #[test_case(
        vec![
            TargetSelector {
                name_pattern: "package-3".to_string(),
                git_range: Some(GitRange { from_ref: Some("HEAD~1".to_string()), to_ref: None, ..Default::default() }),
                ..Default::default()
            }
        ],
        &[] ;
        "gh 9096"
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
            packages.into_keys().collect::<HashSet<_>>(),
            expected.iter().map(|s| PackageName::from(*s)).collect()
        );
    }

    struct TestChangeDetector<'a>(
        HashMap<(&'a str, Option<&'a str>), HashMap<PackageName, PackageInclusionReason>>,
    );

    impl<'a> TestChangeDetector<'a> {
        fn new(pairs: &[(&'a str, Option<&'a str>, &[&'a str])]) -> Self {
            let mut map = HashMap::new();
            for (from, to, changed) in pairs {
                map.insert(
                    (*from, *to),
                    changed
                        .iter()
                        .map(|s| {
                            (
                                PackageName::from(*s),
                                // This is just a random reason,
                                PackageInclusionReason::IncludedByFilter { filters: vec![] },
                            )
                        })
                        .collect(),
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
            _merge_base: bool,
        ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
            Ok(self
                .0
                .get(&(from.expect("expected base branch"), to))
                .map(|h| h.to_owned())
                .expect("unsupported range"))
        }
    }

    /// Creates a package graph for testing PackageInference::calculate
    fn make_package_graph(
        package_paths: &[&str],
    ) -> (TempDir, &'static AbsoluteSystemPathBuf, PackageGraph) {
        let temp_folder = tempfile::tempdir().unwrap();
        let turbo_root = Box::leak(Box::new(
            AbsoluteSystemPathBuf::new(temp_folder.path().as_os_str().to_str().unwrap()).unwrap(),
        ));

        let package_jsons = package_paths
            .iter()
            .map(|package_path| {
                let (_, name) = get_name(package_path);
                (
                    turbo_root.join_unix_path(
                        RelativeUnixPathBuf::new(format!("{package_path}/package.json")).unwrap(),
                    ),
                    PackageJson {
                        name: Some(Spanned::new(name.to_string())),
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

        (temp_folder, turbo_root, graph)
    }

    /// Test that PackageInference::calculate is deterministic when invoked from
    /// a directory that contains nested packages (regression test for
    /// GitHub issue #11428).
    ///
    /// The bug was that HashMap iteration order is non-deterministic, and the
    /// previous implementation would return early when it found ANY package
    /// below the inference path, rather than finding the most specific one.
    #[test]
    fn test_package_inference_deterministic_with_nested_packages() {
        // Simulate the structure from the issue:
        // apps/onprem/          <- invocation directory
        // apps/onprem/backend/  <- nested package
        // apps/onprem/web/      <- nested package
        // packages/shared/      <- unrelated package
        let (_tempdir, turbo_root, graph) =
            make_package_graph(&["apps/onprem/backend", "apps/onprem/web", "packages/shared"]);

        // Invoke from apps/onprem (a directory containing packages)
        let inference_path = AnchoredSystemPathBuf::try_from("apps/onprem").unwrap();

        // Run the calculation multiple times to verify determinism
        // (with the bug, different runs could give different results)
        for _ in 0..10 {
            let inference = PackageInference::calculate(turbo_root, &inference_path, &graph);

            // We should NOT infer a specific package - we're in a directory that contains
            // packages, not inside a specific package
            assert!(
                inference.package_name.is_none(),
                "Expected no package to be inferred when running from a directory containing \
                 packages, but got {:?}",
                inference.package_name
            );

            // The directory root should be the inference path itself
            assert_eq!(
                inference.directory_root, inference_path,
                "Expected directory_root to be the inference path"
            );
        }
    }

    /// Test that PackageInference::calculate correctly identifies when we're
    /// inside a specific package (at or below the package root).
    #[test]
    fn test_package_inference_inside_package() {
        let (_tempdir, turbo_root, graph) =
            make_package_graph(&["apps/onprem/backend", "apps/onprem/web", "packages/shared"]);

        // Invoke from inside apps/onprem/backend
        let inference_path = AnchoredSystemPathBuf::try_from("apps/onprem/backend").unwrap();
        let inference = PackageInference::calculate(turbo_root, &inference_path, &graph);

        assert_eq!(
            inference.package_name,
            Some("backend".to_string()),
            "Expected to infer 'backend' package"
        );
        assert_eq!(
            inference.directory_root,
            AnchoredSystemPathBuf::try_from("apps/onprem/backend").unwrap()
        );

        // Invoke from a subdirectory inside apps/onprem/backend/src
        let inference_path = AnchoredSystemPathBuf::try_from("apps/onprem/backend/src").unwrap();
        let inference = PackageInference::calculate(turbo_root, &inference_path, &graph);

        assert_eq!(
            inference.package_name,
            Some("backend".to_string()),
            "Expected to infer 'backend' package from subdirectory"
        );
        assert_eq!(
            inference.directory_root,
            AnchoredSystemPathBuf::try_from("apps/onprem/backend").unwrap()
        );
    }

    /// Test that PackageInference selects the most specific (deepest) package
    /// when packages are nested.
    #[test]
    fn test_package_inference_selects_deepest_package() {
        // Create a structure with nested packages
        let (_tempdir, turbo_root, graph) = make_package_graph(&[
            "apps",        // shallow package
            "apps/web",    // deeper package inside apps
            "apps/web/ui", // even deeper package
            "packages/shared",
        ]);

        // When invoked from apps/web/ui/src, should infer apps/web/ui (the deepest)
        let inference_path = AnchoredSystemPathBuf::try_from("apps/web/ui/src").unwrap();
        let inference = PackageInference::calculate(turbo_root, &inference_path, &graph);

        assert_eq!(
            inference.package_name,
            Some("ui".to_string()),
            "Expected to infer the deepest package 'ui'"
        );
        assert_eq!(
            inference.directory_root,
            AnchoredSystemPathBuf::try_from("apps/web/ui").unwrap()
        );

        // When invoked from apps/web/src (outside ui), should infer apps/web
        let inference_path = AnchoredSystemPathBuf::try_from("apps/web/src").unwrap();
        let inference = PackageInference::calculate(turbo_root, &inference_path, &graph);

        assert_eq!(
            inference.package_name,
            Some("web".to_string()),
            "Expected to infer 'web' package"
        );
        assert_eq!(
            inference.directory_root,
            AnchoredSystemPathBuf::try_from("apps/web").unwrap()
        );
    }

    /// End-to-end test for GitHub issue #11428: running `turbo -F "{./*}^..."
    /// build` from a directory containing packages should select those
    /// packages' dependencies.
    ///
    /// This test verifies that running from apps/onprem with filter {./*}^...
    /// correctly selects packages under apps/onprem/* and their dependencies,
    /// rather than randomly returning 0 packages.
    #[test]
    fn test_issue_11428_filter_from_directory_with_nested_packages() {
        // Create the structure from the issue:
        // apps/onprem/backend depends on packages/shared
        // apps/onprem/web depends on packages/shared
        let (_tempdir, resolver) = make_project(
            &[
                ("apps/onprem/backend", "packages/shared"),
                ("apps/onprem/web", "packages/shared"),
            ],
            &[],
            // Simulate running from apps/onprem
            Some(PackageInference {
                package_name: None,
                directory_root: AnchoredSystemPathBuf::try_from("apps/onprem").unwrap(),
            }),
            TestChangeDetector::new(&[]),
        );

        // The filter {./*}^... means:
        // - {./*} = packages matching "./*" relative to inference directory
        //   (apps/onprem/*)
        // - ^... = exclude self, include dependencies
        //
        // So this should select: packages/shared (dependency of backend and web)
        // but NOT backend or web themselves (^ excludes self)
        let selector = TargetSelector::from_str("{./*}^...").unwrap();
        let packages = resolver.get_filtered_packages(vec![selector]).unwrap();

        // We should get the dependencies of apps/onprem/* packages
        // which is packages/shared
        assert!(
            !packages.is_empty(),
            "Expected to find packages, but got none. This is the bug from issue #11428!"
        );

        assert!(
            packages.contains_key(&PackageName::from("shared")),
            "Expected 'shared' to be selected as a dependency. Got: {:?}",
            packages.keys().collect::<Vec<_>>()
        );

        // backend and web should NOT be included (^ excludes self)
        assert!(
            !packages.contains_key(&PackageName::from("backend")),
            "backend should be excluded due to ^ in filter"
        );
        assert!(
            !packages.contains_key(&PackageName::from("web")),
            "web should be excluded due to ^ in filter"
        );
    }

    /// Test that running from a directory containing packages with filter {./*}
    /// (without dependency traversal) selects those packages directly.
    #[test]
    fn test_filter_from_directory_selects_child_packages() {
        let (_tempdir, resolver) = make_project(
            &[
                ("apps/onprem/backend", "packages/shared"),
                ("apps/onprem/web", "packages/shared"),
            ],
            &[],
            Some(PackageInference {
                package_name: None,
                directory_root: AnchoredSystemPathBuf::try_from("apps/onprem").unwrap(),
            }),
            TestChangeDetector::new(&[]),
        );

        // {./*} without ^... should select the packages themselves
        let selector = TargetSelector::from_str("{./*}").unwrap();
        let packages = resolver.get_filtered_packages(vec![selector]).unwrap();

        assert!(
            packages.contains_key(&PackageName::from("backend")),
            "Expected 'backend' to be selected. Got: {:?}",
            packages.keys().collect::<Vec<_>>()
        );
        assert!(
            packages.contains_key(&PackageName::from("web")),
            "Expected 'web' to be selected. Got: {:?}",
            packages.keys().collect::<Vec<_>>()
        );

        // shared should NOT be selected (it's not under apps/onprem/*)
        assert!(
            !packages.contains_key(&PackageName::from("shared")),
            "shared should not be selected as it's not under apps/onprem/*"
        );
    }

    /// Test for filter bug where running from a directory that is BOTH a
    /// workspace itself AND contains child workspaces, using a subdirectory
    /// filter like `./*` fails to match the child packages.
    ///
    /// Reproduction structure:
    /// - `apps` is a workspace (has package.json with name "apps")
    /// - `apps/app-a` is a workspace
    /// - `apps/app-b` is a workspace
    /// - `apps/app-a/app-a-client` is a nested workspace
    /// - `packages/*` are workspaces
    ///
    /// When running `turbo run build -F ./*` from the `apps` directory:
    /// - The filter `./*` should match `app-a` and `app-b` (direct children)
    /// - It should NOT match `apps` itself
    /// - It should NOT match `app-a-client` (too deeply nested)
    #[test]
    fn test_subdirectory_filter_from_workspace_directory() {
        // Create the structure from the reproduction:
        // - apps (workspace itself)
        // - apps/app-a depends on pkg-a
        // - apps/app-b depends on pkg-c
        // - apps/app-a/app-a-client depends on app-a
        // - packages/pkg-a, pkg-b, pkg-c, tooling-config
        let (_tempdir, resolver) = make_project(
            &[
                ("apps/app-a", "packages/pkg-a"),
                ("apps/app-b", "packages/pkg-c"),
                ("apps/app-a/app-a-client", "apps/app-a"),
            ],
            &[
                "apps", // apps directory is ALSO a workspace
                "packages/pkg-b",
                "packages/tooling-config",
            ],
            // Simulate running from apps directory
            // PackageInference::calculate would set package_name to Some("apps")
            // because apps is a workspace at that path
            Some(PackageInference {
                package_name: Some("apps".to_string()),
                directory_root: AnchoredSystemPathBuf::try_from("apps").unwrap(),
            }),
            TestChangeDetector::new(&[]),
        );

        // Filter "./*" means: packages matching "./*" relative to current directory
        // (apps) This should resolve to "apps/*" and match app-a, app-b
        let selector = TargetSelector::from_str("./*").unwrap();
        let packages = resolver.get_filtered_packages(vec![selector]).unwrap();

        // We should get app-a and app-b
        assert!(
            packages.contains_key(&PackageName::from("app-a")),
            "Expected 'app-a' to be selected with filter './*' from apps directory. Got: {:?}",
            packages.keys().collect::<Vec<_>>()
        );
        assert!(
            packages.contains_key(&PackageName::from("app-b")),
            "Expected 'app-b' to be selected with filter './*' from apps directory. Got: {:?}",
            packages.keys().collect::<Vec<_>>()
        );

        // Should NOT include the apps package itself (it's at "apps", not "apps/*")
        assert!(
            !packages.contains_key(&PackageName::from("apps")),
            "The 'apps' package itself should not match './*' filter"
        );

        // Should NOT include nested packages like app-a-client
        assert!(
            !packages.contains_key(&PackageName::from("app-a-client")),
            "Deeply nested 'app-a-client' should not match './*' filter"
        );

        // Should NOT include packages outside apps/
        assert!(
            !packages.contains_key(&PackageName::from("pkg-a")),
            "pkg-a should not match './*' filter from apps directory"
        );
    }
}
