//! Subprocess-free static discovery for Go workspaces.
//!
//! [`super::discover_workspace`] delegates module membership, module paths,
//! replacements, and internal edges to the `go` command (`go work edit -json`,
//! `go mod edit -json`, `go mod graph`, `go list -m all`). That is the
//! authoritative path used during preparation.
//!
//! Planning cannot invoke subprocesses, so this module reconstructs the same
//! observations by parsing `go.work` and `go.mod` directly:
//!
//! * `use` entries and `module` directives determine workspace membership and
//!   scope identities — the same set the authoritative path observes.
//! * `require` and `replace` directives reproduce the internal module graph
//!   exactly, as described below, or defer the scopes that require an ambiguous
//!   path when the native metadata needed to pick between outcomes is
//!   unavailable. A deferred edge contributes nothing — never a guessed target,
//!   not even the selected-version one — and is reported, never silently
//!   treated as "no edge".
//! * `runnable_target` is classified from Go sources by
//!   `runnable_target_statically`, which returns the exact package pattern for
//!   the `dev` task or defers the scope; it never guesses a catalogue.
//!
//! ## Scoped planning uncertainty
//!
//! Membership — which modules the workspace contains, their identities, and
//! their manifests — is always exact and is collected before anything can
//! defer. Each member's sources then decide two facts, its internal edges and
//! its `dev` target, and each unproven fact becomes a [`PlanningUncertainty`]
//! record scoped to the member whose sources produced it:
//!
//! * an edge record travels with the member's provable remaining edges and
//!   bounds the unresolved edge to where it could connect — the precise
//!   candidate targets when the ambiguity can be narrowed, every other member
//!   identity otherwise;
//! * a catalogue record covers the tasks the unproven `dev` target shapes:
//!   `dev` itself, and `build`, whose entrypoint, outputs, and cacheability
//!   follow the same fact.
//!
//! Structural failures — broken manifests, unresolvable replacements — still
//! fail discovery; only scoped facts defer. Core keeps the records with the
//! planning graph, ignores them for selections that never consult them,
//! refuses selections that cannot be proven exact, and resolves them by
//! preparing this toolchain when one of its own tasks is selected.
//!
//! ## The static prune domain
//!
//! Every Go scope's task contract references the `go` prune domain, so the
//! static observation contributes one built from static facts — membership,
//! directories, `go.work` directives, and replacements are exact, and the
//! retention closure is exact unless an edge deferred. The domain refuses to
//! plan when its closure (or the `go` directive the pruned `go.work` needs)
//! is not provable, rather than silently under-retaining; preparation swaps
//! it for the authoritative domain, and the eager prune path always uses full
//! discovery's.
//!
//! ## Exact internal edges
//!
//! Go resolves modules in workspace mode with these rules, which this module
//! reproduces:
//!
//! 1. A require on a path that is a workspace member resolves to that member.
//!    Membership wins over any replacement, matching `go mod graph`, whose
//!    `from`/`to` module paths are matched against membership before
//!    replacement targets.
//! 2. Otherwise, the replacement effective at the selected version applies.
//!    Precedence is evaluated per version, not per old path: a `go.work`
//!    replacement for one version does not suppress a member replacement for
//!    another version. Within a tier the order is `go.work` exact version,
//!    `go.work` wildcard, then directives from every main module (replacements
//!    from all main modules apply workspace-wide), where within one module an
//!    exact version overrides that module's wildcard. Directives that agree on
//!    one target are deduplicated; effective replacements that conflict on the
//!    target are a broken workspace, which fails discovery exactly as the `go`
//!    command rejects conflicting replacements under every environment.
//!
//! Only local (directory) replacements can resolve to a workspace member,
//! mirroring the authoritative path, which derives replacement targets from
//! resolved replacement directories on `go list -m all`. A replacement to a
//! remote module path stays external.
//!
//! The selected version for a required path is the maximum version required
//! across every main manifest. Every local replacement target is a workspace
//! member, so its manifest contributes requirements as well and the local
//! graph is closed: minimum version selection cannot exceed that maximum,
//! which makes the selected replacement — and therefore every internal edge —
//! exact rather than a conservative superset. Replacements for versions
//! provably below the selected one can never apply and are ignored.
//!
//! ## Remote requirements
//!
//! A selected dependency that is neither a workspace identity nor a selected
//! local replacement is remote, and its `go.mod` is unavailable to planning. A
//! remote transitive requirement may raise any selected version. When a remote
//! dependency is active, each required path is checked against every future
//! higher replacement version and the version-less wildcard tier: if any of
//! them would change the internal target, the scopes that require the path
//! defer rather than guessing an edge set. Hypothetical versions are
//! evaluated for that deferral only; they never contribute edges. Without an
//! active remote requirement the graph is closed and the selected versions
//! are exact, so no hypothetical version is evaluated.
//!
//! Every `go.work` local replacement is validated even when no member requires
//! its old path, because the authoritative path validates all of them while
//! building prune knowledge and a versioned remote replacement stays external.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use super::{
    Error, GO_MOD, GO_WORK, GO_WORKSPACE_NAME, GoModule, GoPruneKnowledge, GoPruneReplacement,
    join_relative_path, path_is_within_repository, resolve_member_dir, validate_single_workspace,
};
use crate::{
    relationships::{DependencyKind, Relationship, RelationshipTarget},
    toolchain::{PlanningUncertainty, PlanningUncertaintyKind},
};

/// A require directive from a `go.mod`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GoRequire {
    path: String,
    version: Option<String>,
}

/// A `replace` directive from `go.work` or `go.mod`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GoReplaceDirective {
    old_path: String,
    old_version: Option<String>,
    new_path: String,
    new_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedGoWork {
    uses: Vec<String>,
    replaces: Vec<GoReplaceDirective>,
    /// The `go` directive's version, which the pruned `go.work` must carry.
    go: Option<String>,
    /// The optional `toolchain` directive.
    toolchain: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedGoMod {
    module_path: Option<String>,
    requires: Vec<GoRequire>,
    replaces: Vec<GoReplaceDirective>,
}

/// The static planning observation of a Go workspace.
#[derive(Debug, Default)]
pub(super) struct StaticWorkspace {
    /// Workspace members with the edges and runnable targets that are proven
    /// under every possible resolution — a provable subset. Scope identities
    /// match the authoritative observation; only environment-dependent
    /// metadata stays provisional until preparation.
    pub modules: Vec<GoModule>,
    /// The planning facts this observation could not prove, each scoped to a
    /// member identity it contributes (see [`PlanningUncertainty`]). Empty
    /// when every fact is exact. Structural workspace failures never reach
    /// this shape: they fail discovery instead.
    pub uncertainties: Vec<PlanningUncertainty>,
    /// The prune domain every Go scope's task contract references. Static
    /// facts build the same domain shape preparation replaces; the domain
    /// refuses to plan only when its retention closure is not provable.
    /// `None` only when there is no `go.work` at all, in which case no Go
    /// scope exists to reference it.
    pub prune: Option<GoPruneKnowledge>,
}

/// One required module path whose internal edge could not be proven, with
/// the bound on where the unresolved edge could connect. The path itself has
/// no single owner, so every member that requires it inherits the deferral
/// as its own scoped uncertainty; the path is named in the message.
#[derive(Debug, Clone)]
struct PathUncertainty {
    /// Stable machine-readable classifier for diagnostics.
    code: &'static str,
    /// Human explanation, written relative to the requiring scope ("its
    /// requirement on the required path ...").
    message: String,
    /// The member identities the unresolved edge could connect to; `None`
    /// means the deferral could not narrow the bound, which resolves to every
    /// member identity when the record is scoped.
    possible_targets: Option<BTreeSet<String>>,
}

impl PathUncertainty {
    /// The scoped record for one member that requires this path: the member
    /// owns the fact, and the bound resolves to every member identity when
    /// the deferral could not narrow it. The member contributes no edge for
    /// the path — never the selected-version target as a guess.
    fn scoped_record(
        &self,
        owner: &str,
        member_identities: &HashSet<String>,
    ) -> PlanningUncertainty {
        PlanningUncertainty::internal_edges(owner, self.code, self.message.clone())
            .with_possible_targets(self.possible_targets.clone().unwrap_or_else(|| {
                let mut identities: BTreeSet<String> = member_identities.iter().cloned().collect();
                // A bound is a set of candidate identities, so the owner is
                // not a candidate for its own unresolved edge.
                identities.remove(owner);
                identities
            }))
    }
}

/// The static resolution of one required module path: the exact internal
/// target set — empty when the dependency stays remote, which is itself
/// exact — or the deferral members that require the path inherit.
#[derive(Debug, Clone)]
enum PathResolution {
    Targets(BTreeSet<String>),
    Deferred(PathUncertainty),
}

/// A member of the workspace inventory, collected before any per-scope
/// classification can defer.
struct Member {
    module_path: String,
    manifest_path: AbsoluteSystemPathBuf,
    directory: AbsoluteSystemPathBuf,
    requires: Vec<GoRequire>,
    replaces: Vec<GoReplaceDirective>,
}

/// The exact membership inventory of a workspace.
struct Membership {
    /// Members in `go.work` `use` order.
    members: Vec<Member>,
    /// Member module paths.
    paths: HashSet<String>,
    /// Member module paths by real (canonical) directory, for resolving
    /// local replacements to members.
    real_directories: HashMap<AbsoluteSystemPathBuf, String>,
}

/// Collect the workspace's membership inventory: identities, manifests, and
/// the directives that decide per-scope classification.
///
/// Membership is always exact. Broken workspaces — a missing or malformed
/// `go.mod`, a duplicate identity, a vendored member, a member outside the
/// repository — fail here, before any per-scope deferral, because even the
/// inventory would be untrustworthy.
fn collect_members(
    repo_root: &AbsoluteSystemPath,
    uses: &[String],
    root_module_path: Option<&str>,
) -> Result<Membership, Error> {
    let mut members: Vec<Member> = Vec::new();
    let mut identities: HashMap<String, String> = HashMap::new();
    let mut paths: HashSet<String> = HashSet::new();
    let mut real_directories: HashMap<AbsoluteSystemPathBuf, String> = HashMap::new();

    for disk_path in uses {
        let member_dir = resolve_member_dir(repo_root, disk_path)?;
        let manifest = member_dir.join_component(GO_MOD);
        if !manifest.exists() {
            return Err(Error::MissingGoMod {
                path: member_dir.to_string(),
            });
        }
        if member_dir.join_component("vendor").exists() {
            return Err(Error::VendoredModule {
                path: member_dir.join_component("vendor").to_string(),
            });
        }

        let parsed = parse_go_mod(&read_manifest(&manifest)?, &manifest)?;
        let module_path = required_module_path(&parsed, &manifest)?.to_string();
        if module_path == GO_WORKSPACE_NAME {
            return Err(Error::WorkspaceNameCollision {
                name: GO_WORKSPACE_NAME.to_string(),
            });
        }
        if let Some(root_path) = root_module_path
            && root_path == module_path
        {
            return Err(Error::RootDefinitionCollision {
                path: member_dir.to_string(),
                module_path,
            });
        }
        if let Some(other_manifest) = identities.get(&module_path) {
            return Err(Error::DuplicateModuleIdentity {
                module_path,
                other_manifest: other_manifest.clone(),
            });
        }
        identities.insert(module_path.clone(), manifest.to_string());
        paths.insert(module_path.clone());
        real_directories.insert(member_dir.to_realpath()?, module_path.clone());

        members.push(Member {
            module_path,
            manifest_path: manifest,
            directory: member_dir,
            requires: parsed.requires,
            replaces: parsed.replaces,
        });
    }

    Ok(Membership {
        members,
        paths,
        real_directories,
    })
}

/// Discover Go modules without invoking the `go` command.
pub(super) fn discover_workspace_statically(
    repo_root: &AbsoluteSystemPath,
) -> Result<StaticWorkspace, Error> {
    let work_path = repo_root.join_component(GO_WORK);
    if !work_path.exists() {
        return Ok(StaticWorkspace::default());
    }

    validate_single_workspace(repo_root)?;
    let work = parse_go_work(&read_manifest(&work_path)?, &work_path)?;
    if work.uses.is_empty() {
        return Err(Error::EmptyWorkspace);
    }

    let root_module_path = if repo_root.join_component(GO_MOD).exists() {
        let manifest = repo_root.join_component(GO_MOD);
        let parsed = parse_go_mod(&read_manifest(&manifest)?, &manifest)?;
        Some(required_module_path(&parsed, &manifest)?.to_string())
    } else {
        None
    };

    // Membership is always exact and always collected first, so the minimal
    // inventory exists before any per-scope classification can defer.
    let Membership {
        members,
        paths: member_paths,
        real_directories: member_real_directories,
    } = collect_members(repo_root, &work.uses, root_module_path.as_deref())?;

    // `go.work` replacements override module replacements, so a module
    // replacement is only reachable when no workspace replacement names the
    // same old path. Both lists are ordered for deterministic diagnostics.
    let mut workspace_replaces = work.replaces;
    workspace_replaces.sort_by(replace_order);
    let mut module_replaces: Vec<(AbsoluteSystemPathBuf, GoReplaceDirective)> = members
        .iter()
        .flat_map(|member| {
            member
                .replaces
                .iter()
                .cloned()
                .map(|replace| (member.directory.clone(), replace))
        })
        .collect();
    module_replaces
        .sort_by(|left, right| replace_order(&left.1, &right.1).then_with(|| left.0.cmp(&right.0)));

    // The selected version for every required path is the maximum version
    // required across all main manifests. Local replacement targets are
    // members, so their manifests contribute requirements too and the maximum
    // closes the local graph. A path whose requirements cannot be ordered
    // defers its requirers instead of failing the whole workspace here.
    let mut selected_versions: HashMap<String, SelectedVersion> = HashMap::new();
    let mut unorderable_versions: HashMap<String, PathUncertainty> = HashMap::new();
    for member in &members {
        for require in &member.requires {
            // The parser rejects a `require` without a version; an absent one
            // contributes no ordering and leaves the wildcard tier as the
            // fallback for that path.
            if let Some(version) = require.version.as_deref() {
                raise_selected_version(
                    &mut selected_versions,
                    &mut unorderable_versions,
                    &require.path,
                    version,
                    &member.manifest_path,
                );
            }
        }
    }

    let resolver = DependencyResolver {
        repo_root,
        member_paths: &member_paths,
        member_real_directories: &member_real_directories,
        workspace_replaces: &workspace_replaces,
        module_replaces: &module_replaces,
        selected_versions: &selected_versions,
    };

    // `discover_workspace` resolves and validates every `go.work` local
    // replacement while building prune knowledge, even when no member requires
    // its old path. Static planning must fail the same way rather than quietly
    // skipping a replacement that preparation would reject. A versioned
    // replacement names a remote module and stays external.
    for replace in &workspace_replaces {
        if replace.new_version.is_some() {
            continue;
        }
        resolver.resolve(repo_root, replace, true)?;
    }

    // Resolve every required path once at its selected version. The result is
    // the single exact internal target, an empty set when the dependency
    // stays remote — which is itself exact — or the reason the path's edge
    // defers to native metadata, inherited by every member that requires it.
    let mut required_paths: Vec<&str> = members
        .iter()
        .flat_map(|member| member.requires.iter().map(|require| require.path.as_str()))
        .collect();
    required_paths.sort_unstable();
    required_paths.dedup();
    let mut resolutions: HashMap<&str, PathResolution> = HashMap::new();
    for path in &required_paths {
        let resolution = if let Some(uncertainty) = unorderable_versions.get(*path) {
            // The selected version itself is unknown, so the edge is.
            PathResolution::Deferred(uncertainty.clone())
        } else {
            resolver.selected_target(path)?
        };
        resolutions.insert(*path, resolution);
    }

    // A selected dependency that is neither a member identity nor a selected
    // local replacement is remote, and its unseen `go.mod` may raise any
    // selected version through transitive requirements. When raising a version
    // could change or remove a required path's internal edge, the path defers
    // its requirers: those members contribute no edge for the path — not the
    // selected-version target, which an upgrade could invalidate — and the
    // deferral is bounded to every target the edge could still connect to. The
    // check is deterministic and happens before any module is classified.
    // Without an active remote requirement the graph is closed and the
    // resolved targets are already exact, so no hypothetical version is
    // evaluated.
    let remote_active = resolutions.values().any(
        |resolution| matches!(resolution, PathResolution::Targets(targets) if targets.is_empty()),
    );
    if remote_active {
        for path in &required_paths {
            if member_paths.contains(*path) {
                // Membership is version-independent: no upgrade can change a
                // require that already names a member.
                continue;
            }
            let Some(PathResolution::Targets(targets)) = resolutions.get(*path) else {
                // Already deferred: the path carries its own diagnostic.
                continue;
            };
            if resolver.target_could_change_under_remote_upgrade(path, targets)? {
                let version = selected_versions
                    .get(*path)
                    .map(|selected| selected.version.as_str())
                    .unwrap_or("an unknown version");
                resolutions.insert(
                    *path,
                    PathResolution::Deferred(PathUncertainty {
                        code: "ambiguous-remote-replacement",
                        message: format!(
                            "its requirement on {path} cannot be proven exactly: an active remote \
                             requirement could raise the selected version {version} past a \
                             version-sensitive local replacement and change or remove the \
                             internal edge"
                        ),
                        // Precise bound: the selected target plus every
                        // target the not-provably-lower replacement versions
                        // and the version-less wildcard tier could select.
                        possible_targets: Some(
                            resolver.possible_targets_under_remote_upgrade(path, targets)?,
                        ),
                    }),
                );
            }
        }
    }

    // Classify each member's facts from its own sources: edges from the
    // per-path resolutions its manifest required, the `dev` target from its
    // Go sources. Each dimension defers independently, so a member keeps its
    // exact identity and every proven fact alongside what it could not prove,
    // which becomes a scoped record instead of a guessed fact or a global
    // failure.
    let mut modules = Vec::with_capacity(members.len());
    let mut uncertainties = Vec::new();
    for member in &members {
        let (targets, deferred) = classify_relationships(member, &resolutions);
        for uncertainty in deferred {
            // One record per required path whose edge could not be proven:
            // the member owns the fact, the bound is resolved to member
            // identities, and no edge is contributed for the path.
            uncertainties.push(uncertainty.scoped_record(&member.module_path, &member_paths));
        }

        let runnable_target = match runnable_target_statically(&member.directory) {
            Ok(target) => target,
            // Source classification — a build-constrained `main` package or a
            // `go.mod` `ignore` directive — could not prove the `dev` target.
            // The module contributes no `dev` task, and the catalogue record
            // also covers `build`, whose entrypoint, outputs, and cacheability
            // are shaped by the same unproven fact.
            Err(Error::StaticDiscoveryUnavailable { path, reason }) => {
                uncertainties.push(
                    PlanningUncertainty::task_catalogue(
                        member.module_path.clone(),
                        "constrained-runnable-target",
                        format!(
                            "the runnable target that shapes the `dev` and `build` tasks cannot \
                             be proven from source classification: {reason} ({path})"
                        ),
                    )
                    .with_uncertain_task_names(["dev", "build"]),
                );
                None
            }
            // Structural failures — unreadable manifests, malformed
            // directives — are workspace errors, not scoped uncertainty.
            Err(error) => return Err(error),
        };

        modules.push(GoModule {
            module_path: member.module_path.clone(),
            manifest_path: member.manifest_path.clone(),
            relationships: targets
                .into_iter()
                .map(|target| Relationship::internal(target, DependencyKind::Production))
                .collect(),
            runnable_target,
        });
    }

    // The static prune domain. Membership, directories, `go.work` directives,
    // and replacements are exact static facts, and the relationship closure
    // is exact unless an edge deferred — so planning contributes the same
    // domain shape preparation replaces, refusing to plan only when the
    // retention closure (or the `go` directive the pruned file needs) is not
    // provable. Without this domain, every Go scope's task contract would
    // reference a prune domain no observation retains.
    let mut package_directories = HashMap::with_capacity(members.len());
    for member in &members {
        package_directories.insert(
            member.module_path.clone(),
            AnchoredSystemPathBuf::new(repo_root, &member.directory)?
                .to_unix()
                .to_string(),
        );
    }
    let mut prune_replacements = Vec::with_capacity(workspace_replaces.len());
    for replace in &workspace_replaces {
        // A versioned replacement names a remote module and stays external;
        // a local one resolves to a member the discovery already validated.
        let local = if replace.new_version.is_none() {
            resolver.resolve(repo_root, replace, false)?
        } else {
            None
        };
        let (new_path, local_target) = match local {
            Some(target) => (format!("./{}", package_directories[&target]), Some(target)),
            None => (replace.new_path.clone(), None),
        };
        prune_replacements.push(GoPruneReplacement {
            old_path: replace.old_path.clone(),
            old_version: replace.old_version.clone(),
            new_path,
            new_version: replace.new_version.clone(),
            local_target,
        });
    }
    prune_replacements.sort_by(|left, right| {
        (
            &left.old_path,
            &left.old_version,
            &left.new_path,
            &left.new_version,
        )
            .cmp(&(
                &right.old_path,
                &right.old_version,
                &right.new_path,
                &right.new_version,
            ))
    });
    let relationships = modules
        .iter()
        .map(|module| {
            let dependencies = module
                .relationships
                .iter()
                .filter_map(|relationship| match relationship.target() {
                    RelationshipTarget::Internal(target) => Some(target.clone()),
                    RelationshipTarget::UnresolvedExternal { .. } => None,
                })
                .collect();
            (module.module_path.clone(), dependencies)
        })
        .collect();
    let deferred = if uncertainties
        .iter()
        .any(|uncertainty| uncertainty.kind() == PlanningUncertaintyKind::InternalEdges)
    {
        Some(
            "static planning deferred some internal edges to full Go discovery, so the prune \
             retention closure cannot be computed exactly"
                .to_string(),
        )
    } else if work.go.is_none() {
        Some(
            "go.work declares no `go` directive, so the pruned go.work cannot be rendered"
                .to_string(),
        )
    } else {
        None
    };
    let prune = GoPruneKnowledge {
        domain: crate::prune_knowledge::GO_PRUNE_DOMAIN.clone(),
        go: work.go.clone().unwrap_or_default(),
        toolchain: work.toolchain.clone(),
        package_directories,
        relationships,
        replacements: prune_replacements,
        deferred,
    };

    Ok(StaticWorkspace {
        modules,
        uncertainties,
        prune: Some(prune),
    })
}

/// Classify one member's internal edges: the subset proven under every
/// possible resolution, plus one deferral per required path whose edge could
/// not be proven.
///
/// The member's manifest requires are its own sources, so a deferred path
/// defers this member's edge for it — contributing no edge and no guessed
/// target, never the selected-version one — while requires that resolve
/// exactly contribute their edges. A member that never requires an ambiguous
/// path keeps its exact edges, including an exactly empty set, which is a
/// statement, not a placeholder. Self edges are dropped.
fn classify_relationships(
    member: &Member,
    resolutions: &HashMap<&str, PathResolution>,
) -> (BTreeSet<String>, Vec<PathUncertainty>) {
    let mut targets = BTreeSet::new();
    let mut deferred = Vec::new();
    for require in &member.requires {
        let Some(resolution) = resolutions.get(require.path.as_str()) else {
            // `resolutions` covers every member require by construction; a
            // gap would be an internal invariant failure, so it defers the
            // require rather than contributing an edge for it.
            deferred.push(PathUncertainty {
                code: "internal-resolution-invariant",
                message: format!(
                    "its requirement on {} could not be resolved by static planning; this is an \
                     internal invariant failure, not a source condition",
                    require.path
                ),
                possible_targets: None,
            });
            continue;
        };
        match resolution {
            PathResolution::Targets(resolved) => {
                // A member require maps directly to that member; any other
                // require resolves to its single selected target. Self edges
                // are dropped.
                for target in resolved {
                    if target != &member.module_path {
                        targets.insert(target.clone());
                    }
                }
            }
            // The edge for this require is unproven: it contributes nothing
            // here and is reported through the deferral the member owns.
            PathResolution::Deferred(uncertainty) => {
                deferred.push(uncertainty.clone());
            }
        }
    }
    (targets, deferred)
}

/// The selected version for one required module path: the maximum version
/// required across every main manifest, with the manifest that contributed it
/// for diagnostics.
#[derive(Debug, Clone)]
struct SelectedVersion {
    version: String,
    manifest: AbsoluteSystemPathBuf,
}

/// Fold one requirement into the per-path maximum required version.
///
/// Minimum version selection never chooses below the maximum direct
/// requirement, so only a strictly greater version raises the selection.
/// Versions that cannot be ordered, or that compare equal only through
/// normalization, defer the path's requirers instead of breaking the tie
/// arbitrarily: the first unorderable pair is recorded, and later
/// requirements of the same path need no further ordering.
fn raise_selected_version(
    selected: &mut HashMap<String, SelectedVersion>,
    unorderable: &mut HashMap<String, PathUncertainty>,
    path: &str,
    version: &str,
    manifest: &AbsoluteSystemPathBuf,
) {
    // A deferral already recorded for the path needs no further ordering.
    if unorderable.contains_key(path) {
        return;
    }
    let Some(current) = selected.get(path) else {
        selected.insert(
            path.to_string(),
            SelectedVersion {
                version: version.to_string(),
                manifest: manifest.clone(),
            },
        );
        return;
    };
    match compare_go_versions(&current.version, version) {
        // Only a strictly greater requirement raises the selection.
        Some(Ordering::Less) => {
            selected.insert(
                path.to_string(),
                SelectedVersion {
                    version: version.to_string(),
                    manifest: manifest.clone(),
                },
            );
        }
        Some(Ordering::Greater) => {}
        Some(Ordering::Equal) if current.version == version => {}
        // Versions that compare equal only through normalization have no
        // canonical form to choose without native metadata: the path's
        // requirers defer rather than receiving a guessed maximum.
        Some(Ordering::Equal) => {
            unorderable.insert(
                path.to_string(),
                PathUncertainty {
                    code: "unorderable-required-versions",
                    message: format!(
                        "its requirement on {path} cannot be proven exactly: main manifests \
                         require {version} ({}) and {} ({}), which differ despite comparing \
                         equal, so static planning will not choose a canonical form arbitrarily",
                        manifest.as_str(),
                        current.version,
                        current.manifest.as_str(),
                    ),
                    // No selected version exists to narrow replacements
                    // with; the bound is every member identity.
                    possible_targets: None,
                },
            );
        }
        None => {
            unorderable.insert(
                path.to_string(),
                PathUncertainty {
                    code: "unorderable-required-versions",
                    message: format!(
                        "its requirement on {path} cannot be proven exactly: main manifests \
                         require {version} ({}) and {} ({}), which static planning cannot order",
                        manifest.as_str(),
                        current.version,
                        current.manifest.as_str(),
                    ),
                    // No selected version exists to narrow replacements
                    // with; the bound is every member identity.
                    possible_targets: None,
                },
            );
        }
    }
}

struct DependencyResolver<'a> {
    repo_root: &'a AbsoluteSystemPath,
    member_paths: &'a HashSet<String>,
    member_real_directories: &'a HashMap<AbsoluteSystemPathBuf, String>,
    workspace_replaces: &'a [GoReplaceDirective],
    module_replaces: &'a [(AbsoluteSystemPathBuf, GoReplaceDirective)],
    /// The selected (maximum required) version per required module path.
    selected_versions: &'a HashMap<String, SelectedVersion>,
}

impl DependencyResolver<'_> {
    /// The resolution of one required path at its selected version: the member
    /// identity when the path is a member, the single replacement target when
    /// the effective replacement is local, an empty set when the dependency
    /// stays remote — which is itself exact — or a deferral when the effective
    /// replacements disagree on the target.
    ///
    /// The selected version definitely applies — every main manifest's
    /// requirement is already folded into it — so a broken, outside-repository,
    /// or non-member local replacement there is a real error. Effective
    /// replacements that agree on one target are deduplicated; replacements
    /// that disagree are a broken workspace, exactly as the `go` command
    /// rejects conflicting replacements under every environment.
    fn selected_target(&self, path: &str) -> Result<PathResolution, Error> {
        // Membership wins over replacements, matching `go mod graph`'s
        // membership check before its replacement lookup.
        if self.member_paths.contains(path) {
            return Ok(PathResolution::Targets([path.to_string()].into()));
        }

        let version = self
            .selected_versions
            .get(path)
            .map(|selected| selected.version.as_str());
        let mut targets = BTreeSet::new();
        for (base, replace) in self.directives_at(path, version) {
            if let Some(target) = self.resolve(base, replace, true)? {
                targets.insert(target);
            }
        }
        if targets.len() > 1 {
            let conflicting = targets
                .iter()
                .map(|target| format!("{target:?}"))
                .collect::<Vec<_>>()
                .join(" and ");
            // The `go` command rejects a workspace whose effective
            // replacements for one path disagree: `go work sync`, `go mod
            // graph`, and `go list -m all` all fail with "conflicting
            // replacements" under every environment. This is a broken
            // workspace, not a fact native metadata would resolve — full
            // discovery fails the same way — so it fails discovery instead
            // of deferring with a bound that could never be honest.
            return Err(Error::MalformedReplacement {
                reason: format!(
                    "the replacements effective for {path} at the selected version {} resolve to \
                     conflicting local targets {conflicting}; the go command rejects a workspace \
                     whose effective replacements disagree, so make one replacement win (for \
                     example with `go work edit -replace`)",
                    version.unwrap_or("an unknown version"),
                ),
            });
        }
        Ok(PathResolution::Targets(targets))
    }

    /// Whether an active remote requirement could raise `path` past its
    /// selected version into a replacement whose internal target differs.
    ///
    /// Every replacement version that is not provably below the selected one,
    /// plus the version-less wildcard tier, is evaluated hypothetically. The
    /// answer only ever defers an ambiguous graph; it never adds edges. A
    /// hypothetical that resolves to no target at all — the edge vanishes
    /// when the upgrade names no matching replacement — also differs from the
    /// selected set and defers.
    fn target_could_change_under_remote_upgrade(
        &self,
        path: &str,
        selected: &BTreeSet<String>,
    ) -> Result<bool, Error> {
        for candidate in self.upgrade_candidate_versions(path) {
            if &self.hypothetical_target(path, Some(&candidate))? != selected {
                return Ok(true);
            }
        }
        // A version no directive names — an external requirement may require
        // one — leaves only wildcard replacements able to match.
        Ok(&self.hypothetical_target(path, None)? != selected)
    }

    /// The member identities `path`'s unresolved edge could still connect to
    /// while an active remote requirement may raise its selected version: the
    /// selected targets plus every target the not-provably-lower replacement
    /// versions and the version-less wildcard tier would select. This bounds
    /// the deferral record; it never contributes edges.
    fn possible_targets_under_remote_upgrade(
        &self,
        path: &str,
        selected: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, Error> {
        let mut targets = selected.clone();
        for candidate in self.upgrade_candidate_versions(path) {
            targets.extend(self.hypothetical_target(path, Some(&candidate))?);
        }
        targets.extend(self.hypothetical_target(path, None)?);
        Ok(targets)
    }

    /// Replacement versions for `path` that a remote transitive requirement
    /// could still select: every named version that is not provably below the
    /// selected one.
    fn upgrade_candidate_versions(&self, path: &str) -> Vec<String> {
        let selected = self
            .selected_versions
            .get(path)
            .map(|selected| selected.version.as_str());
        let mut candidates: BTreeSet<&str> = BTreeSet::new();
        let directives = self
            .workspace_replaces
            .iter()
            .chain(self.module_replaces.iter().map(|(_, replace)| replace));
        for replace in directives {
            if replace.old_path != path {
                continue;
            }
            let Some(version) = replace.old_version.as_deref() else {
                continue;
            };
            if selected.is_some_and(|selected| !replacement_could_be_selected(version, selected)) {
                continue;
            }
            candidates.insert(version);
        }
        candidates.into_iter().map(str::to_string).collect()
    }

    /// The internal target a replacement at a hypothetical version would
    /// select, without validating it: a candidate that may never be selected is
    /// skipped rather than failing planning.
    fn hypothetical_target(
        &self,
        path: &str,
        version: Option<&str>,
    ) -> Result<BTreeSet<String>, Error> {
        let mut targets = BTreeSet::new();
        for (base, replace) in self.directives_at(path, version) {
            if let Some(target) = self.resolve(base, replace, false)? {
                targets.insert(target);
            }
        }
        Ok(targets)
    }

    fn directives_at(
        &self,
        old_path: &str,
        version: Option<&str>,
    ) -> Vec<(&AbsoluteSystemPath, &GoReplaceDirective)> {
        select_replacement_directives(
            self.repo_root,
            self.workspace_replaces,
            self.module_replaces,
            old_path,
            version,
        )
    }

    /// Resolve one replacement's new module to a workspace member, if any.
    ///
    /// `strict` propagates validation errors for a replacement that definitely
    /// applies. Hypothetical candidates are best-effort: an unresolvable
    /// candidate may never be selected, so it is skipped rather than failing
    /// planning.
    fn resolve(
        &self,
        base_dir: &AbsoluteSystemPath,
        replace: &GoReplaceDirective,
        strict: bool,
    ) -> Result<Option<String>, Error> {
        let new_path = &replace.new_path;
        let is_local = new_path.starts_with('.') || Path::new(new_path).is_absolute();
        if !is_local {
            // Only local (directory) replacements can name a workspace member.
            // A replacement to a remote module path resolves externally, which
            // is what `go list -m all` plus `go mod graph` observe as well.
            return Ok(None);
        }

        let resolved = join_relative_path(base_dir, new_path)?;
        if !path_is_within_repository(self.repo_root, &resolved)? {
            if strict {
                return Err(Error::OutsideRepositoryLocalModule {
                    module_path: new_path.clone(),
                    manifest_path: resolved.join_component(GO_MOD).to_string(),
                });
            }
            return Ok(None);
        }
        let manifest = resolved.join_component(GO_MOD);
        if !manifest.exists() {
            if strict {
                return Err(Error::MissingGoMod {
                    path: resolved.to_string(),
                });
            }
            return Ok(None);
        }
        let resolved_directory = resolved.to_realpath()?;
        if let Some(module_path) = self.member_real_directories.get(&resolved_directory) {
            return Ok(Some(module_path.clone()));
        }
        if strict {
            let parsed = parse_go_mod(&read_manifest(&manifest)?, &manifest)?;
            let module_path = required_module_path(&parsed, &manifest)?.to_string();
            return Err(Error::NonMemberLocalModule {
                module_path,
                manifest_path: manifest.to_string(),
            });
        }
        Ok(None)
    }
}

/// Choose the replacement directives that apply to `old_path` at `version`,
/// following Go workspace precedence:
///
/// 1. A `go.work` replacement with an exact version match.
/// 2. A `go.work` wildcard replacement.
/// 3. Otherwise, directives from every main module. Within one module an exact
///    version match overrides that module's wildcard, and directives from all
///    main modules are unioned because their replacements apply workspace-wide.
///
/// `None` models a version that no directive names, so no exact directive can
/// be assumed to match and only wildcards apply at each tier.
fn select_replacement_directives<'a>(
    repo_root: &'a AbsoluteSystemPath,
    workspace_replaces: &'a [GoReplaceDirective],
    module_replaces: &'a [(AbsoluteSystemPathBuf, GoReplaceDirective)],
    old_path: &str,
    version: Option<&str>,
) -> Vec<(&'a AbsoluteSystemPath, &'a GoReplaceDirective)> {
    let workspace_exact: Vec<_> = workspace_replaces
        .iter()
        .filter(|replace| {
            replace.old_path == old_path && matches_version(replace.old_version.as_deref(), version)
        })
        .collect();
    if !workspace_exact.is_empty() {
        return workspace_exact
            .into_iter()
            .map(|replace| (repo_root, replace))
            .collect();
    }
    let workspace_wildcard: Vec<_> = workspace_replaces
        .iter()
        .filter(|replace| replace.old_path == old_path && replace.old_version.is_none())
        .collect();
    if !workspace_wildcard.is_empty() {
        return workspace_wildcard
            .into_iter()
            .map(|replace| (repo_root, replace))
            .collect();
    }

    let mut per_directory: Vec<(
        &'a AbsoluteSystemPath,
        Vec<&'a GoReplaceDirective>,
        Vec<&'a GoReplaceDirective>,
    )> = Vec::new();
    for (directory, replace) in module_replaces {
        if replace.old_path != old_path {
            continue;
        }
        let directory = directory.as_ref();
        let position = per_directory
            .iter()
            .position(|(candidate, _, _)| *candidate == directory);
        let entry = match position {
            Some(position) => &mut per_directory[position],
            None => {
                per_directory.push((directory, Vec::new(), Vec::new()));
                per_directory.last_mut().expect("entry was just pushed")
            }
        };
        if matches_version(replace.old_version.as_deref(), version) {
            entry.1.push(replace);
        } else if replace.old_version.is_none() {
            entry.2.push(replace);
        }
    }

    let mut selected = Vec::new();
    for (directory, exact, wildcard) in per_directory {
        let chosen = if exact.is_empty() { wildcard } else { exact };
        selected.extend(chosen.into_iter().map(|replace| (directory, replace)));
    }
    selected
}

fn matches_version(candidate: Option<&str>, version: Option<&str>) -> bool {
    match (candidate, version) {
        (Some(candidate), Some(version)) => candidate == version,
        _ => false,
    }
}

fn replace_order(left: &GoReplaceDirective, right: &GoReplaceDirective) -> Ordering {
    (
        &left.old_path,
        &left.old_version,
        &left.new_path,
        &left.new_version,
    )
        .cmp(&(
            &right.old_path,
            &right.old_version,
            &right.new_path,
            &right.new_version,
        ))
}

fn required_module_path<'a>(
    parsed: &'a ParsedGoMod,
    manifest_path: &AbsoluteSystemPath,
) -> Result<&'a str, Error> {
    parsed
        .module_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| Error::MissingModulePath {
            path: manifest_path.to_string(),
        })
}

fn read_manifest(path: &AbsoluteSystemPath) -> Result<String, Error> {
    path.read_to_string().map_err(|source| Error::ManifestRead {
        path: path.to_string(),
        source,
    })
}

/// The directives the `go` command accepts in a `go.work` (verified against
/// Go 1.26: `go`, `toolchain`, `use`, `replace`, and `godebug`; directives
/// like `module`, `require`, `exclude`, `retract`, `env`, or typos such as
/// `unsupported` are rejected with "unknown directive"). Keep this list in
/// sync with the native parser when Go grows new workspace directives.
const GO_WORK_DIRECTIVES: [&str; 5] = ["go", "godebug", "replace", "toolchain", "use"];

fn parse_go_work(text: &str, work_path: &AbsoluteSystemPath) -> Result<ParsedGoWork, Error> {
    let mut parsed = ParsedGoWork::default();
    for (keyword, tokens) in
        scan_directives(text).map_err(|reason| Error::MalformedGoWork { reason })?
    {
        if !GO_WORK_DIRECTIVES.contains(&keyword.as_str()) {
            // The go command rejects the same file, but this diagnostic comes
            // from the static parser: it names the file and the offending
            // directive without pretending any command ran.
            return Err(Error::UnknownGoWorkDirective {
                path: work_path.to_string(),
                directive: keyword,
            });
        }
        match keyword.as_str() {
            "use" => {
                if tokens.len() != 1 {
                    return Err(Error::MalformedGoWork {
                        reason: format!("`use` expects one directory, found {}", tokens.len()),
                    });
                }
                parsed
                    .uses
                    .push(tokens.into_iter().next().unwrap_or_default());
            }
            // The `go` and `toolchain` versions ride along for the prune
            // domain; each is a single token and may appear once.
            "go" | "toolchain" => {
                let value = match tokens.len() {
                    1 => tokens.into_iter().next().unwrap_or_default(),
                    count => {
                        return Err(Error::MalformedGoWork {
                            reason: format!("`{keyword}` expects one version, found {count}"),
                        });
                    }
                };
                let field = if keyword == "go" {
                    &mut parsed.go
                } else {
                    &mut parsed.toolchain
                };
                if field.replace(value).is_some() {
                    return Err(Error::MalformedGoWork {
                        reason: format!("duplicate `{keyword}` directive"),
                    });
                }
            }
            // `godebug` tunes runtime defaults; it does not affect membership,
            // edges, or tasks, so its value is accepted and ignored.
            "godebug" => {}
            "replace" => parsed
                .replaces
                .push(parse_replace(&tokens).map_err(|reason| Error::MalformedGoWork { reason })?),
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_go_mod(text: &str, manifest_path: &AbsoluteSystemPath) -> Result<ParsedGoMod, Error> {
    let malformed = |reason: String| Error::MalformedGoMod {
        path: manifest_path.to_string(),
        reason,
    };
    let mut parsed = ParsedGoMod::default();
    for (keyword, tokens) in scan_directives(text).map_err(malformed)? {
        match keyword.as_str() {
            "module" => {
                if tokens.len() != 1 {
                    return Err(malformed(format!(
                        "`module` expects one path, found {}",
                        tokens.len()
                    )));
                }
                if parsed.module_path.is_some() {
                    return Err(malformed(
                        "duplicate `module` directive; each go.mod declares one module".to_string(),
                    ));
                }
                parsed.module_path = tokens.into_iter().next();
            }
            "require" => {
                if tokens.len() != 2 {
                    return Err(malformed(format!(
                        "`require` expects a module path and version, found {} token(s)",
                        tokens.len()
                    )));
                }
                let mut tokens = tokens.into_iter();
                let path = tokens.next().unwrap_or_default();
                let version = tokens.next();
                parsed.requires.push(GoRequire { path, version });
            }
            "replace" => parsed
                .replaces
                .push(parse_replace(&tokens).map_err(malformed)?),
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_replace(tokens: &[String]) -> Result<GoReplaceDirective, String> {
    // Go directives write `=>` as its own token, so a path or module path may
    // contain `=>` punctuation (for example a quoted `./dir=>name`) without
    // being mistaken for the separator.
    let Some(arrow) = tokens.iter().position(|token| token == "=>") else {
        return Err("replacement is missing the `=>` separator".to_string());
    };
    if tokens[arrow + 1..].iter().any(|token| token == "=>") {
        return Err("replacement contains more than one `=>`".to_string());
    }
    let old = &tokens[..arrow];
    let new = &tokens[arrow + 1..];
    if old.is_empty() || old.len() > 2 {
        return Err("replacement must name `old [version] => new [version]`".to_string());
    }
    if new.is_empty() || new.len() > 2 {
        return Err("replacement must name `old [version] => new [version]`".to_string());
    }
    Ok(GoReplaceDirective {
        old_path: old[0].clone(),
        old_version: old.get(1).cloned(),
        new_path: new[0].clone(),
        new_version: new.get(1).cloned(),
    })
}

/// Flatten a `go.work`/`go.mod` file into `(keyword, entry tokens)` pairs,
/// unwrapping single and parenthesized block forms.
fn scan_directives(text: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut directives = Vec::new();
    let mut block: Option<String> = None;

    for line in logical_lines(text)? {
        let tokens = tokenize(&line)?;
        if tokens.is_empty() {
            continue;
        }

        if let Some(keyword) = block.as_ref() {
            if tokens.iter().any(|token| token == "(") {
                return Err(format!("nested `(` in `{}`", line.trim()));
            }
            if let Some(close) = tokens.iter().position(|token| token == ")") {
                if close + 1 != tokens.len() {
                    return Err(format!("unexpected tokens after `)` in `{}`", line.trim()));
                }
                if close > 0 {
                    directives.push((keyword.clone(), tokens[..close].to_vec()));
                }
                block = None;
            } else {
                directives.push((keyword.clone(), tokens));
            }
            continue;
        }

        let Some(open) = tokens.iter().position(|token| token == "(") else {
            if tokens.iter().any(|token| token == ")") {
                return Err(format!("unexpected `)` in `{}`", line.trim()));
            }
            let mut tokens = tokens.into_iter();
            let keyword = tokens.next().unwrap_or_default();
            let entry: Vec<String> = tokens.collect();
            if entry.is_empty() {
                return Err(format!("`{keyword}` is missing its operands"));
            }
            directives.push((keyword, entry));
            continue;
        };

        if open != 1 {
            return Err(format!("unexpected tokens before `(` in `{}`", line.trim()));
        }

        let keyword = tokens[0].clone();
        let inner = &tokens[open + 1..];
        if inner.iter().any(|token| token == "(") {
            return Err(format!("nested `(` in `{}`", line.trim()));
        }

        if let Some(close) = inner.iter().position(|token| token == ")") {
            if close + 1 != inner.len() {
                return Err(format!("unexpected tokens after `)` in `{}`", line.trim()));
            }
            if close > 0 {
                directives.push((keyword, inner[..close].to_vec()));
            }
        } else {
            if !inner.is_empty() {
                directives.push((keyword.clone(), inner.to_vec()));
            }
            block = Some(keyword);
        }
    }

    if let Some(keyword) = block {
        return Err(format!("unterminated `{keyword} (` block"));
    }
    Ok(directives)
}

/// Split a manifest into logical directive lines, keeping quoted and raw
/// strings intact and removing comments that appear outside of them.
///
/// A Go raw string may span newlines, so it is never broken into directive
/// lines, and carriage returns inside one are discarded as Go does. An
/// interpreted string cannot span lines; the newline is kept so tokenizing
/// reports the unterminated literal instead of silently merging directives.
fn logical_lines(text: &str) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_raw = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if in_raw {
            match character {
                '\r' => {}
                '`' => {
                    in_raw = false;
                    current.push(character);
                }
                other => current.push(other),
            }
            continue;
        }
        if in_double {
            match character {
                '\\' => {
                    current.push(character);
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                }
                '"' => {
                    in_double = false;
                    current.push(character);
                }
                other => current.push(other),
            }
            continue;
        }
        match character {
            '\n' => lines.push(std::mem::take(&mut current)),
            '"' => {
                in_double = true;
                current.push(character);
            }
            '`' => {
                in_raw = true;
                current.push(character);
            }
            '/' if chars.peek() == Some(&'/') => {
                while chars.peek().is_some_and(|next| *next != '\n') {
                    chars.next();
                }
            }
            other => current.push(other),
        }
    }
    if in_raw {
        return Err("unterminated raw string".to_string());
    }
    if in_double {
        return Err("unterminated quoted string".to_string());
    }
    lines.push(current);
    Ok(lines)
}

/// Split a directive line into whitespace-delimited tokens, decoding Go
/// interpreted (double-quoted) strings and backtick raw strings.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            break;
        };
        if first == '"' {
            chars.next();
            tokens.push(unquote_interpreted(&mut chars)?);
        } else if first == '`' {
            chars.next();
            let mut token = String::new();
            let mut terminated = false;
            for character in chars.by_ref() {
                if character == '`' {
                    terminated = true;
                    break;
                }
                token.push(character);
            }
            if !terminated {
                return Err("unterminated raw string".to_string());
            }
            tokens.push(token);
        } else {
            let mut token = String::new();
            while let Some(&character) = chars.peek() {
                if character.is_whitespace() {
                    break;
                }
                token.push(character);
                chars.next();
            }
            tokens.push(token);
        }
    }
    Ok(tokens)
}

type CharStream<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// Decode a Go interpreted string literal, starting after the opening quote.
///
/// Go's escape rules are reproduced so valid paths and module paths survive
/// byte-for-byte: `\\x61` must stay `a` rather than becoming `x61`, and unknown
/// escapes or invalid code points must fail instead of corrupting a path.
fn unquote_interpreted(chars: &mut CharStream<'_>) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                return String::from_utf8(bytes)
                    .map_err(|_| "invalid UTF-8 in quoted string".to_string());
            }
            '\\' => decode_escape(chars, &mut bytes)?,
            '\n' | '\r' => return Err("newline in interpreted string literal".to_string()),
            '\0' => return Err("invalid NUL character in string literal".to_string()),
            other => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    Err("unterminated quoted string".to_string())
}

fn decode_escape(chars: &mut CharStream<'_>, bytes: &mut Vec<u8>) -> Result<(), String> {
    let Some(escape) = chars.next() else {
        return Err("unterminated escape in quoted string".to_string());
    };

    match escape {
        'a' => bytes.push(b'\x07'),
        'b' => bytes.push(b'\x08'),
        'f' => bytes.push(b'\x0c'),
        'n' => bytes.push(b'\n'),
        'r' => bytes.push(b'\r'),
        't' => bytes.push(b'\t'),
        'v' => bytes.push(b'\x0b'),
        '\\' => bytes.push(b'\\'),
        '"' => bytes.push(b'"'),
        'x' => bytes.push(read_hex(chars, 2)? as u8),
        'u' => {
            let character = decode_unicode(read_hex(chars, 4)?, "\\u")?;
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        'U' => {
            let character = decode_unicode(read_hex(chars, 8)?, "\\U")?;
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        '0'..='7' => {
            let mut value = escape.to_digit(8).expect("octal digit");
            for _ in 0..2 {
                let Some(digit) = chars.next() else {
                    return Err("incomplete octal escape in quoted string".to_string());
                };
                let Some(digit) = digit.to_digit(8) else {
                    return Err("invalid octal escape in quoted string".to_string());
                };
                value = value * 8 + digit;
            }
            if value > 0xFF {
                return Err("octal escape out of range in quoted string".to_string());
            }
            bytes.push(value as u8);
        }
        other => return Err(format!("unknown escape `\\{other}` in quoted string")),
    }

    Ok(())
}

fn decode_unicode(value: u32, escape: &str) -> Result<char, String> {
    char::from_u32(value).ok_or_else(|| format!("invalid `{escape}` escape in quoted string"))
}

/// Read exactly `count` hexadecimal digits.
fn read_hex(chars: &mut CharStream<'_>, count: usize) -> Result<u32, String> {
    let mut value = 0u32;
    for _ in 0..count {
        let Some(digit) = chars.next() else {
            return Err("incomplete hexadecimal escape in quoted string".to_string());
        };
        let Some(digit) = digit.to_digit(16) else {
            return Err("invalid hexadecimal escape in quoted string".to_string());
        };
        value = value * 16 + digit;
    }
    Ok(value)
}

/// Whether a version-specific replacement could still apply after minimum
/// version selection. Only a provably lower candidate is excluded; anything
/// greater, equal, or unparseable is conservatively kept.
fn replacement_could_be_selected(candidate: &str, requested: &str) -> bool {
    !matches!(
        compare_go_versions(candidate, requested),
        Some(Ordering::Less)
    )
}

fn compare_go_versions(left: &str, right: &str) -> Option<Ordering> {
    let (left_core, left_pre) = go_version_parts(left)?;
    let (right_core, right_pre) = go_version_parts(right)?;
    Some(match compare_version_core(&left_core, &right_core) {
        Ordering::Equal => compare_prerelease(left_pre.as_deref(), right_pre.as_deref()),
        ordering => ordering,
    })
}

fn compare_version_core(left: &[u64], right: &[u64]) -> Ordering {
    for index in 0..left.len().max(right.len()) {
        let ordering = left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn compare_prerelease(left: Option<&[String]>, right: Option<&[String]>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        // A release outranks its prereleases.
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => {
            for (left, right) in left.iter().zip(right.iter()) {
                let left_numeric =
                    !left.is_empty() && left.bytes().all(|byte| byte.is_ascii_digit());
                let right_numeric =
                    !right.is_empty() && right.bytes().all(|byte| byte.is_ascii_digit());
                let ordering = match (left_numeric, right_numeric) {
                    (true, true) => left.len().cmp(&right.len()).then_with(|| left.cmp(right)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => left.cmp(right),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
    }
}

fn go_version_parts(value: &str) -> Option<(Vec<u64>, Option<Vec<String>>)> {
    let value = value.strip_prefix('v')?;
    let value = value.split('+').next().unwrap_or(value);
    let (core, prerelease) = match value.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (value, None),
    };
    let core: Vec<u64> = core
        .split('.')
        .map(|component| component.parse().ok())
        .collect::<Option<_>>()?;
    if core.is_empty() {
        return None;
    }
    let prerelease = prerelease.map(|prerelease| {
        prerelease
            .split('.')
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    Some((core, prerelease))
}

// ---------------------------------------------------------------------------
// Runnable target classification
// ---------------------------------------------------------------------------

/// The `GOOS` values that can constrain a source file by name, mirroring the
/// `go` command's `internal/syslist.KnownOS` table.
const GO_OS_NAME_SUFFIXES: [&str; 18] = [
    "aix",
    "android",
    "darwin",
    "dragonfly",
    "freebsd",
    "hurd",
    "illumos",
    "ios",
    "js",
    "linux",
    "nacl",
    "netbsd",
    "openbsd",
    "plan9",
    "solaris",
    "wasip1",
    "windows",
    "zos",
];

/// The `GOARCH` values that can constrain a source file by name, mirroring
/// `internal/syslist.KnownArch`.
const GO_ARCH_NAME_SUFFIXES: [&str; 24] = [
    "386",
    "amd64",
    "amd64p32",
    "arm",
    "armbe",
    "arm64",
    "arm64be",
    "loong64",
    "mips",
    "mipsle",
    "mips64",
    "mips64le",
    "mips64p32",
    "mips64p32le",
    "ppc",
    "ppc64",
    "ppc64le",
    "riscv",
    "riscv64",
    "s390",
    "s390x",
    "sparc",
    "sparc64",
    "wasm",
];

/// Classify a module's `dev` target from Go sources without invoking `go`,
/// reproducing what [`super::runnable_target`] observes from
/// `go list -find -json ./...`:
///
/// * a directory is a package when at least one of its `.go` files survives the
///   `go` command's name, file-name-suffix, and build-constraint filtering,
///   with files declaring `package documentation` never counting;
/// * the package name comes from the first surviving file in name-sorted order;
///   `_test.go` files contribute too, and an external test clause (`package
///   foo_test`) contributes `foo`;
/// * the module has a target only when exactly one package is `main`.
///
/// Classification that depends on environment state — build tags, `GOOS`,
/// `GOARCH`, or the toolchain release — refuses with
/// [`Error::StaticDiscoveryUnavailable`] instead of guessing a catalogue.
/// Failures that `go list` hits under every environment (conflicting package
/// clauses, unparsable headers) produce `Ok(None)`, because the authoritative
/// helper drops the `dev` default whenever `go list` exits non-zero.
pub(super) fn runnable_target_statically(
    module_dir: &AbsoluteSystemPath,
) -> Result<Option<String>, Error> {
    let manifest = module_dir.join_component(GO_MOD);
    if !manifest.exists() {
        // Without a module manifest the `go` command cannot list `./...` at
        // all; the authoritative helper observes the failed command and
        // omits the `dev` default.
        return Ok(None);
    }

    // A `go.mod` `ignore` directive removes directories from `./...` by
    // pattern. Reproducing the removal needs those pattern semantics, and
    // walking too much would invent package directories the `go` command
    // never sees, so planning refuses.
    let text = read_manifest(&manifest)?;
    let directives = scan_directives(&text).map_err(|reason| Error::MalformedGoMod {
        path: manifest.to_string(),
        reason,
    })?;
    if directives.iter().any(|(keyword, _)| keyword == "ignore") {
        return Err(Error::StaticDiscoveryUnavailable {
            path: manifest.to_string(),
            reason: "its `ignore` directive removes directories from `go list ./...` by pattern; \
                     reproducing the removal exactly requires native go list metadata"
                .to_string(),
        });
    }

    let mut scan = ModuleScan::default();
    scan_module_tree(module_dir.as_std_path(), false, &mut scan);
    // A listing that fails under every environment — conflicting package
    // clauses, an unparsable header, an unreadable directory — makes the
    // authoritative helper drop the `dev` default, whatever the environment
    // does, so that answer is exact and needs no refusal.
    if scan.fails {
        return Ok(None);
    }
    if let Some(error) = scan.refusal {
        return Err(error);
    }

    match scan.mains.as_slice() {
        [] => Ok(None),
        [directory] => {
            // `go list` reports canonical directories, and the authoritative
            // helper strips the module directory it was given, so a module
            // behind a symlink yields no target there. The scanner walks the
            // directory it was given and mirrors the same strip exactly.
            let Some(real) = dunce::canonicalize(directory).ok() else {
                return Ok(None);
            };
            let Ok(relative) = Path::new(&real).strip_prefix(module_dir.as_std_path()) else {
                return Ok(None);
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            Ok(Some(if relative.is_empty() {
                ".".to_string()
            } else {
                format!("./{relative}")
            }))
        }
        // Two `main` packages leave the `dev` target ambiguous; the
        // authoritative helper reports none rather than picking one.
        _ => Ok(None),
    }
}

/// The static scan of one module's package tree.
#[derive(Default)]
struct ModuleScan {
    /// The walked directories of `main` packages.
    mains: Vec<PathBuf>,
    /// `go list` fails under every environment, so the `dev` default is
    /// dropped exactly as the authoritative helper drops it on a non-zero
    /// exit.
    fails: bool,
    /// The first environment-dependent classification, as a refusal.
    refusal: Option<Error>,
}

/// Walk one directory of the module tree the way `go list ./...` does:
/// entry names in sorted order, symlinked directories neither traversed nor
/// read, and the exclusions below applied to subdirectories only.
///
/// `below_vendor` is set for the children of a directory named `vendor`:
/// `go list ./...` ignores paths with a `vendor` element anywhere except the
/// last, so a package *named* `vendor` is matched for historical reasons
/// while nothing beneath it is.
fn scan_module_tree(directory: &Path, below_vendor: bool, scan: &mut ModuleScan) {
    let directory_entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        // A directory the `go` command cannot read fails its whole walk, and
        // `go list` exits non-zero.
        Err(_) => {
            scan.fails = true;
            return;
        }
    };
    let mut entries = Vec::new();
    // Iterating `directory_entries`, not the `entries` vec being built: a
    // loop over the vec itself would infer its element type from what it
    // pushes into itself, which overflows type inference (E0275) and would
    // observe no entries at all.
    for entry in directory_entries {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(_) => {
                scan.fails = true;
                return;
            }
        }
    }
    // `os.ReadDir` returns entries sorted by name, and the package-name
    // sequence below depends on that order.
    entries.sort_by_key(|entry| entry.file_name());

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut subdirectories: Vec<(String, PathBuf)> = Vec::new();
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            scan.fails = true;
            continue;
        };
        if file_type.is_dir() {
            subdirectories.push((name, entry.path()));
            continue;
        }
        if file_type.is_symlink()
            && matches!(fs::metadata(entry.path()), Ok(metadata) if metadata.is_dir())
        {
            // Symlinks to directories are neither traversed nor read, which
            // is why a symlinked sibling cannot add a second `main`.
            continue;
        }
        files.push((name, entry.path()));
    }

    let classification = classify_directory(&files);
    scan.fails |= classification.fails;
    if scan.refusal.is_none() {
        scan.refusal = classification.refusal;
    }
    if classification.name.as_deref() == Some("main".as_bytes()) {
        scan.mains.push(directory.to_path_buf());
    }

    if below_vendor {
        // Nothing below a `vendor` element is ever matched.
        return;
    }
    for (name, path) in subdirectories {
        // The `./...` walk skips `testdata` trees and dotted and underscored
        // names at any depth, and nested modules.
        if name.starts_with('.') || name.starts_with('_') || name == "testdata" {
            continue;
        }
        if matches!(fs::metadata(path.join(GO_MOD)), Ok(metadata) if metadata.is_file()) {
            continue;
        }
        scan_module_tree(&path, name == "vendor", scan);
    }
}

/// The classification of one directory in the module tree.
struct DirectoryClassification {
    /// The package name `go list` reports, or `None` when `./...` does not
    /// match the directory at all.
    name: Option<Vec<u8>>,
    /// `go list` fails on this directory under every environment.
    fails: bool,
    /// Exact classification is environment-dependent, carrying the first
    /// offending file.
    refusal: Option<Error>,
}

/// How one `.go` file in a considered directory participates in the
/// classification `go list -find` produces.
enum GoSourceRole {
    /// Included under every environment; contributes its package clause.
    Contributing(Vec<u8>),
    /// Included only under some environments: a `//go:build` line, a
    /// `// +build` line, or a `GOOS`/`GOARCH` file-name suffix decides.
    Constrained(Vec<u8>),
    /// Declares `package documentation`, which the `go` command ignores
    /// outright under every environment.
    Documentation,
    /// The `go` command fails on the file under every environment, so
    /// `go list ./...` exits non-zero.
    Fails,
    /// Only the environments that include the file fail on it, so exact
    /// classification is environment-dependent.
    EnvironmentSensitive,
}

/// One `.go` file of a directory, with what classification needs from it.
struct ClassifiedFile {
    is_test: bool,
    role: GoSourceRole,
    /// The file's absolute path, for refusal diagnostics.
    path: String,
}

/// Classify one directory exactly as `go/build`'s directory reader does: the
/// package name is settled by the first contributing file in sorted order,
/// and every later contributing file must agree with it.
fn classify_directory(files: &[(String, PathBuf)]) -> DirectoryClassification {
    let mut classified: Vec<ClassifiedFile> = Vec::new();
    for (name, path) in files {
        // `go/build` ignores `_`- and `.`-prefixed names outright, and only
        // `.go` files contribute to package classification.
        if name.starts_with('_') || name.starts_with('.') || !name.ends_with(".go") {
            continue;
        }
        classified.push(ClassifiedFile {
            is_test: name.ends_with("_test.go"),
            role: classify_go_source(name, path),
            path: path.display().to_string(),
        });
    }

    let mut classification = DirectoryClassification {
        name: None,
        fails: false,
        refusal: None,
    };
    let mut first_contributing: Option<usize> = None;
    for (position, file) in classified.iter().enumerate() {
        match &file.role {
            GoSourceRole::Contributing(clause) => {
                let clause = normalized_test_package_clause(
                    clause,
                    file.is_test,
                    classification.name.as_deref(),
                );
                if classification.name.is_none() {
                    classification.name = Some(clause);
                    first_contributing = Some(position);
                } else if classification.name != Some(clause) {
                    // `go/build` reports `found packages a and b`, `go list`
                    // exits non-zero, and the `dev` default is dropped.
                    classification.fails = true;
                }
            }
            GoSourceRole::Documentation => {}
            GoSourceRole::Fails => classification.fails = true,
            GoSourceRole::EnvironmentSensitive => {
                if classification.refusal.is_none() {
                    classification.refusal = Some(Error::StaticDiscoveryUnavailable {
                        path: file.path.clone(),
                        reason: "a GOOS or GOARCH file-name suffix decides whether the `go` \
                                 command ever reads it, and it only parses under the environments \
                                 that include it, so the module's package tree cannot be \
                                 classified without native go list metadata"
                            .to_string(),
                    });
                }
            }
            // Checked against the settled name in the second pass below.
            GoSourceRole::Constrained(_) => {}
        }
    }

    for (position, file) in classified.iter().enumerate() {
        let GoSourceRole::Constrained(clause) = &file.role else {
            continue;
        };
        // The name this file would contribute under the environments that
        // include it. The strip rule consults the name settled by the files
        // the `go` command reads before it, which never changes once set.
        let settled = if first_contributing.is_some_and(|first| first < position) {
            classification.name.as_deref()
        } else {
            None
        };
        let contribution = normalized_test_package_clause(clause, file.is_test, settled);
        match classification.name.as_deref() {
            Some(current) if current == contribution.as_slice() => {}
            Some(current) => {
                if classification.refusal.is_none() {
                    classification.refusal = Some(Error::StaticDiscoveryUnavailable {
                        path: file.path.clone(),
                        reason: format!(
                            "build constraints decide whether it contributes its `package {}` \
                             clause, which disagrees with the `package {}` clause its directory \
                             otherwise settles on; the tags, target platform, and toolchain that \
                             evaluate the constraints are environment state that static planning \
                             cannot sample",
                            String::from_utf8_lossy(&contribution),
                            String::from_utf8_lossy(current),
                        ),
                    });
                }
            }
            // No file settles the directory's package under every
            // environment, so whether the directory is a package at all
            // depends on the constraints on this file.
            None => {
                if classification.refusal.is_none() {
                    classification.refusal = Some(Error::StaticDiscoveryUnavailable {
                        path: file.path.clone(),
                        reason: "build constraints decide whether its directory contains a Go \
                                 package at all; the tags, target platform, and toolchain that \
                                 evaluate them are environment state that static planning cannot \
                                 sample"
                            .to_string(),
                    });
                }
            }
        }
    }

    classification
}

/// The package clause a `_test.go` file contributes: an external test clause
/// (`package foo_test`) contributes `foo`, unless the directory already
/// settled on the full clause — mirroring `go/build`'s XTest handling, which
/// also strips when no name is settled yet.
fn normalized_test_package_clause(clause: &[u8], is_test: bool, current: Option<&[u8]>) -> Vec<u8> {
    if is_test && clause.ends_with(b"_test") && current != Some(clause) {
        clause[..clause.len() - b"_test".len()].to_vec()
    } else {
        clause.to_vec()
    }
}

/// Classify one source file by reading it exactly as far as `go list -find`
/// does: the package clause and imports through `go/build`'s header lexer,
/// and the leading comments through its `shouldBuild` header scan.
fn classify_go_source(name: &str, path: &Path) -> GoSourceRole {
    let filename_constrained = filename_implies_go_os_or_arch(name);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // A `GOOS`/`GOARCH`-suffixed name is rejected before the `go`
        // command ever opens the file, so the failure appears only under
        // the environments that include it; any other read failure happens
        // under every environment.
        Err(_) if filename_constrained => return GoSourceRole::EnvironmentSensitive,
        Err(_) => return GoSourceRole::Fails,
    };

    let role_for_broken_header = || {
        if filename_constrained {
            GoSourceRole::EnvironmentSensitive
        } else {
            GoSourceRole::Fails
        }
    };
    // `go/build` reads the package clause and checks the `//go:build` lines
    // before it evaluates constraints, so a file whose name is not
    // constrained fails the listing under every environment; a name-
    // constrained file fails only where it is included at all.
    let clause = match lex_go_package_clause(&bytes) {
        Ok(clause) => clause,
        Err(()) => return role_for_broken_header(),
    };
    let constraints = scan_go_file_header(&bytes);
    if constraints.go_build.len() > 1
        || constraints
            .go_build
            .iter()
            .any(|expression| !go_build_expression_parses(expression))
    {
        return role_for_broken_header();
    }

    if clause.as_slice() == b"documentation" {
        // `go list` ignores documentation-only package clauses outright.
        return GoSourceRole::Documentation;
    }
    if filename_constrained || !constraints.go_build.is_empty() || constraints.plus_build {
        return GoSourceRole::Constrained(clause);
    }
    GoSourceRole::Contributing(clause)
}

/// Whether a file name carries the implicit `GOOS`/`GOARCH` constraint the
/// `go` command applies (`foo_linux.go`, `foo_amd64.go`,
/// `foo_darwin_arm64.go`), mirroring `goodOSArchFile`: the components after
/// the first `_` of the name up to its first `.` decide, a trailing `test`
/// component is dropped first, and `linux.go` on its own never constrains.
fn filename_implies_go_os_or_arch(name: &str) -> bool {
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    let Some(underscore) = stem.find('_') else {
        return false;
    };
    let mut components: Vec<&str> = stem[underscore..].split('_').collect();
    if components.last() == Some(&"test") {
        components.pop();
    }
    let Some(&last) = components.last() else {
        return false;
    };
    let os = |component: &str| GO_OS_NAME_SUFFIXES.contains(&component);
    let arch = |component: &str| GO_ARCH_NAME_SUFFIXES.contains(&component);
    // An `OS_ARCH` pair must match both; otherwise the final component
    // alone decides, so `foo_amd64_darwin.go` constrains on `darwin`.
    if components.len() >= 2 && os(components[components.len() - 2]) && arch(last) {
        return true;
    }
    os(last) || arch(last)
}

/// The build-constraint facts the `go` command derives from a file's leading
/// comments, mirroring `go/build`'s `parseFileHeader` and the `+build`
/// fallback in `shouldBuild`.
#[derive(Debug, Default)]
struct GoHeaderConstraints {
    /// The expressions of the `//go:build` lines found before the package
    /// clause; the `go` command fails a file that carries more than one.
    go_build: Vec<String>,
    /// Whether a `// +build` line appears in the region the `go` command
    /// honors when there is no `//go:build` line: before the blank line that
    /// precedes the package clause.
    plus_build: bool,
}

/// Scan a file's leading comments. A `//go:build` line counts wherever it
/// appears before the first non-comment text — after doc comments, blank
/// lines, and closed block comments — while `+build` lines count only in the
/// region that ends at the last blank line before the package clause. A
/// leading byte order mark hides a `//go:build` line from the `go` command,
/// so it hides it here too.
fn scan_go_file_header(bytes: &[u8]) -> GoHeaderConstraints {
    let mut go_build = Vec::new();
    let mut plus_build = false;
    let mut ended = false;
    let mut in_block_comment = false;
    // The offset just past the most recent blank line before the first
    // non-comment text: the end of the region `+build` lines are honored in.
    let mut blank_line_end = 0;

    let mut offset = 0;
    while offset < bytes.len() {
        let line_end = bytes[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(bytes.len(), |position| offset + position);
        let line = String::from_utf8_lossy(&bytes[offset..line_end]);
        let after_line = if line_end < bytes.len() {
            line_end + 1
        } else {
            bytes.len()
        };
        let trimmed = line.trim();

        if trimmed.is_empty() && !ended {
            blank_line_end = after_line;
            offset = after_line;
            continue;
        }
        if !trimmed.starts_with("//") {
            ended = true;
        }
        if !in_block_comment && let Some(expression) = go_build_expression(trimmed) {
            go_build.push(expression.to_string());
        }

        // Scan the line's comment structure; the first non-comment text
        // ends the header entirely, exactly as `parseFileHeader` stops.
        let mut rest = trimmed;
        let mut stops_at_source = false;
        loop {
            if in_block_comment {
                if let Some(close) = rest.find("*/") {
                    in_block_comment = false;
                    rest = rest[close + 2..].trim();
                    continue;
                }
                break;
            }
            // An exhausted line is done without being source text, exactly
            // as `parseFileHeader`'s `for len(line) > 0` exits its loop
            // normally: a block comment that closes at the end of a line
            // must not end the header scan before a later `//go:build`
            // line.
            if rest.is_empty() || rest.starts_with("//") {
                break;
            }
            if rest.starts_with("/*") {
                in_block_comment = true;
                rest = rest[2..].trim();
                continue;
            }
            stops_at_source = true;
            break;
        }
        offset = after_line;
        if stops_at_source {
            break;
        }
    }

    if go_build.is_empty() {
        let region = String::from_utf8_lossy(&bytes[..blank_line_end]);
        for line in region.lines() {
            let trimmed = line.trim();
            if is_plus_build_line(trimmed) {
                plus_build = true;
                break;
            }
        }
    }

    GoHeaderConstraints {
        go_build,
        plus_build,
    }
}

/// The expression carried by a `//go:build` line, when the line is one:
/// the prefix must be followed by whitespace, so `//go:buildignore` is a
/// plain comment.
fn go_build_expression(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("//go:build")?;
    if rest.is_empty() {
        return Some("");
    }
    let trimmed = rest.trim();
    if trimmed.len() == rest.len() {
        return None;
    }
    Some(trimmed)
}

/// Whether a line is a `// +build` line, mirroring `splitPlusBuild`: the
/// `+build` prefix must be followed by whitespace or nothing, and the space
/// after `//` is optional.
fn is_plus_build_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("//") else {
        return false;
    };
    let rest = rest.trim();
    let Some(after) = rest.strip_prefix("+build") else {
        return false;
    };
    after.is_empty() || after.trim() != after
}

/// Whether a `//go:build` expression parses, mirroring
/// `go/build/constraint`'s grammar: `||` chains of `&&` chains of `!`
/// atoms, an atom being a tag or a parenthesized expression, with `!!`
/// banned and every token consumed. Malformed expressions fail `go list`
/// under every environment, so the distinction matters.
fn go_build_expression_parses(expression: &str) -> bool {
    let mut parser = GoBuildExpressionParser {
        text: expression,
        position: 0,
        token: None,
        failed: false,
    };
    parser.parse_or() && parser.token.is_none()
}

/// One token of a `//go:build` expression. Only the token kind matters:
/// validation never needs a tag's text.
#[derive(Debug, PartialEq, Eq)]
enum GoBuildToken {
    Or,
    And,
    Not,
    Open,
    Close,
    Tag,
}

/// A mirror of `go/build/constraint`'s expression parser, sharing its
/// one-token lookahead: an operand's atom loads the token that follows it,
/// and the operator loops then consume that token by advancing past it.
struct GoBuildExpressionParser<'a> {
    text: &'a str,
    position: usize,
    token: Option<GoBuildToken>,
    failed: bool,
}

impl GoBuildExpressionParser<'_> {
    /// Load the next token, mirroring `exprParser.lex`.
    fn lex(&mut self) {
        self.token = None;
        let bytes = self.text.as_bytes();
        while self.position < bytes.len()
            && (bytes[self.position] == b' ' || bytes[self.position] == b'\t')
        {
            self.position += 1;
        }
        let Some(&byte) = bytes.get(self.position) else {
            return;
        };
        match byte {
            b'(' => {
                self.position += 1;
                self.token = Some(GoBuildToken::Open);
            }
            b')' => {
                self.position += 1;
                self.token = Some(GoBuildToken::Close);
            }
            b'!' => {
                self.position += 1;
                self.token = Some(GoBuildToken::Not);
            }
            b'&' if bytes.get(self.position + 1) == Some(&b'&') => {
                self.position += 2;
                self.token = Some(GoBuildToken::And);
            }
            b'|' if bytes.get(self.position + 1) == Some(&b'|') => {
                self.position += 2;
                self.token = Some(GoBuildToken::Or);
            }
            _ => {
                let mut end = self.text.len();
                for (index, character) in self.text[self.position..].char_indices() {
                    if !is_go_build_tag_char(character) {
                        end = self.position + index;
                        break;
                    }
                }
                if end == self.position {
                    self.failed = true;
                    return;
                }
                self.token = Some(GoBuildToken::Tag);
                self.position = end;
            }
        }
    }

    fn parse_or(&mut self) -> bool {
        if !self.parse_and() {
            return false;
        }
        while self.token == Some(GoBuildToken::Or) {
            if !self.parse_and() {
                return false;
            }
        }
        true
    }

    fn parse_and(&mut self) -> bool {
        if !self.parse_not() {
            return false;
        }
        while self.token == Some(GoBuildToken::And) {
            if !self.parse_not() {
                return false;
            }
        }
        true
    }

    fn parse_not(&mut self) -> bool {
        self.lex();
        if self.failed {
            return false;
        }
        if self.token == Some(GoBuildToken::Not) {
            self.lex();
            if self.failed {
                return false;
            }
            if self.token == Some(GoBuildToken::Not) {
                // `!!` is banned outright.
                return false;
            }
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> bool {
        match self.token.take() {
            Some(GoBuildToken::Open) => {
                if !self.parse_or() {
                    return false;
                }
                if self.token != Some(GoBuildToken::Close) {
                    return false;
                }
                self.lex();
                !self.failed
            }
            Some(GoBuildToken::Tag) => {
                self.lex();
                !self.failed
            }
            // An operator or the end of the expression is not an atom.
            _ => false,
        }
    }
}

/// Whether a character can appear in a build tag, mirroring
/// `constraint.isValidTag`'s rule: Unicode letters and digits, `_`, and `.`.
fn is_go_build_tag_char(character: char) -> bool {
    character.is_alphabetic() || character.is_numeric() || character == '_' || character == '.'
}

/// Read a file's package clause, and the import section after it whose
/// syntax `go list` also checks, exactly as `go/build`'s `importReader`
/// does. A failure here fails the whole `go list` invocation, so it maps to
/// the deterministic-failure role rather than to a missing clause.
fn lex_go_package_clause(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    let mut lexer = GoHeaderLexer::new(bytes);
    lexer.read_keyword(b"package")?;
    let clause = lexer.read_ident()?;
    while lexer.peek_byte(true) == Some(b'i') {
        lexer.read_keyword(b"import")?;
        if lexer.peek_byte(true) == Some(b'(') {
            lexer.next_byte(false);
            while lexer.peek_byte(true) != Some(b')') && !lexer.failed {
                lexer.read_import()?;
            }
            lexer.next_byte(false);
        } else {
            lexer.read_import()?;
        }
    }
    if lexer.failed {
        return Err(());
    }
    Ok(clause)
}

/// A byte-level mirror of `go/build`'s `importReader`: whitespace and `;`
/// separate tokens, both comment forms are skipped, and a single peeked byte
/// carries the one-token lookahead.
struct GoHeaderLexer<'a> {
    bytes: &'a [u8],
    position: usize,
    peeked: Option<u8>,
    failed: bool,
}

impl<'a> GoHeaderLexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        // A leading UTF-8 byte order mark is ignored, exactly as the `go`
        // command ignores one at the start of a source file — while the
        // comment scan above still sees it, hiding any `//go:build` line.
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        Self {
            bytes,
            position: 0,
            peeked: None,
            failed: false,
        }
    }

    /// Read the next byte. A NUL byte fails the read, mirroring `errNUL`.
    fn read_byte(&mut self) -> Option<u8> {
        if self.failed {
            return None;
        }
        let byte = self.bytes.get(self.position).copied();
        if byte.is_some() {
            self.position += 1;
        }
        if byte == Some(0) {
            self.failed = true;
        }
        byte
    }

    /// Peek the next byte, optionally skipping whitespace, `;`, and
    /// comments, mirroring `peekByte`. Comments are consumed by the peek
    /// itself, and an unterminated block comment or a lone `/` fails.
    fn peek_byte(&mut self, skip_space: bool) -> Option<u8> {
        if self.failed {
            return None;
        }
        let mut current = match self.peeked.take() {
            Some(byte) => Some(byte),
            None => self.read_byte(),
        };
        if skip_space {
            while let Some(byte) = current {
                match byte {
                    b' ' | b'\x0c' | b'\t' | b'\r' | b'\n' | b';' => {
                        current = self.read_byte();
                    }
                    b'/' => match self.read_byte() {
                        // A line comment runs through its newline, which is
                        // consumed, and lexing resumes on the next byte.
                        Some(b'/') => {
                            while let Some(comment) = self.read_byte() {
                                if comment == b'\n' {
                                    break;
                                }
                            }
                            current = self.read_byte();
                        }
                        // A block comment closes on the first `*/` that
                        // cannot start at the opener's `*`.
                        Some(b'*') => {
                            self.skip_block_comment();
                            current = self.read_byte();
                        }
                        // A lone `/` is not valid header syntax.
                        _ => {
                            self.failed = true;
                            return None;
                        }
                    },
                    _ => break,
                }
            }
        }
        if self.failed {
            return None;
        }
        self.peeked = current;
        current
    }

    /// Consume through the closing `*/` of a block comment, failing on an
    /// unterminated comment or a NUL byte inside one.
    fn skip_block_comment(&mut self) {
        let mut index = self.position;
        loop {
            if index + 1 >= self.bytes.len() {
                self.failed = true;
                return;
            }
            let (left, right) = (self.bytes[index], self.bytes[index + 1]);
            if left == 0 || right == 0 {
                self.failed = true;
                return;
            }
            if left == b'*' && right == b'/' {
                self.position = index + 2;
                return;
            }
            index += 1;
        }
    }

    /// Consume and return the peeked byte, mirroring `nextByte`.
    fn next_byte(&mut self, skip_space: bool) -> Option<u8> {
        if skip_space {
            self.peek_byte(true);
        }
        let byte = self.peek_byte(false);
        self.peeked = None;
        byte
    }

    /// Read a keyword: the exact bytes, then a boundary that must not
    /// continue an identifier, so `packagex` fails while `package main`
    /// does not.
    fn read_keyword(&mut self, keyword: &[u8]) -> Result<(), ()> {
        self.peek_byte(true);
        for expected in keyword {
            if self.next_byte(false) != Some(*expected) {
                self.failed = true;
                return Err(());
            }
        }
        if self.peek_byte(false).is_some_and(is_go_ident_byte) {
            self.failed = true;
            return Err(());
        }
        Ok(())
    }

    /// Read an identifier and return its bytes. A leading digit fails,
    /// because `go/parser` rejects such identifiers and fails the listing.
    fn read_ident(&mut self) -> Result<Vec<u8>, ()> {
        if !self.peek_byte(true).is_some_and(is_go_ident_start) {
            self.failed = true;
            return Err(());
        }
        let mut ident = Vec::new();
        while let Some(byte) = self.peek_byte(false) {
            if !is_go_ident_byte(byte) {
                break;
            }
            ident.push(byte);
            self.peeked = None;
        }
        if self.failed {
            return Err(());
        }
        Ok(ident)
    }

    /// Read one import entry: an optional `.` or package name, then the
    /// quoted path.
    fn read_import(&mut self) -> Result<(), ()> {
        match self.peek_byte(true) {
            Some(b'.') => {
                self.peeked = None;
            }
            Some(byte) if is_go_ident_start(byte) => {
                self.read_ident()?;
            }
            _ => {}
        }
        self.read_string()
    }

    /// Read a raw or interpreted string literal, mirroring `readString`:
    /// an interpreted literal cannot span lines, and both fail at the end
    /// of the file.
    fn read_string(&mut self) -> Result<(), ()> {
        match self.next_byte(true) {
            Some(b'`') => loop {
                match self.next_byte(false) {
                    Some(b'`') => return Ok(()),
                    Some(_) => continue,
                    None => {
                        self.failed = true;
                        return Err(());
                    }
                }
            },
            Some(b'"') => loop {
                match self.next_byte(false) {
                    Some(b'"') => return Ok(()),
                    Some(b'\\') => {
                        self.next_byte(false);
                    }
                    Some(b'\n') | None => {
                        self.failed = true;
                        return Err(());
                    }
                    Some(_) => continue,
                }
            },
            _ => {
                self.failed = true;
                Err(())
            }
        }
    }
}

/// Whether a byte can start a Go identifier: the `go` command's header
/// reader accepts any identifier-shaped byte run, but a clause or import
/// name beginning with a digit fails `go/parser` and the whole listing.
fn is_go_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// Whether a byte can continue a Go identifier.
fn is_go_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_utf8_bytes_and_all_inline_block_operands() {
        assert_eq!(
            tokenize(r#""\xc3\xa9 \303\251""#).unwrap(),
            vec!["é é".to_string()]
        );
        assert_eq!(
            compare_go_versions(
                "v1.0.0-999999999999999999999",
                "v1.0.0-1000000000000000000000",
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_go_versions("v1.0.0-999999999999999999999", "v1.0.0-alpha"),
            Some(Ordering::Less)
        );
        assert_eq!(
            scan_directives("use ( ./one\n./two\n)\n").unwrap(),
            vec![
                ("use".to_string(), vec!["./one".to_string()]),
                ("use".to_string(), vec!["./two".to_string()]),
            ]
        );
        assert!(scan_directives("use ( ./one ) ./two\n").is_err());
    }

    #[test]
    fn parses_single_and_block_directives_with_comments() {
        let work = r#"
go 1.22

toolchain go1.22.0

use ./api
use (
	./lib // trailing comment
	// a whole-line comment
	"./quoted dir"
	`./raw`
)

replace example.com/a => ./api
replace (
	example.com/b v1.0.0 => ./lib
	example.com/c => ./api
)
"#;
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(parsed.uses, vec!["./api", "./lib", "./quoted dir", "./raw"]);
        assert_eq!(parsed.replaces.len(), 3);
        assert_eq!(parsed.replaces[0].old_path, "example.com/a");
        assert_eq!(parsed.replaces[0].old_version, None);
        assert_eq!(parsed.replaces[0].new_path, "./api");
        assert_eq!(parsed.replaces[1].old_path, "example.com/b");
        assert_eq!(parsed.replaces[1].old_version.as_deref(), Some("v1.0.0"));
        assert_eq!(parsed.replaces[1].new_path, "./lib");
        assert_eq!(parsed.replaces[1].new_version, None);
    }

    #[test]
    fn parses_module_require_and_replace_forms() {
        let manifest = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\go.mod"
        } else {
            "/repo/go.mod"
        })
        .unwrap();
        let go_mod = r#"module example.com/api

go 1.22

require example.com/one v1.0.0
require (
	example.com/two v2.0.0 // indirect
	example.com/three v0.1.0
)

exclude example.com/bad v1.0.0

replace example.com/one => ../one
replace (
	example.com/two v2.0.0 => example.com/fork v2.1.0
	example.com/three => `../three dir`
)
"#;
        let parsed = parse_go_mod(go_mod, &manifest).unwrap();
        assert_eq!(parsed.module_path.as_deref(), Some("example.com/api"));
        assert_eq!(parsed.requires.len(), 3);
        assert_eq!(parsed.requires[1].path, "example.com/two");
        assert_eq!(parsed.requires[1].version.as_deref(), Some("v2.0.0"));
        assert_eq!(parsed.replaces.len(), 3);
        assert_eq!(parsed.replaces[1].old_version.as_deref(), Some("v2.0.0"));
        assert_eq!(parsed.replaces[1].new_path, "example.com/fork");
        assert_eq!(parsed.replaces[1].new_version.as_deref(), Some("v2.1.0"));
        assert_eq!(parsed.replaces[2].new_path, "../three dir");
    }

    #[test]
    fn rejects_malformed_replacements() {
        assert!(parse_replace(&["example.com/a".to_string(), "v1.0.0".to_string()]).is_err());
        assert!(
            parse_replace(&[
                "example.com/a".to_string(),
                "=>".to_string(),
                "b".to_string(),
                "=>".to_string(),
                "c".to_string(),
            ])
            .is_err()
        );

        let manifest = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\go.mod"
        } else {
            "/repo/go.mod"
        })
        .unwrap();
        assert!(matches!(
            parse_go_mod("module example.com/api\nreplace example.com/a\n", &manifest),
            Err(Error::MalformedGoMod { .. })
        ));
    }

    #[test]
    fn compares_go_versions_conservatively() {
        assert_eq!(
            compare_go_versions("v1.2.0", "v1.2.0"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_go_versions("v1.10.0", "v1.9.0"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_go_versions("v1.0.0", "v1.0.1"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_go_versions("v1.0.0-rc.1", "v1.0.0"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_go_versions("v2.0.0+incompatible", "v2.0.0"),
            Some(Ordering::Equal)
        );
        // Unparseable versions are never provably lower.
        assert_eq!(compare_go_versions("latest", "v1.0.0"), None);
        assert!(replacement_could_be_selected("v1.1.0", "v1.0.0"));
        assert!(replacement_could_be_selected("latest", "v1.0.0"));
        assert!(!replacement_could_be_selected("v1.0.0", "v1.1.0"));
    }

    fn absolute(path: &str) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(path).unwrap()
    }

    fn replacement(
        old_path: &str,
        old_version: Option<&str>,
        new_path: &str,
    ) -> GoReplaceDirective {
        GoReplaceDirective {
            old_path: old_path.to_string(),
            old_version: old_version.map(str::to_string),
            new_path: new_path.to_string(),
            new_version: None,
        }
    }

    fn selected_new_paths(selected: &[(&AbsoluteSystemPath, &GoReplaceDirective)]) -> Vec<String> {
        selected
            .iter()
            .map(|(_, replace)| replace.new_path.clone())
            .collect()
    }

    /// An absolute path below a repository root that stays absolute on every
    /// platform.
    fn repo_file(relative: &str) -> AbsoluteSystemPathBuf {
        if cfg!(windows) {
            absolute(&format!(r"C:\repo\{}", relative.replace('/', "\\")))
        } else {
            absolute(&format!("/repo/{relative}"))
        }
    }

    #[test]
    fn deferred_paths_contribute_no_edge_and_are_owned_by_their_requirer() {
        let api = Member {
            module_path: "example.com/api".to_string(),
            manifest_path: repo_file("apps/api/go.mod"),
            directory: repo_file("apps/api"),
            requires: vec![
                GoRequire {
                    path: "example.com/exact".to_string(),
                    version: Some("v1.0.0".to_string()),
                },
                GoRequire {
                    path: "example.com/alias".to_string(),
                    version: Some("v1.0.0".to_string()),
                },
            ],
            replaces: Vec::new(),
        };
        let library = Member {
            module_path: "example.com/lib".to_string(),
            manifest_path: repo_file("packages/lib/go.mod"),
            directory: repo_file("packages/lib"),
            requires: Vec::new(),
            replaces: Vec::new(),
        };

        let mut resolutions: HashMap<&str, PathResolution> = HashMap::new();
        resolutions.insert(
            "example.com/exact",
            PathResolution::Targets(["example.com/lib".to_string()].into()),
        );
        resolutions.insert(
            "example.com/alias",
            PathResolution::Deferred(PathUncertainty {
                code: "ambiguous-remote-replacement",
                message: "its requirement on example.com/alias cannot be proven exactly"
                    .to_string(),
                // Unnarrowed: the bound resolves to the member identities.
                possible_targets: None,
            }),
        );

        // The requirer keeps every edge it can prove and contributes no edge
        // for the deferred path — never the selected-version target as a
        // guess — and one record per deferred path names it as the owner.
        let (targets, deferred) = classify_relationships(&api, &resolutions);
        assert_eq!(targets, ["example.com/lib".to_string()].into());
        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].code, "ambiguous-remote-replacement");
        assert!(deferred[0].message.contains("example.com/alias"));

        // The scoped record carries the requirer as its scope, and an
        // unnarrowed bound resolves to every member identity except the
        // owner's own.
        let identities: HashSet<String> =
            ["example.com/api", "example.com/lib", "example.com/extra"]
                .into_iter()
                .map(str::to_string)
                .collect();
        let record = deferred[0].scoped_record(&api.module_path, &identities);
        assert_eq!(record.scope(), "example.com/api");
        assert_eq!(record.code(), "ambiguous-remote-replacement");
        assert_eq!(
            record.possible_targets(),
            Some(
                [
                    "example.com/extra".to_string(),
                    "example.com/lib".to_string()
                ]
                .as_slice()
            ),
            "the bound is every member identity except the owner"
        );

        // A member that never requires the ambiguous path keeps exact — and
        // exactly empty — edges and records nothing: empty is a statement,
        // not a placeholder for an unresolved question.
        let (targets, deferred) = classify_relationships(&library, &resolutions);
        assert!(targets.is_empty());
        assert!(deferred.is_empty());
    }

    #[test]
    fn definite_replacement_precedence_is_evaluated_per_version() {
        let root = absolute(if cfg!(windows) { r"C:\repo" } else { "/repo" });
        let member = absolute(if cfg!(windows) {
            r"C:\repo\member"
        } else {
            "/repo/member"
        });
        let workspace = vec![
            replacement("example.com/alias", Some("v1.0.0"), "./ws-exact"),
            replacement("example.com/alias", None, "./ws-wildcard"),
            replacement("example.com/other", Some("v2.0.0"), "./ws-other"),
        ];
        let modules = vec![
            (
                member.clone(),
                replacement("example.com/alias", Some("v1.0.0"), "./mod-exact"),
            ),
            (
                member.clone(),
                replacement("example.com/alias", None, "./mod-wildcard"),
            ),
        ];

        // A workspace exact match wins over both the workspace wildcard and
        // every member directive for that same version.
        assert_eq!(
            selected_new_paths(&select_replacement_directives(
                &root,
                &workspace,
                &modules,
                "example.com/alias",
                Some("v1.0.0"),
            )),
            ["./ws-exact"]
        );
        // A workspace wildcard wins over member directives.
        let selected = select_replacement_directives(
            &root,
            &workspace,
            &modules,
            "example.com/alias",
            Some("v1.1.0"),
        );
        assert_eq!(selected_new_paths(&selected), ["./ws-wildcard"]);
        // An unknown version names no exact directive, so only wildcards apply.
        let selected =
            select_replacement_directives(&root, &workspace, &modules, "example.com/alias", None);
        assert_eq!(selected_new_paths(&selected), ["./ws-wildcard"]);

        // Without a workspace directive, a member's exact version shadows that
        // member's own wildcard, while other members still contribute.
        let other = absolute(if cfg!(windows) {
            r"C:\repo\other"
        } else {
            "/repo/other"
        });
        let modules = vec![
            (
                member.clone(),
                replacement("example.com/alias", Some("v1.0.0"), "./mod-exact"),
            ),
            (
                member,
                replacement("example.com/alias", None, "./mod-wildcard"),
            ),
            (
                other,
                replacement("example.com/alias", None, "./other-wildcard"),
            ),
        ];
        let unused = absolute(if cfg!(windows) {
            r"C:\unused"
        } else {
            "/unused"
        });
        let selected = select_replacement_directives(
            &unused,
            &[],
            &modules,
            "example.com/alias",
            Some("v1.0.0"),
        );
        assert_eq!(
            selected_new_paths(&selected),
            ["./mod-exact", "./other-wildcard"]
        );
    }

    #[test]
    fn preserves_paths_containing_arrow_and_decodes_go_escapes() {
        let work = "go 1.22\n\nreplace example.com/a => \"./dir=>name\"\nreplace example.com/b => \
                    `./raw=>dir`\nuse \"./\\x61\\u0062\\U00000063\"\nuse \"./oct\\141\"\n";
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(parsed.replaces[0].new_path, "./dir=>name");
        assert_eq!(parsed.replaces[1].new_path, "./raw=>dir");
        assert_eq!(parsed.uses, vec!["./abc", "./octa"]);
    }

    #[test]
    fn keeps_multiline_raw_strings_and_ignores_comments_outside_strings() {
        let work = "go 1.22\n\nreplace example.com/a => `./multi\r\nline` // trailing \
                    comment\nuse \"./kept//not-comment\"\n// whole line comment\nuse ./after\n";
        let parsed = parse_go_work(work, &work_file()).unwrap();
        assert_eq!(parsed.replaces[0].new_path, "./multi\nline");
        assert_eq!(parsed.uses, vec!["./kept//not-comment", "./after"]);
    }

    /// The repository-root `go.work` under test.
    fn work_file() -> AbsoluteSystemPathBuf {
        absolute(if cfg!(windows) {
            r"C:\repo\go.work"
        } else {
            "/repo/go.work"
        })
    }

    #[test]
    fn mirrors_the_go_commands_go_work_directive_allowlist() {
        // Verified against Go 1.26: `godebug` is accepted in go.work and
        // ignored for topology, while go.mod-only directives and typos are
        // rejected with the directive and file named — never a fake command
        // failure.
        let parsed = parse_go_work(
            "go 1.22\n\nuse ./api\n\ngodebug default=go1.22#1\n",
            &work_file(),
        )
        .unwrap();
        assert_eq!(parsed.uses, vec!["./api".to_string()]);

        for directive in [
            "unsupported",
            "module",
            "require",
            "exclude",
            "retract",
            "env",
        ] {
            let work = format!("go 1.22\n\n{directive} ./apps/api\n");
            match parse_go_work(&work, &work_file()) {
                Err(Error::UnknownGoWorkDirective {
                    path,
                    directive: found,
                }) => {
                    assert!(
                        path.ends_with("go.work"),
                        "diagnostic names the file: {path}"
                    );
                    assert_eq!(found, directive);
                }
                other => panic!("accepted `{directive}`: {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_invalid_escapes_and_malformed_directives() {
        let manifest = absolute(if cfg!(windows) {
            r"C:\repo\go.mod"
        } else {
            "/repo/go.mod"
        });
        // Unknown escape, incomplete hex, out-of-range octal, and a surrogate
        // code point must all fail instead of corrupting a path.
        for work in [
            "go 1.22\n\nuse \"./a\\q\"\n",
            "go 1.22\n\nuse \"./a\\x6\"\n",
            "go 1.22\n\nuse \"./a\\400\"\n",
            "go 1.22\n\nuse \"./a\\ud800\"\n",
        ] {
            assert!(
                parse_go_work(work, &work_file()).is_err(),
                "accepted {work:?}"
            );
        }
        // An unterminated block and an operand-less directive must fail instead
        // of silently dropping the directive.
        assert!(parse_go_work("go 1.22\n\nuse (\n./a\n", &work_file()).is_err());
        assert!(parse_go_work("go 1.22\n\nuse\n", &work_file()).is_err());
        // `require` must carry exactly a path and version.
        assert!(
            parse_go_mod("module example.com/api\nrequire example.com/a\n", &manifest).is_err()
        );
        assert!(
            parse_go_mod(
                "module example.com/api\nrequire example.com/a v1.0.0 extra\n",
                &manifest
            )
            .is_err()
        );
        // A second `module` directive is rejected rather than silently winning.
        assert!(parse_go_mod("module example.com/a\nmodule example.com/b\n", &manifest).is_err());
    }

    // ---- Static runnable target classification ----

    /// Write a file (and any missing parent directories) below `root`.
    fn write_file(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// Classify a module rooted at `root`. The root is canonicalized first:
    /// `go list` reports canonical directories and the authoritative helper
    /// strips the directory it was given, so a module behind a symlink has
    /// no target on either path.
    fn static_target(root: &Path) -> Result<Option<String>, Error> {
        let root = AbsoluteSystemPathBuf::new(root.to_str().expect("root is UTF-8")).unwrap();
        runnable_target_statically(&root)
    }

    /// A module fixture whose root is canonical, like `go list` reports.
    fn module_root(tempdir: &tempfile::TempDir) -> PathBuf {
        dunce::canonicalize(tempdir.path()).unwrap()
    }

    #[test]
    fn lexes_package_clauses_and_imports_like_the_go_command() {
        // Comments in every position the header reader allows, a byte order
        // mark, and a newline between `package` and the name.
        assert_eq!(
            lex_go_package_clause(b"package main\n"),
            Ok(b"main".to_vec())
        );
        assert_eq!(
            lex_go_package_clause(b"// lead\n\n/* block */ package lib\n"),
            Ok(b"lib".to_vec())
        );
        assert_eq!(
            lex_go_package_clause(b"\xef\xbb\xbfpackage main\n"),
            Ok(b"main".to_vec())
        );
        assert_eq!(
            lex_go_package_clause(b"package\nmain\n"),
            Ok(b"main".to_vec())
        );
        // The import section is checked too, because a broken one fails the
        // whole `go list` invocation.
        assert_eq!(
            lex_go_package_clause(b"package main\nimport \"a\"\n"),
            Ok(b"main".to_vec())
        );
        assert!(lex_go_package_clause(b"package main\nimport (\n\t\"a\"\n\t\"b\"\n)\n").is_ok());
        assert!(lex_go_package_clause(b"package main\nimport al `raw`\nimport . \"d\"\n").is_ok());
        // Function bodies are never read.
        assert!(lex_go_package_clause(b"package main\nfunc main() { not go }\n").is_ok());
        // Missing clauses, NUL bytes, unterminated comments, a lone `/`, and
        // a keyword run together with its operand.
        assert!(lex_go_package_clause(b"func f() {}\n").is_err());
        assert!(lex_go_package_clause(b"package ma\x00in\n").is_err());
        assert!(lex_go_package_clause(b"/* unterminated\npackage main\n").is_err());
        assert!(lex_go_package_clause(b"package /x\n").is_err());
        assert!(lex_go_package_clause(b"packagex main\n").is_err());
        // A clause beginning with a digit fails `go/parser` and the listing.
        assert!(lex_go_package_clause(b"package 42\n").is_err());
    }

    #[test]
    fn detects_build_constraints_where_the_go_command_does() {
        let scan = |text: &[u8]| scan_go_file_header(text);
        // A `//go:build` line counts anywhere before the package clause:
        // ahead of the doc comment, after a closed block comment, and after
        // a blank line.
        assert_eq!(
            scan(b"//go:build linux\n\npackage main\n").go_build,
            ["linux".to_string()]
        );
        assert_eq!(
            scan(b"// Package doc.\n//go:build ignore\n\npackage main\n").go_build,
            ["ignore".to_string()]
        );
        assert_eq!(
            scan(b"/*\ncopyright\n*/\n//go:build ignore\n\npackage main\n").go_build,
            ["ignore".to_string()]
        );
        // After the package clause it is an ordinary comment, without
        // whitespace after the prefix it is not a directive, inside a block
        // comment it is comment text, and a byte order mark hides it.
        assert!(
            scan(b"package main\n\n//go:build integration\n")
                .go_build
                .is_empty()
        );
        assert!(
            scan(b"//go:buildignore\n\npackage main\n")
                .go_build
                .is_empty()
        );
        assert!(
            scan(b"/* //go:build ignore */\n\npackage main\n")
                .go_build
                .is_empty()
        );
        assert!(
            scan(b"\xef\xbb\xbf//go:build linux\n\npackage main\n")
                .go_build
                .is_empty()
        );
        // More than one `//go:build` line fails the `go` command.
        assert_eq!(
            scan(b"//go:build linux\n\n//go:build darwin\n\npackage main\n")
                .go_build
                .len(),
            2
        );
        // `+build` lines count only in the region before the blank line that
        // precedes the package clause, and the space after `//` is optional.
        assert!(scan(b"// +build ignore\n\npackage main\n").plus_build);
        assert!(scan(b"//+build ignore\n\npackage main\n").plus_build);
        assert!(!scan(b"// +build ignore\npackage main\n").plus_build);
        assert!(!scan(b"package main\n").plus_build);
        // A `//go:build` line controls, so no `+build` fallback is scanned.
        let both = scan(b"//go:build linux\n\n// +build ignore\n\npackage main\n");
        assert_eq!(both.go_build, ["linux".to_string()]);
        assert!(!both.plus_build);
    }

    #[test]
    fn validates_go_build_expressions_like_the_go_command() {
        for valid in [
            "linux",
            "go1.21",
            "!windows",
            "a && b || c",
            "(a || b) && !c",
            "unix && !ignore",
            "a&&(b||!c)",
            "héllo",
        ] {
            assert!(go_build_expression_parses(valid), "rejected {valid:?}");
        }
        for invalid in [
            "", "!!a", "a &&", "(a", "a)", "a b", "a & b", "&&", "!", "a && (b", ")a",
        ] {
            assert!(!go_build_expression_parses(invalid), "accepted {invalid:?}");
        }
    }

    #[test]
    fn recognizes_goos_and_goarch_file_name_constraints() {
        for constrained in [
            "foo_linux.go",
            "foo_amd64.go",
            "foo_darwin_arm64.go",
            "foo_arm64_darwin.go",
            "foo_linux_test.go",
            "foo_test_linux.go",
            "cmd_ios.go",
            "foo_wasm.go",
            "foo_solaris.go",
        ] {
            assert!(
                filename_implies_go_os_or_arch(constrained),
                "missed {constrained}"
            );
        }
        for unconstrained in [
            "linux.go",
            "foo.go",
            "foo_test.go",
            "foo_bar.go",
            "foo_v2.go",
            "a.b.go",
            "foo_Linux.go",
        ] {
            assert!(
                !filename_implies_go_os_or_arch(unconstrained),
                "over-constrained {unconstrained}"
            );
        }
    }

    #[test]
    fn classifies_sole_main_packages_without_go() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        // Several files in one main package, and a library subpackage.
        write_file(&root, "main.go", "package main\nfunc main() {}\n");
        write_file(&root, "helper.go", "package main\n");
        write_file(&root, "lib/helper.go", "package lib\n");
        assert_eq!(static_target(&root).unwrap(), Some(".".to_string()));
    }

    #[test]
    fn classifies_a_single_cmd_main_package() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        write_file(&root, "lib.go", "package lib\n");
        write_file(
            &root,
            "cmd/service/main.go",
            "package main\nfunc main() {}\n",
        );
        assert_eq!(
            static_target(&root).unwrap(),
            Some("./cmd/service".to_string())
        );
    }

    #[test]
    fn reports_no_target_for_ambiguous_or_absent_mains() {
        for (files, name) in [
            (
                vec![
                    ("cmd/first/main.go", "package main\nfunc main() {}\n"),
                    ("cmd/second/main.go", "package main\nfunc main() {}\n"),
                ],
                "two main packages",
            ),
            (vec![("lib.go", "package lib\n")], "a library module"),
            (vec![], "a module without sources"),
        ] {
            let tempdir = tempfile::tempdir().unwrap();
            let root = module_root(&tempdir);
            write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
            for (relative, contents) in files {
                write_file(&root, relative, contents);
            }
            assert_eq!(
                static_target(&root).unwrap(),
                None,
                "{name}: the dev default must be omitted"
            );
        }
    }

    #[test]
    fn excludes_ignored_and_nested_directories_like_go_list() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        write_file(&root, "main.go", "package main\nfunc main() {}\n");
        // Every one of these would add a second `main` package if traversed.
        write_file(&root, ".hidden/x.go", "package main\n");
        write_file(&root, "_hidden/x.go", "package main\n");
        write_file(&root, "testdata/x.go", "package main\n");
        // `go list ./...` ignores paths with a `vendor` element anywhere
        // except the last: a package *named* `vendor` is matched (kept a
        // library here so it adds no second `main`), while nothing beneath
        // it is.
        write_file(&root, "vendor/direct.go", "package lib\n");
        write_file(&root, "vendor/x/x.go", "package main\n");
        write_file(&root, "sub/vendor/x/x.go", "package main\n");
        write_file(
            &root,
            "nested/go.mod",
            "module example.com/nested\n\ngo 1.22\n",
        );
        write_file(&root, "nested/x.go", "package main\n");
        // Ignored file names do not count as sources either.
        write_file(&root, "_underscored.go", "package main\n");
        write_file(&root, ".dotted.go", "package main\n");
        assert_eq!(static_target(&root).unwrap(), Some(".".to_string()));
    }

    #[test]
    fn reads_test_only_main_packages() {
        // A `_test.go` file settles the directory's package when nothing
        // else does, and an external test clause contributes its stem.
        for (file, clause) in [
            ("main_test.go", "package main\n"),
            ("x_test.go", "package main_test\n"),
        ] {
            let tempdir = tempfile::tempdir().unwrap();
            let root = module_root(&tempdir);
            write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
            write_file(&root, file, clause);
            assert_eq!(
                static_target(&root).unwrap(),
                Some(".".to_string()),
                "{file}"
            );
        }
        // A library and its external test package stay a library.
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        write_file(&root, "lib.go", "package lib\n");
        write_file(&root, "lib_test.go", "package lib_test\n");
        assert_eq!(static_target(&root).unwrap(), None);
    }

    #[test]
    fn drops_the_target_when_go_list_would_fail() {
        for (files, name) in [
            (
                vec![("a.go", "package main\n"), ("b.go", "package lib\n")],
                "conflicting package clauses",
            ),
            (vec![("x.go", "func f() {}\n")], "a missing package clause"),
            (
                vec![("main.go", "package main\nimport \"x\n")],
                "a broken import",
            ),
        ] {
            let tempdir = tempfile::tempdir().unwrap();
            let root = module_root(&tempdir);
            write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
            for (relative, contents) in files {
                write_file(&root, relative, contents);
            }
            // `go list` exits non-zero, so the authoritative helper drops the
            // dev default; the static scanner must agree rather than fail.
            assert_eq!(
                static_target(&root).unwrap(),
                None,
                "{name}: the failure must drop the dev default"
            );
        }
    }

    #[test]
    fn refuses_when_build_constraints_decide_the_target() {
        for (files, name) in [
            (
                vec![
                    ("lib.go", "package lib\n"),
                    (
                        "cmd/service/main.go",
                        "//go:build integration\n\npackage main\nfunc main() {}\n",
                    ),
                ],
                "a build-constrained main beside a library",
            ),
            (
                vec![
                    ("lib.go", "package lib\n"),
                    ("m_linux.go", "package main\nfunc main() {}\n"),
                ],
                "a GOOS-suffixed main beside a library",
            ),
            (
                vec![
                    ("lib.go", "package lib\n"),
                    (
                        "c/main.go",
                        "// +build integration\n\npackage main\nfunc main() {}\n",
                    ),
                ],
                "a +build-constrained main in an otherwise empty directory",
            ),
        ] {
            let tempdir = tempfile::tempdir().unwrap();
            let root = module_root(&tempdir);
            write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
            for (relative, contents) in files {
                write_file(&root, relative, contents);
            }
            assert!(
                matches!(
                    static_target(&root),
                    Err(Error::StaticDiscoveryUnavailable { .. })
                ),
                "{name}: classification must refuse instead of guessing"
            );
        }
    }

    #[test]
    fn keeps_constrained_files_that_agree_with_their_package() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        // Whether `lib_integration.go` is included, the package is `lib`, so
        // the classification is environment-independent.
        write_file(&root, "lib.go", "package lib\n");
        write_file(
            &root,
            "lib_integration.go",
            "//go:build integration\n\npackage lib\n",
        );
        write_file(&root, "cmd/m/main.go", "package main\nfunc main() {}\n");
        assert_eq!(static_target(&root).unwrap(), Some("./cmd/m".to_string()));
    }

    #[test]
    fn treats_post_clause_build_comments_as_plain_comments() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "go.mod", "module example.com/m\n\ngo 1.22\n");
        // Neither comment is a constraint: one follows the package clause
        // and the other is not followed by a blank line.
        write_file(
            &root,
            "main.go",
            "// +build ignore\npackage main\nfunc main() {}\n\n//go:build integration\n",
        );
        assert_eq!(static_target(&root).unwrap(), Some(".".to_string()));
    }

    #[test]
    fn refuses_for_go_mod_ignore_directives() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(
            &root,
            "go.mod",
            "module example.com/m\n\ngo 1.22\n\nignore ./tools\n",
        );
        write_file(&root, "main.go", "package main\nfunc main() {}\n");
        assert!(matches!(
            static_target(&root),
            Err(Error::StaticDiscoveryUnavailable { .. })
        ));
    }

    #[test]
    fn reports_no_target_without_a_module_manifest() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = module_root(&tempdir);
        write_file(&root, "main.go", "package main\nfunc main() {}\n");
        assert_eq!(static_target(&root).unwrap(), None);
    }
}
