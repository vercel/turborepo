//! Typed, consumer-oriented projections of normalized package relationships.

use std::{collections::HashMap, sync::Arc};

use super::PackageName;
use crate::{
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    relationships::{DependencyKind, RelationshipTarget},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ScopeId(usize);

impl ScopeId {
    fn index(self) -> usize {
        self.0
    }
}

type Adjacency = Box<[Box<[ScopeId]>]>;

/// Controls whether package pruning follows development dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneDependencyMode {
    /// Follow production, development, and required internal peer
    /// relationships.
    IncludeDevDependencies,
    /// Follow production and required internal peer relationships only.
    ProductionOnly,
}

/// An invalid package identity supplied to a batch relationship query.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelationshipProjectionError {
    /// The package is not an authoritative package scope or the always-present
    /// root Turbo task namespace.
    #[error("unknown package {0}")]
    UnknownPackage(PackageName),
}

#[derive(Debug)]
struct RelationshipIndex {
    names: Box<[PackageName]>,
    non_root_lookup: HashMap<Box<str>, ScopeId>,
    ordering: Adjacency,
    reverse_ordering: Adjacency,
    prune_all: Adjacency,
    prune_production: Adjacency,
    root: ScopeId,
    root_inputs: Box<[ScopeId]>,
    is_root_input: Box<[bool]>,
}

impl RelationshipIndex {
    fn build(repository: &RepositoryKnowledge, relationships: &RelationshipKnowledge) -> Self {
        // Root is the always-present Turbo task namespace, not PackageNode::Root.
        let mut names = vec![PackageName::Root];
        names.extend(
            repository
                .scopes()
                .map(|scope| PackageName::Other(scope.identity().to_string())),
        );
        names[1..].sort();
        let names = names.into_boxed_slice();
        let non_root_lookup: HashMap<_, _> = names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| match name {
                PackageName::Root => None,
                PackageName::Other(name) => Some((name.clone().into_boxed_str(), ScopeId(index))),
            })
            .collect();
        let root = ScopeId(0);
        let mut ordering = vec![Vec::new(); names.len()];
        let mut prune_all = vec![Vec::new(); names.len()];
        let mut prune_production = vec![Vec::new(); names.len()];

        for group in relationships.groups() {
            let Some(source) = lookup_identity(&non_root_lookup, root, group.source()) else {
                unreachable!("validated relationship source must have a scope ID")
            };
            let mut seen_declarations = std::collections::HashSet::new();
            let mut seen_targets = std::collections::HashSet::new();
            let mut effective_concrete = Vec::new();
            let mut required_peers = Vec::new();

            for relationship in group.relationships() {
                // Existing prune semantics intentionally use the required peer's
                // declaration name, independently of classification and ordinary
                // declaration precedence. This retains an authoritative same-name
                // workspace even when the peer specifier was classified external
                // or a duplicate dev declaration won the graph projection.
                // Optional peers are never retained here.
                if relationship.kind() == (DependencyKind::Peer { optional: false })
                    && let Some(target) =
                        lookup_identity(&non_root_lookup, root, relationship.declaration_name())
                {
                    required_peers.push(target);
                }

                if !seen_declarations.insert(relationship.declaration_name()) {
                    continue;
                }
                let kind = relationship.kind();
                let RelationshipTarget::Internal(target) = relationship.target() else {
                    continue;
                };
                if !matches!(
                    kind,
                    DependencyKind::Production
                        | DependencyKind::Optional
                        | DependencyKind::Development
                ) {
                    continue;
                }
                let Some(target) = lookup_identity(&non_root_lookup, root, target) else {
                    unreachable!("validated internal relationship target must have a scope ID")
                };
                if seen_targets.insert(target) {
                    effective_concrete.push((target, kind));
                }
            }

            ordering[source.index()].extend(effective_concrete.iter().map(|(target, _)| *target));
            prune_all[source.index()].extend(effective_concrete.iter().map(|(target, _)| *target));
            prune_all[source.index()].extend(required_peers.iter().copied());
            prune_production[source.index()].extend(
                effective_concrete
                    .iter()
                    .filter(|(_, kind)| {
                        matches!(kind, DependencyKind::Production | DependencyKind::Optional)
                    })
                    .map(|(target, _)| *target),
            );
            prune_production[source.index()].extend(required_peers);
        }

        let ordering = normalize_adjacency(ordering);
        let prune_all = normalize_adjacency(prune_all);
        let prune_production = normalize_adjacency(prune_production);
        let reverse_ordering = reverse_adjacency(names.len(), &ordering);
        let mut root_members = closure_members(&ordering, &[root]);
        root_members[root.index()] = false;
        let root_inputs = member_ids(&root_members);

        Self {
            names,
            non_root_lookup,
            ordering,
            reverse_ordering,
            prune_all,
            prune_production,
            root,
            root_inputs,
            is_root_input: root_members.into_boxed_slice(),
        }
    }

    fn id(&self, package: &PackageName) -> Option<ScopeId> {
        match package {
            PackageName::Root => Some(self.root),
            PackageName::Other(name) => self.non_root_lookup.get(name.as_str()).copied(),
        }
    }

    fn ids(&self, packages: &[PackageName]) -> Result<Vec<ScopeId>, RelationshipProjectionError> {
        packages
            .iter()
            .map(|package| {
                self.id(package)
                    .ok_or_else(|| RelationshipProjectionError::UnknownPackage(package.clone()))
            })
            .collect()
    }

    fn transitive_dependencies(&self, package: &PackageName) -> Option<Vec<PackageName>> {
        let package = self.id(package)?;
        let mut members = closure_members(&self.ordering, &[package]);
        include_ids(&mut members, &self.root_inputs);
        members[package.index()] = false;
        Some(self.names_from_members(&members))
    }

    fn transitive_dependents(&self, package: &PackageName) -> Option<Vec<PackageName>> {
        let package = self.id(package)?;
        let mut members = if self.is_root_input[package.index()] {
            vec![true; self.names.len()]
        } else {
            closure_members(&self.reverse_ordering, &[package])
        };
        members[package.index()] = false;
        Some(self.names_from_members(&members))
    }

    fn names_from_members(&self, members: &[bool]) -> Vec<PackageName> {
        members
            .iter()
            .enumerate()
            .filter(|(_, included)| **included)
            .map(|(index, _)| self.names[index].clone())
            .collect()
    }

    fn names_from_ids(&self, ids: &[ScopeId]) -> Vec<PackageName> {
        ids.iter()
            .map(|id| self.names[id.index()].clone())
            .collect()
    }
}

fn lookup_identity(
    non_root_lookup: &HashMap<Box<str>, ScopeId>,
    root: ScopeId,
    identity: &str,
) -> Option<ScopeId> {
    if identity == super::ROOT_PKG_NAME {
        Some(root)
    } else {
        non_root_lookup.get(identity).copied()
    }
}

fn normalize_adjacency(mut adjacency: Vec<Vec<ScopeId>>) -> Adjacency {
    adjacency.iter_mut().for_each(|targets| {
        targets.sort_unstable_by_key(|target| target.index());
        targets.dedup();
    });
    adjacency
        .into_iter()
        .map(Vec::into_boxed_slice)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn reverse_adjacency(scope_count: usize, adjacency: &Adjacency) -> Adjacency {
    let mut reverse = vec![Vec::new(); scope_count];
    for (source, targets) in adjacency.iter().enumerate() {
        for target in targets {
            reverse[target.index()].push(ScopeId(source));
        }
    }
    normalize_adjacency(reverse)
}

fn closure_members(adjacency: &Adjacency, seeds: &[ScopeId]) -> Vec<bool> {
    let mut visited = vec![false; adjacency.len()];
    let mut stack = Vec::with_capacity(seeds.len());
    for seed in seeds {
        if !visited[seed.index()] {
            visited[seed.index()] = true;
            stack.push(*seed);
        }
    }
    while let Some(current) = stack.pop() {
        for target in &adjacency[current.index()] {
            if !visited[target.index()] {
                // Mark on enqueue so duplicate paths cannot grow the stack.
                visited[target.index()] = true;
                stack.push(*target);
            }
        }
    }
    visited
}

fn include_ids(members: &mut [bool], ids: &[ScopeId]) {
    for id in ids {
        members[id.index()] = true;
    }
}

fn member_ids(members: &[bool]) -> Box<[ScopeId]> {
    members
        .iter()
        .enumerate()
        .filter(|(_, included)| **included)
        .map(|(index, _)| ScopeId(index))
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

/// Direct graph-forming package relationships used for task ordering.
///
/// Production and development declarations participate with first-declaration
/// and first-target precedence. Peer relationships, unresolved externals, and
/// the package graph's structural root sentinel are excluded. Results are
/// sorted and deduplicated.
#[derive(Debug, Clone)]
pub struct OrderingRelationships(Arc<RelationshipIndex>);

impl OrderingRelationships {
    /// Returns direct internal dependencies, or `None` for an unknown package.
    ///
    /// The root Turbo namespace is always recognized. In a pure native
    /// repository it has no declared dependencies. The opaque iterator borrows
    /// the index and has an exact size; items are sorted and deduplicated.
    pub fn direct_dependencies(
        &self,
        package: &PackageName,
    ) -> Option<impl ExactSizeIterator<Item = &PackageName> + DoubleEndedIterator + Clone + '_>
    {
        let id = self.0.id(package)?;
        Some(
            self.0.ordering[id.index()]
                .iter()
                .map(|dependency| &self.0.names[dependency.index()]),
        )
    }
}

/// Transitive package relationships used by package filtering.
///
/// Dependency queries include the root JavaScript package's transitive internal
/// dependencies as implied dependencies. Dependent queries preserve the
/// corresponding all-packages behavior for those root inputs.
#[derive(Debug, Clone)]
pub struct FilteringRelationships(Arc<RelationshipIndex>);

impl FilteringRelationships {
    /// Returns sorted transitive dependencies excluding `package`, or `None`
    /// when the package is unknown.
    pub fn transitive_dependencies(&self, package: &PackageName) -> Option<Vec<PackageName>> {
        self.0.transitive_dependencies(package)
    }

    /// Returns sorted transitive dependents excluding `package`, or `None` when
    /// the package is unknown. A root internal input has every other
    /// authoritative identity as a dependent.
    pub fn transitive_dependents(&self, package: &PackageName) -> Option<Vec<PackageName>> {
        self.0.transitive_dependents(package)
    }

    /// Returns one sorted, deduplicated dependency closure including all seeds.
    ///
    /// The traversal is performed once for the complete seed set. Empty input
    /// returns an empty vector. If any seed is unknown, no partial result is
    /// returned.
    pub fn dependency_closure(
        &self,
        packages: &[PackageName],
    ) -> Result<Vec<PackageName>, RelationshipProjectionError> {
        let seeds = self.0.ids(packages)?;
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let mut members = closure_members(&self.0.ordering, &seeds);
        include_ids(&mut members, &self.0.root_inputs);
        Ok(self.0.names_from_members(&members))
    }
}

/// Reverse ordering relationships used to determine affected packages.
#[derive(Debug, Clone)]
pub struct AffectedRelationships(Arc<RelationshipIndex>);

impl AffectedRelationships {
    /// Returns changed seeds and all transitive ordering dependents.
    ///
    /// If any seed is a root internal input, every authoritative identity,
    /// including the root Turbo namespace, is affected. Empty input is valid.
    /// If any seed is unknown, no partial result is returned. Output is sorted
    /// and deduplicated.
    pub fn affected_by(
        &self,
        packages: &[PackageName],
    ) -> Result<Vec<PackageName>, RelationshipProjectionError> {
        let seeds = self.0.ids(packages)?;
        if seeds.iter().any(|seed| self.0.is_root_input[seed.index()]) {
            return Ok(self.0.names.to_vec());
        }
        let members = closure_members(&self.0.reverse_ordering, &seeds);
        Ok(self.0.names_from_members(&members))
    }
}

/// Internal dependency inputs used by derived task hashing and I/O.
///
/// External lockfile closures are deliberately outside this projection.
#[derive(Debug, Clone)]
pub struct HashRelationships(Arc<RelationshipIndex>);

impl HashRelationships {
    /// Returns the full transitive internal dependency inputs, including
    /// implied root inputs and excluding `package`, or `None` if unknown.
    /// Output is sorted and deduplicated.
    pub fn dependency_inputs(&self, package: &PackageName) -> Option<Vec<PackageName>> {
        self.0.transitive_dependencies(package)
    }

    /// Returns the sorted, deduplicated transitive internal inputs declared by
    /// the root JavaScript package. Pure native repositories return an empty
    /// vector while still recognizing [`PackageName::Root`].
    pub fn root_inputs(&self) -> Vec<PackageName> {
        self.0.names_from_ids(&self.0.root_inputs)
    }
}

/// Install-oriented package relationships used by pruning.
///
/// Unlike ordering, pruning independently resolves a required peer's
/// declaration name to an authoritative same-name workspace, regardless of
/// relationship classification or ordinary graph precedence. This mirrors
/// existing prune behavior for incompatible peer specifiers and aliases.
/// Optional peers are excluded from this package-only closure.
#[derive(Debug, Clone)]
pub struct PruneRelationships(Arc<RelationshipIndex>);

impl PruneRelationships {
    /// Returns a sorted, deduplicated package closure for `mode`.
    ///
    /// The root Turbo namespace is always an implicit seed, including in pure
    /// native repositories, and its mode-appropriate dependency closure is
    /// included. Empty input therefore returns at least the root namespace. The
    /// complete seed set is traversed once. If any explicit seed is unknown, no
    /// partial result is returned.
    pub fn package_closure(
        &self,
        packages: &[PackageName],
        mode: PruneDependencyMode,
    ) -> Result<Vec<PackageName>, RelationshipProjectionError> {
        let mut seeds = self.0.ids(packages)?;
        seeds.push(self.0.root);
        let adjacency = match mode {
            PruneDependencyMode::IncludeDevDependencies => &self.0.prune_all,
            PruneDependencyMode::ProductionOnly => &self.0.prune_production,
        };
        let members = closure_members(adjacency, &seeds);
        Ok(self.0.names_from_members(&members))
    }
}

#[derive(Debug)]
pub(super) struct RelationshipProjections {
    ordering: OrderingRelationships,
    filtering: FilteringRelationships,
    affected: AffectedRelationships,
    hash: HashRelationships,
    prune: PruneRelationships,
}

impl RelationshipProjections {
    pub(super) fn build(
        repository: &RepositoryKnowledge,
        relationships: &RelationshipKnowledge,
    ) -> Self {
        let index = Arc::new(RelationshipIndex::build(repository, relationships));
        Self {
            ordering: OrderingRelationships(Arc::clone(&index)),
            filtering: FilteringRelationships(Arc::clone(&index)),
            affected: AffectedRelationships(Arc::clone(&index)),
            hash: HashRelationships(Arc::clone(&index)),
            prune: PruneRelationships(index),
        }
    }

    pub(super) fn ordering(&self) -> &OrderingRelationships {
        &self.ordering
    }

    pub(super) fn filtering(&self) -> &FilteringRelationships {
        &self.filtering
    }

    pub(super) fn affected(&self) -> &AffectedRelationships {
        &self.affected
    }

    pub(super) fn hash(&self) -> &HashRelationships {
        &self.hash
    }

    pub(super) fn prune(&self) -> &PruneRelationships {
        &self.prune
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{
            PackageScopeObservation, RelationshipGroup, ScopeKind, WorkspaceRootObservation,
        },
        relationships::Relationship,
        toolchain::{ToolchainId, WorkspaceRoot},
    };

    fn name(value: &str) -> PackageName {
        PackageName::Other(value.to_string())
    }

    fn names(values: &[&str]) -> Vec<PackageName> {
        values.iter().map(|value| name(value)).collect()
    }

    fn with_root(mut values: Vec<PackageName>) -> Vec<PackageName> {
        values.push(PackageName::Root);
        values.sort();
        values
    }

    fn fixture(with_javascript_root: bool) -> RelationshipProjections {
        let root = AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" })
            .expect("test root is absolute");
        let identities = [
            "app",
            "dev",
            "optional",
            "prod",
            "required-peer",
            "optional-peer",
            "transitive",
            "root-lib",
            "cycle-a",
            "cycle-b",
            "disconnected",
        ];
        let toolchain = if with_javascript_root {
            ToolchainId::JAVASCRIPT
        } else {
            ToolchainId::RUST
        };
        let observations: Vec<_> = identities
            .iter()
            .map(|identity| PackageScopeObservation {
                identity: Some((*identity).to_string()),
                name_source: None,
                definition_path: root.join_components(&[identity, "package.json"]),
                toolchain: toolchain.clone(),
                scope_kind: ScopeKind::Package,
            })
            .collect();
        let repository = RepositoryKnowledge::build(
            &root,
            with_javascript_root.then_some(Some("root".to_string())),
            &observations,
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new(
                    if with_javascript_root { "npm" } else { "cargo" },
                    root.clone(),
                ),
                toolchain,
            )],
        )
        .expect("fixture repository is valid");
        let mut groups = vec![
            RelationshipGroup::new(
                "app",
                vec![
                    Relationship::internal("prod", DependencyKind::Production),
                    Relationship::internal("dev", DependencyKind::Development),
                    Relationship::internal("optional", DependencyKind::Production),
                    Relationship::internal("required-peer", DependencyKind::Development),
                    Relationship::internal(
                        "optional-peer",
                        DependencyKind::Peer { optional: true },
                    ),
                    // The same-name dev declaration wins graph precedence, but
                    // production prune still retains the authoritative workspace
                    // by the required peer's declaration name. Its incompatible
                    // specifier/classification is intentionally irrelevant.
                    Relationship::new(
                        "required-peer",
                        DependencyKind::Peer { optional: false },
                        RelationshipTarget::UnresolvedExternal {
                            name: "required-peer".to_string(),
                            specifier: "^999.0.0".to_string(),
                        },
                    ),
                    Relationship::new(
                        "prod-alias",
                        DependencyKind::Development,
                        RelationshipTarget::Internal("prod".to_string()),
                    ),
                    Relationship::new(
                        "dev-prod-alias",
                        DependencyKind::Production,
                        RelationshipTarget::Internal("dev".to_string()),
                    ),
                    Relationship::new(
                        "external",
                        DependencyKind::Production,
                        RelationshipTarget::UnresolvedExternal {
                            name: "external".to_string(),
                            specifier: "1.0.0".to_string(),
                        },
                    ),
                ],
            ),
            RelationshipGroup::new(
                "prod",
                vec![Relationship::internal(
                    "transitive",
                    DependencyKind::Production,
                )],
            ),
            RelationshipGroup::new(
                "cycle-a",
                vec![
                    Relationship::internal("cycle-a", DependencyKind::Production),
                    Relationship::internal("cycle-b", DependencyKind::Production),
                ],
            ),
            RelationshipGroup::new(
                "cycle-b",
                vec![Relationship::internal(
                    "cycle-a",
                    DependencyKind::Production,
                )],
            ),
        ];
        if with_javascript_root {
            groups.push(RelationshipGroup::new(
                "//",
                vec![Relationship::internal(
                    "root-lib",
                    DependencyKind::Production,
                )],
            ));
        }
        let relationships =
            RelationshipKnowledge::build(&repository, groups).expect("relationships are valid");
        RelationshipProjections::build(&repository, &relationships)
    }

    #[test]
    fn projections_match_order_filter_affected_hash_and_prune_semantics() {
        let projections = fixture(true);
        let app = name("app");

        assert_eq!(
            projections
                .ordering()
                .direct_dependencies(&app)
                .map(|dependencies| dependencies.cloned().collect::<Vec<_>>()),
            Some(names(&["dev", "optional", "prod", "required-peer"]))
        );
        assert_eq!(
            projections
                .ordering()
                .direct_dependencies(&name("disconnected"))
                .map(|dependencies| dependencies.cloned().collect::<Vec<_>>()),
            Some(Vec::new())
        );
        let transitive = names(&[
            "dev",
            "optional",
            "prod",
            "required-peer",
            "root-lib",
            "transitive",
        ]);
        assert_eq!(
            projections.filtering().transitive_dependencies(&app),
            Some(transitive.clone())
        );
        assert_eq!(projections.hash().dependency_inputs(&app), Some(transitive));
        assert_eq!(projections.hash().root_inputs(), names(&["root-lib"]));
        assert_eq!(
            projections
                .filtering()
                .dependency_closure(std::slice::from_ref(&app)),
            Ok(names(&[
                "app",
                "dev",
                "optional",
                "prod",
                "required-peer",
                "root-lib",
                "transitive"
            ]))
        );
        assert_eq!(
            projections.affected().affected_by(&[name("transitive")]),
            Ok(names(&["app", "prod", "transitive"]))
        );
        assert_eq!(
            projections.prune().package_closure(
                std::slice::from_ref(&app),
                PruneDependencyMode::IncludeDevDependencies,
            ),
            Ok(with_root(names(&[
                "app",
                "dev",
                "optional",
                "prod",
                "required-peer",
                "root-lib",
                "transitive",
            ])))
        );
        assert_eq!(
            projections
                .prune()
                .package_closure(&[app], PruneDependencyMode::ProductionOnly,),
            Ok(with_root(names(&[
                "app",
                "optional",
                "prod",
                "required-peer",
                "root-lib",
                "transitive",
            ])))
        );
    }

    #[test]
    fn root_implication_cycles_self_edges_and_unknown_batches_are_exact() {
        let projections = fixture(true);
        let root_lib = name("root-lib");
        let cycle_a = name("cycle-a");
        let all_but_root_lib = projections
            .filtering()
            .transitive_dependents(&root_lib)
            .expect("root-lib is authoritative");
        assert_eq!(all_but_root_lib.len(), 11);
        assert!(!all_but_root_lib.contains(&root_lib));
        assert_eq!(
            projections.filtering().transitive_dependencies(&cycle_a),
            Some(names(&["cycle-b", "root-lib"]))
        );
        assert_eq!(
            projections.prune().package_closure(
                std::slice::from_ref(&cycle_a),
                PruneDependencyMode::IncludeDevDependencies,
            ),
            Ok(with_root(names(&["cycle-a", "cycle-b", "root-lib"])))
        );
        assert_eq!(
            projections.affected().affected_by(&[root_lib]),
            Ok(with_root(names(&[
                "app",
                "cycle-a",
                "cycle-b",
                "dev",
                "disconnected",
                "optional",
                "optional-peer",
                "prod",
                "required-peer",
                "root-lib",
                "transitive",
            ])))
        );
        let unknown = name("missing");
        assert_eq!(
            projections
                .filtering()
                .dependency_closure(&[cycle_a.clone(), unknown.clone()]),
            Err(RelationshipProjectionError::UnknownPackage(unknown.clone()))
        );
        assert_eq!(
            projections
                .affected()
                .affected_by(&[cycle_a.clone(), unknown.clone()]),
            Err(RelationshipProjectionError::UnknownPackage(unknown.clone()))
        );
        assert_eq!(
            projections.prune().package_closure(
                &[cycle_a, unknown.clone()],
                PruneDependencyMode::IncludeDevDependencies,
            ),
            Err(RelationshipProjectionError::UnknownPackage(unknown))
        );
    }

    #[test]
    fn pure_native_generation_keeps_root_namespace_without_declared_edges() {
        let projections = fixture(false);
        assert_eq!(
            projections
                .ordering()
                .direct_dependencies(&PackageName::Root)
                .map(|dependencies| dependencies.cloned().collect::<Vec<_>>()),
            Some(Vec::new())
        );
        assert!(projections.hash().root_inputs().is_empty());
        assert_eq!(
            projections
                .prune()
                .package_closure(&[], PruneDependencyMode::IncludeDevDependencies,),
            Ok(vec![PackageName::Root])
        );
        assert_eq!(
            projections.affected().affected_by(&[name("disconnected")]),
            Ok(names(&["disconnected"]))
        );
        assert_eq!(
            projections.filtering().dependency_closure(&[]),
            Ok(Vec::new())
        );
    }
}
