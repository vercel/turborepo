use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use turbopath::{
    AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};
use turborepo_lockfiles::Lockfile;

use super::{ExternalDependencyChange, PackageGraph, PackageName, ROOT_PKG_NAME, WorkspacePackage};
use crate::{
    external_resolution::{
        ExternalFileHashes, ExternalPackageIdentity, ExternalResolutionChanges,
        ExternalResolutionData, ExternalResolutionDomain, ExternalResolutionGeneration,
        JAVASCRIPT_RESOLUTION_DOMAIN, PackageResolution, ResolutionCompleteness,
        ResolutionUnavailableReason, compare_resolution_data,
    },
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    package_json::DependencyKind,
    package_manager::PackageManager,
    toolchain::ToolchainId,
};

pub(super) struct ResolutionSnapshot {
    pub generation: Arc<ExternalResolutionGeneration>,
    pub warning: Option<String>,
}

fn resolution_packages(knowledge: &RepositoryKnowledge) -> Vec<(&str, &AnchoredSystemPath)> {
    let mut packages = knowledge.package_json_packages().collect::<Vec<_>>();
    packages.sort_unstable_by_key(|(identity, _)| *identity);
    packages
}

pub(super) fn external_dependencies(
    knowledge: &RepositoryKnowledge,
    relationships: &RelationshipKnowledge,
) -> HashMap<String, BTreeMap<String, String>> {
    let packages = resolution_packages(knowledge);
    let directories = packages
        .iter()
        .map(|(identity, directory)| ((*identity).to_string(), *directory))
        .collect::<HashMap<_, _>>();
    let mut by_source: BTreeMap<String, BTreeMap<String, String>> = packages
        .into_iter()
        .map(|(identity, _)| (identity.to_string(), BTreeMap::new()))
        .collect();
    // Iterate the relationship groups directly rather than materializing an
    // owned `ExternalDeclarations` (one struct per declaration, cloning the
    // group source and declaration name that this loop only reads). Only the
    // `name`/`specifier` that actually land in `by_source` are cloned. The
    // per-group dedup by declaration name — first occurrence wins, applied
    // before the external filter — matches `ExternalDeclarations::build`.
    for group in relationships.groups() {
        let Some(dependencies) = by_source.get_mut(group.source()) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for relationship in group.relationships() {
            if !seen.insert(relationship.declaration_name()) {
                continue;
            }
            let crate::relationships::RelationshipTarget::UnresolvedExternal { name, specifier } =
                relationship.target()
            else {
                continue;
            };
            if matches!(relationship.kind(), DependencyKind::Peer { .. }) {
                continue;
            }
            dependencies
                .entry(name.clone())
                .or_insert_with(|| specifier.clone());
        }
    }

    by_source
        .into_iter()
        .filter_map(|(identity, dependencies)| {
            let directory = directories.get(&identity)?;
            Some((directory.to_unix().to_string(), dependencies))
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "package.json is a validated static path"
)]
pub(super) fn unavailable_resolution(
    knowledge: &RepositoryKnowledge,
    mut domains: Vec<ExternalResolutionDomain>,
    definition_source: AnchoredSystemPathBuf,
    code: &str,
    message: String,
    warning: Option<String>,
) -> Result<ResolutionSnapshot, String> {
    let members = resolution_packages(knowledge)
        .into_iter()
        .map(|(identity, _)| identity.to_string())
        .collect::<Vec<_>>();
    domains.push(
        ExternalResolutionDomain::new(
            JAVASCRIPT_RESOLUTION_DOMAIN.clone(),
            ToolchainId::JAVASCRIPT,
            AnchoredSystemPathBuf::default(),
            members,
            [definition_source],
            ExternalResolutionData::Unavailable(ResolutionUnavailableReason::new(code, message)),
        )
        .with_fallback_inputs([
            AnchoredSystemPathBuf::from_raw("package.json").expect("static path is valid")
        ]),
    );
    let generation = ExternalResolutionGeneration::build(knowledge, domains)
        .map_err(|error| error.to_string())?;
    Ok(ResolutionSnapshot {
        generation: Arc::new(generation),
        warning,
    })
}

pub(super) fn resolve_dependencies(
    repo_root: &AbsoluteSystemPath,
    knowledge: &RepositoryKnowledge,
    mut domains: Vec<ExternalResolutionDomain>,
    lockfile: &dyn Lockfile,
    external_dependencies: HashMap<String, BTreeMap<String, String>>,
    ignore_missing_packages: bool,
    definition_source: AnchoredSystemPathBuf,
) -> Result<ResolutionSnapshot, String> {
    let closures = match turborepo_lockfiles::all_transitive_closures_indexed(
        lockfile,
        external_dependencies,
        ignore_missing_packages,
    ) {
        Ok(closures) => closures,
        Err(error) => {
            if (lockfile as &dyn std::any::Any).is::<turborepo_lockfiles::BunLockfile>()
                && !lockfile
                    .patches()
                    .map_err(|error| error.to_string())?
                    .is_empty()
            {
                return Err(format!("unable to resolve Bun patch file inputs: {error}"));
            }
            let message = error.to_string();
            return unavailable_resolution(
                knowledge,
                domains,
                definition_source,
                "closure-unavailable",
                message.clone(),
                Some(message),
            );
        }
    };
    let (package_files, file_hashes) = hash_file_inputs(repo_root, lockfile, &closures.packages)?;
    let packages = package_resolutions(knowledge, lockfile, closures, &package_files, &file_hashes);
    let members = packages
        .iter()
        .map(|package| package.package().to_string())
        .collect::<Vec<_>>();
    domains.push(ExternalResolutionDomain::new(
        JAVASCRIPT_RESOLUTION_DOMAIN.clone(),
        ToolchainId::JAVASCRIPT,
        AnchoredSystemPathBuf::default(),
        members,
        [definition_source],
        ExternalResolutionData::Resolved {
            completeness: ResolutionCompleteness::Complete,
            packages,
        },
    ));
    let generation = ExternalResolutionGeneration::build(knowledge, domains)
        .map_err(|error| error.to_string())?;
    Ok(ResolutionSnapshot {
        generation: Arc::new(generation),
        warning: None,
    })
}

fn package_resolutions(
    knowledge: &RepositoryKnowledge,
    lockfile: &dyn Lockfile,
    closures: turborepo_lockfiles::IndexedClosures,
    package_files: &[Vec<RelativeUnixPathBuf>],
    file_hashes: &ExternalFileHashes,
) -> Vec<PackageResolution> {
    // Workspaces sharing an identical external closure (common when a
    // monorepo has many packages with the same dependencies) share one
    // materialized identity list instead of each cloning every member's
    // strings and re-reading its lockfile display name. Closures arrive as
    // ascending indices into one sorted, duplicate-free package table, so an
    // identical index sequence means an identical closure.
    //
    // The identity lists for the *distinct* closures are built in parallel:
    // this is the bulk of resolution assembly on large monorepos (an
    // identity clone per closure member).
    use rayon::prelude::*;

    let turborepo_lockfiles::IndexedClosures {
        packages: table,
        closures,
    } = closures;
    let resolution_members = resolution_packages(knowledge);
    let mut index_of: HashMap<&[u32], usize, IndexHashBuilder> = HashMap::default();
    let mut distinct_closures: Vec<&[u32]> = Vec::new();
    let mut plan: Vec<(&str, usize)> = Vec::with_capacity(resolution_members.len());
    for &(identity, directory) in &resolution_members {
        let members: &[u32] = closures
            .get(directory.to_unix().as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let idx = *index_of.entry(members).or_insert_with(|| {
            distinct_closures.push(members);
            distinct_closures.len() - 1
        });
        plan.push((identity, idx));
    }
    drop(index_of);

    // Intern one `ExternalPackageIdentity` per package in the table, in
    // parallel (the key/version clones and lockfile `human_name` lookup), so
    // its strings are allocated once and every closure that contains it
    // clones one shared identity.
    let interned: Vec<ExternalPackageIdentity> = table
        .par_iter()
        .map(|package| {
            let mut identity =
                ExternalPackageIdentity::new(package.key.as_str(), package.version.as_str());
            if let Some(human_name) = lockfile.human_name(package) {
                identity = identity.with_human_name(human_name);
            }
            identity
        })
        .collect();

    // Assemble the lists in parallel by cloning interned identities.
    let built: Vec<_> = distinct_closures
        .par_iter()
        .map(|indices| {
            let identities = PackageResolution::shared_identity_list(
                indices.iter().map(|&i| interned[i as usize].clone()),
            );
            let files: ExternalFileHashes = indices
                .iter()
                .flat_map(|&i| package_files.get(i as usize).into_iter().flatten())
                .map(|path| (path.clone(), file_hashes[path]))
                .collect();
            (identities, Arc::new(files))
        })
        .collect();
    drop(distinct_closures);

    plan.into_iter()
        .map(|(identity, idx)| {
            let (identities, files) = &built[idx];
            PackageResolution::from_shared(identity, Arc::clone(identities))
                .with_file_inputs(Arc::clone(files))
        })
        .collect()
}

fn validate_file_input(
    repo_root: &AbsoluteSystemPath,
    path: &RelativeUnixPathBuf,
) -> Result<(), String> {
    // Reject Windows prefixes and separators on every host as these inputs
    // must retain the same repository-relative meaning across platforms.
    if path.as_str().contains('\\') || path.as_str().as_bytes().get(1) == Some(&b':') {
        return Err(format!("not a repository-relative Unix path: {path}"));
    }
    let absolute = repo_root.resolve(&path.to_anchored_system_path_buf());
    if !repo_root.contains(&absolute) {
        return Err(format!("path escapes the repository: {path}"));
    }
    Ok(())
}

fn hash_file_inputs(
    repo_root: &AbsoluteSystemPath,
    lockfile: &dyn Lockfile,
    packages: &[Arc<turborepo_lockfiles::Package>],
) -> Result<(Vec<Vec<RelativeUnixPathBuf>>, ExternalFileHashes), String> {
    let mut used_paths = BTreeMap::new();
    let mut package_files = Vec::with_capacity(packages.len());
    for package in packages {
        let context = |error| format!("external file input for {}: {error}", package.key);
        let paths = lockfile
            .external_file_inputs(package)
            .map_err(|error| context(error.to_string()))?;
        for path in &paths {
            validate_file_input(repo_root, path).map_err(&context)?;
            used_paths
                .entry(path.clone())
                .or_insert(package.key.as_str());
        }
        package_files.push(paths);
    }

    let mut hashes = ExternalFileHashes::new();
    if !used_paths.is_empty() {
        let physical_root = dunce::canonicalize(repo_root.as_std_path())
            .map_err(|error| format!("external file inputs in {repo_root}: {error}"))?;
        for (path, package) in used_paths {
            let context = |error| format!("external file input {path} for {package}: {error}");
            let absolute = repo_root.resolve(&path.to_anchored_system_path_buf());
            let physical_path = dunce::canonicalize(absolute.as_std_path())
                .map_err(|error| context(error.to_string()))?;
            if !physical_path.starts_with(&physical_root) {
                return Err(context("symlink target escapes the repository".to_string()));
            }
            let bytes =
                std::fs::read(&physical_path).map_err(|error| context(error.to_string()))?;
            hashes.insert(path, xxhash_rust::xxh64::xxh64(&bytes, 0).to_le_bytes());
        }
    }
    Ok((package_files, hashes))
}

fn lockfile_identity_resolution(
    knowledge: &RepositoryKnowledge,
    lockfile: &dyn Lockfile,
    dependencies: HashMap<String, BTreeMap<String, String>>,
) -> Result<ExternalResolutionData, String> {
    let closures =
        turborepo_lockfiles::all_transitive_closures_indexed(lockfile, dependencies, true)
            .map_err(|error| error.to_string())?;
    Ok(ExternalResolutionData::Resolved {
        completeness: ResolutionCompleteness::Complete,
        packages: package_resolutions(
            knowledge,
            lockfile,
            closures,
            &[],
            &ExternalFileHashes::new(),
        ),
    })
}

/// Multiplicative (Fx-style) hasher for closure index slices. Grouping
/// hashes every closure member (workspaces x closure size on large
/// monorepos), and the keys are internal, so SipHash's DoS resistance buys
/// nothing; slices are consumed eight bytes at a time.
#[derive(Default)]
struct IndexHasher(u64);

impl std::hash::Hasher for IndexHasher {
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }

    fn write(&mut self, bytes: &[u8]) {
        let (chunks, remainder) = bytes.as_chunks::<8>();
        for &chunk in chunks {
            self.write_u64(u64::from_le_bytes(chunk));
        }
        for &byte in remainder {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        const SEED: u64 = 0xf135_7aea_2e62_a9c5;
        self.0 = self.0.wrapping_add(value).wrapping_mul(SEED);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

type IndexHashBuilder = std::hash::BuildHasherDefault<IndexHasher>;

#[cfg(test)]
mod tests {
    use std::hash::Hasher;

    use turbopath::AbsoluteSystemPath;
    use turborepo_lockfiles::BunLockfile;

    use super::*;
    use crate::{
        knowledge::{PackageScopeObservation, ScopeKind, WorkspaceRootObservation},
        toolchain::WorkspaceRoot,
    };

    fn bun_lockfile(patch: &str) -> BunLockfile {
        serde_json::json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": { "name": "root" },
                "direct": { "name": "direct" },
                "transitive": { "name": "transitive" },
                "unrelated": { "name": "unrelated" }
            },
            "packages": {
                "foo": ["foo@1.0.0", "", {}, "sha512-foo"],
                "parent": ["parent@1.0.0", "", { "dependencies": { "foo": "1.0.0" } }, "sha512-parent"],
                "bar": ["bar@1.0.0", "", {}, "sha512-bar"]
            },
            "patchedDependencies": {
                "foo@1.0.0": patch,
                "unused@1.0.0": "missing-unused.patch"
            }
        })
        .to_string()
        .parse()
        .unwrap()
    }

    fn test_repository(root: &AbsoluteSystemPath) -> RepositoryKnowledge {
        let observations =
            ["direct", "transitive", "unrelated"].map(|name| PackageScopeObservation {
                identity: Some(name.to_string()),
                name_source: None,
                definition_path: root.join_components(&[name, "package.json"]),
                toolchain: ToolchainId::JAVASCRIPT,
                scope_kind: ScopeKind::Package,
            });
        RepositoryKnowledge::build(
            root,
            Some(Some("root".to_string())),
            &observations,
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("bun", root.to_owned()),
                ToolchainId::JAVASCRIPT,
            )],
        )
        .unwrap()
    }

    fn test_dependencies() -> HashMap<String, BTreeMap<String, String>> {
        [
            ("", "foo"),
            ("direct", "foo"),
            ("transitive", "parent"),
            ("unrelated", "bar"),
        ]
        .into_iter()
        .map(|(workspace, dependency)| {
            (
                workspace.to_string(),
                BTreeMap::from([(dependency.to_string(), "1.0.0".to_string())]),
            )
        })
        .collect()
    }

    fn test_snapshot(
        root: &AbsoluteSystemPath,
        knowledge: &RepositoryKnowledge,
        lockfile: &dyn Lockfile,
    ) -> Result<ResolutionSnapshot, String> {
        resolve_dependencies(
            root,
            knowledge,
            Vec::new(),
            lockfile,
            test_dependencies(),
            false,
            AnchoredSystemPathBuf::from_raw("bun.lock").unwrap(),
        )
    }

    #[test]
    fn bun_patch_bytes_change_only_consuming_fingerprints() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        let lockfile = bun_lockfile("foo.patch");
        std::fs::write(temp.path().join("foo.patch"), b"first patch\r\n").unwrap();
        let before = test_snapshot(root, &knowledge, &lockfile).unwrap();
        std::fs::write(temp.path().join("foo.patch"), b"second patch\xff\n").unwrap();
        let after = test_snapshot(root, &knowledge, &lockfile).unwrap();
        let before = resolution_data(&before.generation).unwrap();
        let after = resolution_data(&after.generation).unwrap();
        for name in ["//", "direct", "transitive", "unrelated"] {
            let before = before.package_resolution(name).unwrap();
            let after = after.package_resolution(name).unwrap();
            assert_eq!(before.identities(), after.identities());
            if name == "unrelated" {
                assert_eq!(before.fingerprint(), after.fingerprint());
            } else {
                assert_ne!(before.fingerprint(), after.fingerprint(), "{name}");
            }
        }
    }

    #[test]
    fn missing_used_bun_patch_is_fatal() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        let error = test_snapshot(root, &knowledge, &bun_lockfile("missing.patch"))
            .err()
            .expect("a missing patch must fail current resolution");
        assert!(error.contains("missing.patch"), "{error}");
        assert!(error.contains("foo@1.0.0"), "{error}");
    }

    #[test]
    fn historical_bun_resolution_never_reads_current_patch_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        let previous = lockfile_identity_resolution(
            &knowledge,
            &bun_lockfile("absent-old.patch"),
            test_dependencies(),
        )
        .unwrap();
        let old = previous.package_resolution("direct").unwrap();
        assert_eq!(old.identities()[0].key(), "foo@1.0.0");
        assert_eq!(old.identities()[0].version(), "1.0.0+absent-old.patch");
        std::fs::write(temp.path().join("current.patch"), b"current patch").unwrap();
        let current = test_snapshot(root, &knowledge, &bun_lockfile("current.patch")).unwrap();
        assert_eq!(
            compare_resolution_data(
                resolution_data(&current.generation).unwrap(),
                &previous,
                false,
                ROOT_PKG_NAME
            )
            .unwrap(),
            ExternalResolutionChanges::All
        );
    }

    #[test]
    fn bun_closure_failure_with_patches_cannot_use_lockfile_only_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        let resolve = |lockfile: &dyn Lockfile| {
            resolve_dependencies(
                root,
                &knowledge,
                Vec::new(),
                lockfile,
                HashMap::from([(
                    "absent-workspace".to_string(),
                    BTreeMap::from([("foo".to_string(), "1.0.0".to_string())]),
                )]),
                false,
                AnchoredSystemPathBuf::from_raw("bun.lock").unwrap(),
            )
        };
        let error = resolve(&bun_lockfile("missing.patch"))
            .err()
            .expect("patches must not be omitted on closure failure");
        assert!(error.contains("Bun patch file inputs"), "{error}");

        let unpatched: BunLockfile = r#"{
            "lockfileVersion": 1,
            "workspaces": { "": { "name": "root" } },
            "packages": {}
        }"#
        .parse()
        .unwrap();
        let fallback = resolve(&unpatched).unwrap();
        assert!(fallback.warning.is_some());
        assert!(matches!(
            resolution_data(&fallback.generation).unwrap(),
            ExternalResolutionData::Unavailable(_)
        ));
    }

    #[test]
    fn external_file_inputs_preserve_and_deduplicate_declared_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        std::fs::write(temp.path().join("foo.patch"), b"patch\0\xff\r\n").unwrap();
        std::fs::write(temp.path().join(".gitignore"), "*.patch\n").unwrap();
        let lockfile = bun_lockfile("./nested/../foo.patch");
        let package = Arc::new(turborepo_lockfiles::Package::new("foo@1.0.0", "1.0.0"));
        let (files, hashes) =
            hash_file_inputs(root, &lockfile, &[Arc::clone(&package), package]).unwrap();
        let path = RelativeUnixPathBuf::new("./nested/../foo.patch").unwrap();
        assert_eq!(files, vec![vec![path.clone()], vec![path.clone()]]);
        assert_eq!(hashes.len(), 1);
        assert_eq!(
            hashes[&path],
            xxhash_rust::xxh64::xxh64(b"patch\0\xff\r\n", 0).to_le_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_file_inputs_reject_missing_intermediate_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        std::fs::write(temp.path().join("foo.patch"), b"patch").unwrap();
        let lockfile = bun_lockfile("missing/../foo.patch");
        let package = Arc::new(turborepo_lockfiles::Package::new("foo@1.0.0", "1.0.0"));
        let error = hash_file_inputs(root, &lockfile, &[package]).unwrap_err();
        assert!(error.contains("missing/../foo.patch"), "{error}");
        assert!(error.contains("foo@1.0.0"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn external_file_inputs_follow_symlink_before_parent_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        std::fs::create_dir_all(temp.path().join("target/nested")).unwrap();
        std::fs::write(temp.path().join("foo.patch"), b"lexical").unwrap();
        std::fs::write(temp.path().join("target/foo.patch"), b"physical").unwrap();
        symlink("target/nested", temp.path().join("link")).unwrap();
        let lockfile = bun_lockfile("link/../foo.patch");
        let package = Arc::new(turborepo_lockfiles::Package::new("foo@1.0.0", "1.0.0"));
        let (files, hashes) = hash_file_inputs(root, &lockfile, &[package]).unwrap();
        let path = RelativeUnixPathBuf::new("link/../foo.patch").unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(
            hashes.values().next().unwrap(),
            &xxhash_rust::xxh64::xxh64(b"physical", 0).to_le_bytes()
        );
        assert_eq!(files, vec![vec![path.clone()]]);
        assert!(hashes.contains_key(&path));
    }

    #[cfg(unix)]
    #[test]
    fn external_file_inputs_reject_symlink_parent_escape() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        std::fs::create_dir(outside.path().join("nested")).unwrap();
        std::fs::write(temp.path().join("foo.patch"), b"lexical").unwrap();
        std::fs::write(outside.path().join("foo.patch"), b"outside").unwrap();
        symlink(outside.path().join("nested"), temp.path().join("link")).unwrap();
        let lockfile = bun_lockfile("link/../foo.patch");
        let package = Arc::new(turborepo_lockfiles::Package::new("foo@1.0.0", "1.0.0"));
        let error = hash_file_inputs(root, &lockfile, &[package]).unwrap_err();
        assert!(error.contains("link/../foo.patch"), "{error}");
        assert!(error.contains("foo@1.0.0"), "{error}");
        assert!(error.contains("escapes"), "{error}");
    }

    #[test]
    fn invalid_bun_patch_paths_and_directories_are_fatal() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        std::fs::create_dir(temp.path().join("directory.patch")).unwrap();
        for path in [
            "../escape.patch",
            "/absolute.patch",
            "C:/absolute.patch",
            r"C:\absolute.patch",
            "directory.patch",
        ] {
            let error = test_snapshot(root, &knowledge, &bun_lockfile(path))
                .err()
                .expect(path);
            assert!(error.contains("foo@1.0.0"), "{error}");
            assert!(error.contains(path), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn bun_patch_symlinks_hash_contained_target_bytes_and_reject_escapes() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let knowledge = test_repository(root);
        let lockfile = bun_lockfile("link.patch");
        std::fs::write(temp.path().join("target.patch"), b"first").unwrap();
        symlink("target.patch", temp.path().join("link.patch")).unwrap();
        let before = test_snapshot(root, &knowledge, &lockfile).unwrap();
        std::fs::write(temp.path().join("target.patch"), b"second").unwrap();
        let after = test_snapshot(root, &knowledge, &lockfile).unwrap();
        assert_ne!(
            resolution_data(&before.generation)
                .unwrap()
                .package_resolution("direct")
                .unwrap()
                .fingerprint(),
            resolution_data(&after.generation)
                .unwrap()
                .package_resolution("direct")
                .unwrap()
                .fingerprint()
        );
        std::fs::remove_file(temp.path().join("link.patch")).unwrap();
        std::fs::write(outside.path().join("outside.patch"), b"outside").unwrap();
        symlink(
            outside.path().join("outside.patch"),
            temp.path().join("link.patch"),
        )
        .unwrap();
        let error = test_snapshot(root, &knowledge, &lockfile)
            .err()
            .expect("escaping symlink must fail");
        assert!(error.contains("link.patch"), "{error}");
        assert!(error.contains("foo@1.0.0"), "{error}");
        assert!(error.contains("escapes"), "{error}");
    }

    #[test]
    fn index_hasher_preserves_byte_hashes_for_all_remainders_and_alignments() {
        let bytes: Vec<u8> = (0..136).map(|i| (i * 73) as u8).collect();
        for start in 0..8 {
            for len in 0..=128 {
                let bytes = &bytes[start..start + len];
                let mut hasher = IndexHasher::default();
                hasher.write(bytes);

                let whole_bytes = bytes.len() / 8 * 8;
                let words = bytes[..whole_bytes]
                    .chunks(8)
                    .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()));
                let tail = bytes[whole_bytes..].iter().copied().map(u64::from);
                let expected = words.chain(tail).fold(0u64, |hash, value| {
                    hash.wrapping_add(value).wrapping_mul(0xf135_7aea_2e62_a9c5)
                });
                assert_eq!(hasher.finish(), expected.rotate_left(26));
            }
        }
    }
}

fn resolution_data(
    generation: &ExternalResolutionGeneration,
) -> Result<&ExternalResolutionData, ChangedPackagesError> {
    generation
        .domain(&JAVASCRIPT_RESOLUTION_DOMAIN)
        .map(ExternalResolutionDomain::data)
        .ok_or(ChangedPackagesError::ResolutionUnavailable)
}

impl PackageGraph {
    pub(super) fn changed_javascript_packages_from_lockfile_contents(
        &self,
        previous_lockfile_contents: &[u8],
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let package_manager = self
            .package_manager()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let root_package_json = self
            .root_package_json
            .as_ref()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let yarnrc = matches!(package_manager, PackageManager::Berry)
            .then(|| crate::package_manager::yarnrc::YarnRc::from_file(self.repo_root()))
            .transpose()?;
        let previous = package_manager.parse_lockfile(
            root_package_json,
            previous_lockfile_contents,
            yarnrc,
        )?;
        self.changed_packages_from_lockfile(previous.as_ref())
    }

    /// Returns packages whose normalized external resolution changed from the
    /// provided previous lockfile. Callers remain responsible for detecting
    /// descriptor changes independently.
    pub fn changed_packages_from_lockfile(
        &self,
        previous_lockfile: &dyn Lockfile,
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let current_lockfile = self.lockfile().ok_or(ChangedPackagesError::NoLockfile)?;
        self.package_manager()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let previous_resolution = turborepo_rayon_compat::block_in_place(|| {
            lockfile_identity_resolution(
                &self.knowledge,
                previous_lockfile,
                external_dependencies(&self.knowledge, &self.relationship_knowledge),
            )
        })
        .map_err(ChangedPackagesError::Resolution)?;
        let current_resolution = self
            .external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changes = compare_resolution_data(
            resolution_data(
                current_resolution
                    .generation
                    .as_deref()
                    .ok_or(ChangedPackagesError::ResolutionUnavailable)?,
            )?,
            &previous_resolution,
            current_lockfile.global_change(previous_lockfile),
            ROOT_PKG_NAME,
        )
        .map_err(|_| ChangedPackagesError::ResolutionUnavailable)?;

        let all_changes = || {
            self.package_task_contexts()
                .map(|context| ExternalDependencyChange {
                    package: WorkspacePackage {
                        name: context.package().clone(),
                        path: context.directory().to_owned(),
                    },
                    added: Vec::new(),
                    removed: Vec::new(),
                })
                .collect()
        };
        let ExternalResolutionChanges::Packages(changes) = changes else {
            return Ok(all_changes());
        };
        changes
            .into_iter()
            .map(|change| {
                let name = PackageName::from(change.package);
                let context = self
                    .package_task_context(&name)
                    .ok_or(ChangedPackagesError::ResolutionUnavailable)?;
                let to_lockfile_package = |identity: ExternalPackageIdentity| {
                    turborepo_lockfiles::Package::new(identity.key(), identity.version())
                };
                Ok(ExternalDependencyChange {
                    package: WorkspacePackage {
                        name,
                        path: context.directory().to_owned(),
                    },
                    added: change.added.into_iter().map(to_lockfile_package).collect(),
                    removed: change
                        .removed
                        .into_iter()
                        .map(to_lockfile_package)
                        .collect(),
                })
            })
            .collect()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ChangedPackagesError {
    #[error("No lockfile")]
    NoLockfile,
    #[error("External resolution unavailable")]
    ResolutionUnavailable,
    #[error("External resolution failed: {0}")]
    Resolution(String),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("Package manager error: {0}")]
    PackageManager(#[from] crate::package_manager::Error),
    #[error("Yarn config error: {0}")]
    Yarnrc(#[from] crate::package_manager::yarnrc::Error),
    #[error("uv lockfile error: {0}")]
    Uv(#[from] turborepo_lockfiles::UvLockError),
    #[error("Cargo lockfile error: {0}")]
    Cargo(#[from] turborepo_lockfiles::CargoLockError),
    #[error("Lockfile content is not UTF-8")]
    NonUtf8Lockfile,
}
