#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    str::FromStr,
    sync::{LazyLock, OnceLock},
};

use globwalk::{ValidatedGlob, WalkType};
use miette::Diagnostic;
use tracing::{trace, warn};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
    RelativeUnixPath, RelativeUnixPathBuf,
};
use turborepo_repository::{
    package_graph::{self, PackageGraph, PackageName, PackageNode},
    package_json::PackageJson,
    package_manager::{npmrc::NpmRc, PackageManager},
    toolchain::ToolchainId,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::BOLD;

use super::CommandBase;
use crate::{
    config::{CONFIG_FILE, CONFIG_FILE_JSONC},
    turbo_json::{RawRootTurboJson, RawTurboJson},
};

pub const DEFAULT_OUTPUT_DIR: &str = "out";

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("I/O error while pruning: {0}")]
    Io(#[from] std::io::Error),
    #[error("File system error while pruning. The error from the operating system is: {0}")]
    Fs(#[from] turborepo_fs::Error),
    #[error("JSON error while pruning: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Path error while pruning: {0}")]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJsonParser(#[from] crate::turbo_json::parser::Error),
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    PackageGraph(#[from] package_graph::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error("`turbo` does not support workspaces at file system root.")]
    WorkspaceAtFilesystemRoot,
    #[error("At least one target must be specified.")]
    NoWorkspaceSpecified,
    #[error("Invalid scope. Package with name {0} in `package.json` not found.")]
    MissingWorkspace(PackageName),
    #[error(
        "Invalid patched dependency path `{0}`: path escapes the repository or output directory"
    )]
    InvalidPatchPath(RelativeUnixPathBuf),
    #[error("Cannot prune without parsed lockfile.")]
    MissingLockfile,
    #[error("Unable to read config: {0}")]
    Config(#[from] crate::config::Error),
    #[error("Glob error while resolving globalDependencies: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error("Walk error while resolving globalDependencies: {0}")]
    Walk(#[from] globwalk::WalkError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
    #[error(
        "Cannot prune {0}: it has no directory of its own (it represents the whole repository).          Prune a specific package instead."
    )]
    PackageNotPruneable(String),
    #[error(transparent)]
    Toolchain(#[from] turborepo_repository::toolchain::Error),
}

static ADDITIONAL_FILES: LazyLock<Vec<(&'static RelativeUnixPath, Option<CopyDestination>)>> =
    LazyLock::new(|| {
        vec![
            (relative_unix_path(".gitattributes"), None),
            (relative_unix_path(".gitignore"), None),
            (relative_unix_path(".npmrc"), Some(CopyDestination::Docker)),
            (
                relative_unix_path(".yarnrc.yml"),
                Some(CopyDestination::Docker),
            ),
            (
                relative_unix_path("bunfig.toml"),
                Some(CopyDestination::Docker),
            ),
        ]
    });
static ADDITIONAL_DIRECTORIES: LazyLock<Vec<(&'static RelativeUnixPath, Option<CopyDestination>)>> =
    LazyLock::new(|| {
        vec![
            (
                relative_unix_path(".yarn/plugins"),
                Some(CopyDestination::Docker),
            ),
            (
                relative_unix_path(".yarn/releases"),
                Some(CopyDestination::Docker),
            ),
        ]
    });

fn relative_unix_path(path: &'static str) -> &'static RelativeUnixPath {
    match RelativeUnixPath::new(path) {
        Ok(path) => path,
        Err(_) => unreachable!("static relative Unix path should be valid"),
    }
}

fn anchored_path(path: &'static str) -> &'static AnchoredSystemPath {
    match AnchoredSystemPath::new(path) {
        Ok(path) => path,
        Err(_) => unreachable!("static anchored path should be valid"),
    }
}

fn package_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| anchored_path("package.json"))
}

fn turbo_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| anchored_path(CONFIG_FILE))
}

fn turbo_jsonc() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| anchored_path(CONFIG_FILE_JSONC))
}

#[allow(clippy::expect_used)]
pub async fn prune(
    base: &CommandBase,
    scope: &[String],
    docker: bool,
    production: bool,
    output_dir: &str,
    use_gitignore: bool,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_arg_usage("docker", docker);
    telemetry.track_arg_usage("production", production);
    telemetry.track_arg_usage("out-dir", output_dir != DEFAULT_OUTPUT_DIR);

    let prune = Prune::new(
        base,
        scope,
        docker,
        production,
        output_dir,
        use_gitignore,
        telemetry,
    )
    .await?;

    println!(
        "Generating pruned monorepo for {} in {}",
        base.color_config.apply(BOLD.apply_to(scope.join(", "))),
        base.color_config.apply(BOLD.apply_to(&prune.out_directory)),
    );

    if let Some(workspace_config_path) = prune
        .package_graph
        .package_manager()
        .and_then(|pm| pm.workspace_configuration_path())
    {
        prune.copy_file(
            &AnchoredSystemPathBuf::from_raw(workspace_config_path)?,
            Some(CopyDestination::All),
        )?;
    }

    let mut workspace_paths = Vec::new();
    let mut workspace_names = Vec::new();
    let workspaces = prune.internal_dependencies();
    let retained_workspace_names: HashSet<_> = workspaces
        .iter()
        .filter_map(|workspace| match workspace {
            PackageName::Root => None,
            PackageName::Other(name) => Some(name.as_str()),
        })
        .collect();
    let excluded_dev_workspaces = if prune.production
        && prune
            .package_graph
            .package_manager()
            .is_some_and(|package_manager| {
                package_manager.lockfile_manager() == &PackageManager::Bun
            }) {
        prune
            .package_graph
            .packages()
            .filter_map(|(workspace, _)| match workspace {
                PackageName::Root => None,
                PackageName::Other(name) if !retained_workspace_names.contains(name.as_str()) => {
                    Some(name.clone())
                }
                PackageName::Other(_) => None,
            })
            .collect()
    } else {
        HashSet::new()
    };
    // Only JavaScript packages participate in the JS lockfile subgraph:
    // other toolchains' external-dependency keys (e.g. Cargo's rustc and
    // crates.io identities) mean nothing to it and must not leak in.
    let js_workspaces: Vec<PackageName> = workspaces
        .iter()
        .filter(|workspace| {
            prune
                .package_graph
                .package_info(workspace)
                .is_none_or(|info| info.toolchain == ToolchainId::JAVASCRIPT)
        })
        .cloned()
        .collect();
    // The JS lockfile subgraph only exists when there is a JavaScript package
    // manager. A pure Cargo workspace has none; its lockfile is pruned by the
    // Cargo toolchain's prune plan below.
    let lockfile_keys = if prune.package_graph.package_manager().is_some() {
        prune.lockfile_keys(&js_workspaces)?
    } else {
        Vec::new()
    };
    let mut kept_by_toolchain: HashMap<ToolchainId, Vec<String>> = HashMap::new();
    let mut planned_toolchains = HashSet::new();
    for workspace in workspaces {
        let entry = prune
            .package_graph
            .package_info(&workspace)
            .ok_or_else(|| Error::MissingWorkspace(workspace.clone()))?;

        // We don't want to do any copying for the root workspace
        if let PackageName::Other(workspace) = workspace {
            if entry.toolchain != ToolchainId::JAVASCRIPT {
                // A package anchored at the repo root (the synthetic Cargo
                // workspace package) has no directory of its own; its
                // workspace-level files come from the toolchain's prune
                // plan below.
                if entry.package_path().components().next().is_none() {
                    continue;
                }
                prune.copy_package_dir(entry.package_json_path())?;
                println!(" - Added {workspace}");
                kept_by_toolchain
                    .entry(entry.toolchain.clone())
                    .or_default()
                    .push(workspace.clone());
                // Non-JS packages participate in turbo.json task pruning,
                // but not in the JS lockfile subgraph or package.json
                // workspaces.
                workspace_names.push(workspace);
                continue;
            }
            prune.copy_workspace(
                entry.package_json_path(),
                &entry.package_json,
                &excluded_dev_workspaces,
            )?;
            let parent = entry
                .package_json_path()
                .parent()
                .expect("workspace package.json path should have a parent");
            workspace_paths.push(parent.to_unix().to_string());

            println!(" - Added {workspace}");
            workspace_names.push(workspace);
        }
    }

    // Each toolchain contributes whatever the pruned repository needs
    // beyond the packages themselves: extra members it requires, rewritten
    // workspace files, and config files to carry over.
    for toolchain in prune.package_graph.toolchains().iter() {
        let toolchain_id = toolchain.id();
        let kept = kept_by_toolchain.remove(&toolchain_id).unwrap_or_default();
        let Some(plan) = toolchain.prune_plan(&kept)? else {
            continue;
        };
        planned_toolchains.insert(toolchain_id);
        for extra in plan.extra_packages {
            let name = PackageName::Other(extra.clone());
            let info = prune
                .package_graph
                .package_info(&name)
                .ok_or_else(|| Error::MissingWorkspace(name.clone()))?;
            prune.copy_package_dir(info.package_json_path())?;
            println!(" - Added {extra} (required by kept packages)");
            workspace_names.push(extra);
        }
        for (path, contents) in plan.root_files {
            let rel = RelativeUnixPath::new(&path)?.to_anchored_system_path_buf();
            let full_path = prune.full_directory.resolve(&rel);
            full_path.ensure_dir()?;
            full_path.create_with_contents(&contents)?;
            if prune.docker {
                let docker_path = prune.docker_directory().resolve(&rel);
                docker_path.ensure_dir()?;
                docker_path.create_with_contents(&contents)?;
            }
        }
        for path in plan.copy_paths {
            let rel = RelativeUnixPath::new(&path)?.to_anchored_system_path_buf();
            prune.copy_file(&rel, Some(CopyDestination::Docker))?;
        }
    }
    prune.copy_file_dependencies(&workspace_names)?;

    trace!("new workspaces: {}", workspace_paths.join(", "));
    trace!("lockfile keys: {}", lockfile_keys.join(", "));

    // Files carried into every pruned repository regardless of toolchain.
    // These do not depend on the JavaScript lockfile subgraph, so they run
    // for a pure Cargo workspace too.
    for (relative_path, required_for_install) in ADDITIONAL_FILES.as_slice() {
        let path = relative_path.to_anchored_system_path_buf();
        prune.copy_file(&path, *required_for_install)?;
    }

    for (relative_path, required_for_install) in ADDITIONAL_DIRECTORIES.as_slice() {
        let path = relative_path.to_anchored_system_path_buf();
        prune.copy_directory(&path, *required_for_install)?;
    }

    prune.copy_turbo_json(&workspace_names)?;
    prune.copy_global_dependencies()?;

    // The JavaScript lockfile subgraph, root package.json rewrite, and pnpm
    // workspace patch pruning apply only when the repository has a JavaScript
    // package manager and root manifest. A pure Cargo workspace has neither;
    // its Cargo.lock and Cargo.toml were already rewritten by the Cargo
    // toolchain's prune plan above.
    if let (Some(package_manager), Some(root_package_json)) = (
        prune.package_graph.package_manager(),
        prune.package_graph.root_package_json(),
    ) {
        let lockfile = prune
            .package_graph
            .lockfile()
            .ok_or(Error::MissingLockfile)?
            .subgraph(&workspace_paths, &lockfile_keys)?;

        let lockfile_name = package_manager.lockfile_name();

        if prune.uses_per_workspace_lockfiles {
            // Per-workspace lockfiles are already in the pruned output from
            // recursive_copy in copy_workspace. Copy the original root lockfile
            // as-is (it only contains root-level dependencies).
            let original_root_lockfile = prune.root.join_component(lockfile_name);
            let out_lockfile = prune.out_directory.join_component(lockfile_name);
            turborepo_fs::copy_file(&original_root_lockfile, &out_lockfile)?;
            if prune.docker {
                turborepo_fs::copy_file(
                    &original_root_lockfile,
                    prune.docker_directory().join_component(lockfile_name),
                )?;
            }
        } else {
            let lockfile_contents = lockfile.encode()?;
            let lockfile_path = prune.out_directory.join_component(lockfile_name);
            lockfile_path.create_with_contents(&lockfile_contents)?;
            if prune.docker {
                prune
                    .docker_directory()
                    .join_component(lockfile_name)
                    .create_with_contents(&lockfile_contents)?;
            }
        }

        let original_lockfile = prune
            .package_graph
            .lockfile()
            .ok_or(Error::MissingLockfile)?;
        let original_patches = collect_patch_paths(
            original_lockfile,
            root_package_json,
            &prune.root,
            package_manager,
        )?;
        let pruned_patches = if original_patches.is_empty() {
            Vec::new()
        } else {
            collect_patch_paths(
                lockfile.as_ref(),
                root_package_json,
                &prune.root,
                package_manager,
            )?
        };

        if !original_patches.is_empty() {
            trace!(
                "original patches: {:?}, pruned patches: {:?}",
                original_patches,
                pruned_patches
            );
        }

        let original_contents = prune.root.resolve(package_json()).read_to_string()?;
        let original_value: serde_json::Value = serde_json::from_str(&original_contents)?;
        if !original_patches.is_empty()
            || original_value.get("workspaces").is_some()
            || !excluded_dev_workspaces.is_empty()
        {
            let pruned_json = if original_patches.is_empty() {
                root_package_json.clone()
            } else {
                package_manager.prune_patched_packages(
                    root_package_json,
                    &pruned_patches,
                    &prune.root,
                )
            };

            let mut pruned_value = serde_json::to_value(&pruned_json)?;
            prune_package_json_workspaces(&mut pruned_value, &workspace_paths);
            prune_package_json_dev_dependencies(&mut pruned_value, &excluded_dev_workspaces);
            // Merge into the original JSON value so package.json key order stays stable.
            let merged = merge_preserving_key_order(&original_value, &pruned_value);
            let mut pruned_json_contents = serde_json::to_string_pretty(&merged)?;
            // Add trailing newline to match Go behavior
            pruned_json_contents.push('\n');

            let original = prune.root.resolve(package_json());
            let permissions = original.symlink_metadata()?.permissions();
            let new_package_json_path = prune.full_directory.resolve(package_json());
            new_package_json_path.create_with_contents(&pruned_json_contents)?;
            #[cfg(unix)]
            new_package_json_path.set_mode(permissions.mode())?;
            #[cfg(windows)]
            if permissions.readonly() {
                new_package_json_path.set_readonly()?
            }
            if prune.docker {
                turborepo_fs::copy_file(
                    new_package_json_path,
                    prune.docker_directory().resolve(package_json()),
                )?;
            }
        } else {
            prune.copy_file(package_json(), Some(CopyDestination::Docker))?;
        }

        if !original_patches.is_empty() {
            for patch in &pruned_patches {
                prune.copy_patch_file(patch)?;
            }
        }

        // Prune pnpm-workspace.yaml's patchedDependencies so it only
        // references patches that are actually in the pruned output.
        if package_manager.is_pnpm_family() {
            let ws_config =
                turborepo_repository::package_manager::pnpm::WORKSPACE_CONFIGURATION_PATH;
            let ws_path = AnchoredSystemPathBuf::from_raw(ws_config)?;
            let out_ws = prune.out_directory.resolve(&ws_path);
            turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                &out_ws,
                &pruned_patches,
            )?;
            let full_ws = prune.full_directory.resolve(&ws_path);
            turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                &full_ws,
                &pruned_patches,
            )?;
            if prune.docker {
                let docker_ws = prune.docker_directory().resolve(&ws_path);
                turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                    &docker_ws,
                    &pruned_patches,
                )?;
            }
        }
    }

    // The pruned output is complete; let each toolchain polish its own
    // files in place (e.g. Cargo canonicalizes the pruned lockfile).
    for toolchain in prune.package_graph.toolchains().iter() {
        if !planned_toolchains.contains(&toolchain.id()) {
            continue;
        }
        let finalized_files = toolchain.prune_finalize(&prune.full_directory);
        if prune.docker {
            sync_prune_finalize_files(
                &prune.full_directory,
                &prune.docker_directory(),
                finalized_files,
            );
        }
    }

    Ok(())
}

fn finalized_path_is_contained(root: &AbsoluteSystemPath, path: &AbsoluteSystemPath) -> bool {
    if !root.contains(path) {
        return false;
    }

    let Ok(root_realpath) = root.to_realpath() else {
        return false;
    };
    match path.symlink_metadata() {
        Ok(_) => {
            return path
                .to_realpath()
                .is_ok_and(|realpath| root_realpath.contains(&realpath));
        }
        Err(error) if !error.is_io_error(std::io::ErrorKind::NotFound) => {
            return false;
        }
        Err(_) => {}
    }

    for ancestor in path.ancestors().skip(1) {
        match ancestor.try_exists() {
            Ok(true) => {
                return ancestor
                    .to_realpath()
                    .is_ok_and(|realpath| root_realpath.contains(&realpath));
            }
            Ok(false) => {}
            Err(_) => return false,
        }
    }
    false
}

fn sync_prune_finalize_files(
    source_root: &AbsoluteSystemPath,
    destination_root: &AbsoluteSystemPath,
    files: Vec<String>,
) {
    for path in files {
        let Ok(relative_path) = RelativeUnixPath::new(&path) else {
            warn!("unable to synchronize invalid finalized prune path {path:?}");
            continue;
        };
        let relative_path = relative_path.to_anchored_system_path_buf();
        let source = source_root.resolve(&relative_path);
        let destination = destination_root.resolve(&relative_path);
        let source_is_regular_file = source
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file());
        if !source_is_regular_file
            || !finalized_path_is_contained(source_root, &source)
            || !finalized_path_is_contained(destination_root, &destination)
        {
            warn!("unable to synchronize unsafe finalized prune path: {path:?}");
            continue;
        }

        if let Err(error) = turborepo_fs::copy_file(&source, destination) {
            warn!("unable to synchronize finalized prune file {path:?}: {error}");
        }
    }
}

fn workspace_dependency_target<'a>(name: &'a str, version: &'a str) -> Option<&'a str> {
    let specifier = version.strip_prefix("workspace:")?;
    match specifier.rsplit_once('@') {
        Some((target, "*" | "^" | "~")) if !target.is_empty() => Some(target),
        _ => Some(name),
    }
}

fn prune_package_json_dev_dependencies(
    package_json: &mut serde_json::Value,
    excluded_workspaces: &HashSet<String>,
) -> bool {
    let Some(dev_dependencies) = package_json
        .get_mut("devDependencies")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };

    let original_len = dev_dependencies.len();
    dev_dependencies.retain(|name, version| {
        let Some(version) = version.as_str() else {
            return true;
        };
        workspace_dependency_target(name, version)
            .is_none_or(|target| !excluded_workspaces.contains(target))
    });
    let changed = dev_dependencies.len() != original_len;
    let remove_dev_dependencies = dev_dependencies.is_empty();
    if remove_dev_dependencies {
        if let Some(package_json) = package_json.as_object_mut() {
            package_json.remove("devDependencies");
        }
    }
    changed
}

fn prune_package_json_workspaces(package_json: &mut serde_json::Value, workspace_paths: &[String]) {
    let Some(workspaces) = package_json.get_mut("workspaces") else {
        return;
    };

    let pruned_workspaces = || {
        workspace_paths
            .iter()
            .map(|workspace| serde_json::Value::String(workspace.clone()))
            .collect::<Vec<_>>()
    };

    match workspaces {
        serde_json::Value::Array(packages) => *packages = pruned_workspaces(),
        serde_json::Value::Object(config) => {
            if let Some(packages) = config.get_mut("packages") {
                *packages = serde_json::Value::Array(pruned_workspaces());
            }
        }
        _ => {}
    }
}

fn collect_patch_paths(
    lockfile: &dyn turborepo_lockfiles::Lockfile,
    root_package_json: &PackageJson,
    repo_root: &turbopath::AbsoluteSystemPath,
    package_manager: &PackageManager,
) -> Result<Vec<RelativeUnixPathBuf>, Error> {
    let mut patches = lockfile.patches()?;
    let patch_keys = lockfile.patch_keys();

    if !patch_keys.is_empty() {
        patches.extend(package_json_patch_paths(root_package_json, &patch_keys));

        if package_manager.is_pnpm_family() {
            let workspace_yaml_path = repo_root.join_component(
                turborepo_repository::package_manager::pnpm::WORKSPACE_CONFIGURATION_PATH,
            );
            patches.extend(
                turborepo_repository::package_manager::pnpm::patch_paths_for_keys(
                    &workspace_yaml_path,
                    &patch_keys,
                )?,
            );
        }
    }

    patches.sort();
    patches.dedup();
    validate_patch_source_paths(repo_root, &patches)?;
    Ok(patches)
}

fn validate_patch_source_paths(
    repo_root: &AbsoluteSystemPath,
    patches: &[RelativeUnixPathBuf],
) -> Result<(), Error> {
    let repo_root_realpath = repo_root.to_realpath()?;

    for patch in patches {
        let patch_path = repo_root.join_unix_path(patch);
        if !patch_path.starts_with(repo_root.as_std_path()) {
            return Err(Error::InvalidPatchPath(patch.clone()));
        }

        if patch_path.try_exists()? {
            let patch_realpath = patch_path.to_realpath()?;
            if !patch_realpath.starts_with(repo_root_realpath.as_std_path()) {
                return Err(Error::InvalidPatchPath(patch.clone()));
            }
        }
    }

    Ok(())
}

fn package_json_patch_paths(
    package_json: &PackageJson,
    patch_keys: &[String],
) -> Vec<RelativeUnixPathBuf> {
    let patch_keys: BTreeSet<_> = patch_keys.iter().map(String::as_str).collect();
    let mut patches = Vec::new();

    if let Some(patched_dependencies) = package_json.patched_dependencies.as_ref() {
        patches.extend(
            patched_dependencies.iter().filter_map(|(key, path)| {
                patch_keys.contains(key.as_str()).then_some(path.clone())
            }),
        );
    }

    if let Some(patched_dependencies) = package_json
        .pnpm
        .as_ref()
        .and_then(|config| config.patched_dependencies.as_ref())
    {
        patches.extend(
            patched_dependencies.iter().filter_map(|(key, path)| {
                patch_keys.contains(key.as_str()).then_some(path.clone())
            }),
        );
    }

    patches
}

struct Prune<'a> {
    package_graph: PackageGraph,
    root: AbsoluteSystemPathBuf,
    out_directory: AbsoluteSystemPathBuf,
    full_directory: AbsoluteSystemPathBuf,
    docker: bool,
    production: bool,
    scope: &'a [String],
    use_gitignore: bool,
    uses_per_workspace_lockfiles: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum CopyDestination {
    // Copies to full and json
    Docker,
    // Copies to out, full, and json
    // This behavior comes from a bug in the Go impl that people depend on.
    All,
}

impl<'a> Prune<'a> {
    async fn new(
        base: &CommandBase,
        scope: &'a [String],
        docker: bool,
        production: bool,
        output_dir: &str,
        use_gitignore: bool,
        telemetry: CommandEventBuilder,
    ) -> Result<Self, Error> {
        let allow_missing_package_manager = base.opts().repo_opts.allow_no_package_manager;
        telemetry.track_arg_usage(
            "dangerously-allow-missing-package-manager",
            allow_missing_package_manager,
        );

        if scope.is_empty() {
            return Err(Error::NoWorkspaceSpecified);
        }

        let root_package_json_path = base.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(&root_package_json_path)?;

        let mut graph_builder = PackageGraph::builder(&base.repo_root, root_package_json)
            .with_allow_no_package_manager(allow_missing_package_manager);
        for adapter in crate::run::builder::configured_ecosystem_adapters(
            &base.repo_root,
            crate::run::builder::cargo_enabled(&base.opts().future_flags),
        ) {
            graph_builder = graph_builder.with_ecosystem_adapter(adapter);
        }
        let package_graph = graph_builder.build().await?;

        let out_directory = AbsoluteSystemPathBuf::from_unknown(&base.repo_root, output_dir);

        let full_directory = match docker {
            true => out_directory.join_component("full"),
            false => out_directory.clone(),
        };

        trace!("scope: {}", scope.join(", "));
        trace!("docker: {}", docker);
        trace!("production: {}", production);
        trace!("out directory: {}", &out_directory);

        for target in scope {
            let workspace = PackageName::Other(target.clone());
            let Some(info) = package_graph.package_info(&workspace) else {
                return Err(Error::MissingWorkspace(workspace));
            };
            // A package anchored at the repository root (e.g. the synthetic
            // Cargo workspace package) has no directory of its own; pruning
            // it would mean copying the whole repository.
            if info.package_path().components().next().is_none() {
                return Err(Error::PackageNotPruneable(target.clone()));
            }
            trace!(
                "target: {}",
                info.package_json
                    .name
                    .as_ref()
                    .map(|name| name.as_str())
                    .unwrap_or_default()
            );
            trace!("workspace package.json: {}", &info.package_json_path);
            trace!(
                "external dependencies: {:?}",
                &info.unresolved_external_dependencies
            );
        }

        // A JavaScript project must have a lockfile to subgraph. A pure Cargo
        // workspace has no JavaScript package manager and no JS lockfile; its
        // Cargo.lock is pruned by the Cargo toolchain's prune plan.
        if package_graph.package_manager().is_some() && package_graph.lockfile().is_none() {
            return Err(Error::MissingLockfile);
        }

        let uses_per_workspace_lockfiles = package_graph
            .package_manager()
            .is_some_and(|pm| pm.is_pnpm_family())
            && NpmRc::from_file(&base.repo_root)
                .unwrap_or_default()
                .shared_workspace_lockfile
                == Some(false);

        full_directory.resolve(package_json()).ensure_dir()?;
        if docker {
            out_directory
                .join_component("json")
                .resolve(package_json())
                .ensure_dir()?;
        }

        Ok(Self {
            package_graph,
            root: base.repo_root.clone(),
            out_directory,
            full_directory,
            docker,
            production,
            scope,
            use_gitignore,
            uses_per_workspace_lockfiles,
        })
    }

    fn docker_directory(&self) -> AbsoluteSystemPathBuf {
        self.out_directory.join_component("json")
    }

    fn copy_file(
        &self,
        path: &AnchoredSystemPath,
        destination: Option<CopyDestination>,
    ) -> Result<(), Error> {
        let from_path = self.root.resolve(path);
        if !from_path.try_exists()? {
            trace!("{from_path} doesn't exist, skipping copying");
            return Ok(());
        }
        let full_to = self.full_directory.resolve(path);
        turborepo_fs::copy_file(&from_path, full_to)?;
        if matches!(destination, Some(CopyDestination::All)) {
            let out_to = self.out_directory.resolve(path);
            turborepo_fs::copy_file(&from_path, out_to)?;
        }
        if self.docker
            && matches!(
                destination,
                Some(CopyDestination::Docker) | Some(CopyDestination::All)
            )
        {
            let docker_to = self.docker_directory().resolve(path);
            turborepo_fs::copy_file(&from_path, docker_to)?;
        }
        Ok(())
    }

    fn copy_patch_file(&self, patch: &RelativeUnixPathBuf) -> Result<(), Error> {
        self.validate_patch_destination_path(patch, &self.full_directory)?;
        if self.docker {
            self.validate_patch_destination_path(patch, &self.docker_directory())?;
        }

        self.copy_file(
            &patch.to_anchored_system_path_buf(),
            Some(CopyDestination::Docker),
        )
    }

    fn validate_patch_destination_path(
        &self,
        patch: &RelativeUnixPathBuf,
        destination_root: &AbsoluteSystemPath,
    ) -> Result<(), Error> {
        let destination_root_realpath = destination_root.to_realpath()?;
        let patch_path = destination_root.join_unix_path(patch);

        if !patch_path.starts_with(destination_root.as_std_path()) {
            return Err(Error::InvalidPatchPath(patch.clone()));
        }

        if patch_path.symlink_metadata().is_ok() {
            let patch_realpath = patch_path.to_realpath()?;
            if !patch_realpath.starts_with(destination_root_realpath.as_std_path()) {
                return Err(Error::InvalidPatchPath(patch.clone()));
            }
        }

        for ancestor in patch_path.ancestors().skip(1) {
            if ancestor.try_exists()? {
                let ancestor_realpath = ancestor.to_realpath()?;
                if !ancestor_realpath.starts_with(destination_root_realpath.as_std_path()) {
                    return Err(Error::InvalidPatchPath(patch.clone()));
                }
                break;
            }
        }

        Ok(())
    }

    fn copy_directory(
        &self,
        path: &AnchoredSystemPath,
        destination: Option<CopyDestination>,
    ) -> Result<(), Error> {
        let from_path = self.root.resolve(path);
        if !from_path.try_exists()? {
            trace!("{from_path} doesn't exist, skipping copying");
            return Ok(());
        }
        let full_to = self.full_directory.resolve(path);
        turborepo_fs::recursive_copy(&from_path, full_to, self.use_gitignore, Some(&self.root))?;
        if matches!(destination, Some(CopyDestination::All)) {
            let out_to = self.out_directory.resolve(path);
            turborepo_fs::recursive_copy(&from_path, out_to, self.use_gitignore, Some(&self.root))?;
        }
        if self.docker
            && matches!(
                destination,
                Some(CopyDestination::Docker) | Some(CopyDestination::All)
            )
        {
            let docker_to = self.docker_directory().resolve(path);
            turborepo_fs::recursive_copy(
                &from_path,
                docker_to,
                self.use_gitignore,
                Some(&self.root),
            )?;
        }
        Ok(())
    }

    /// Copy a non-JavaScript package's directory into the pruned output.
    /// Mirrors [`Self::copy_workspace`], except the docker "json" layer
    /// receives the package's actual manifest (e.g. `Cargo.toml`) rather
    /// than a `package.json`, and there are no npm bin stubs to create.
    fn copy_package_dir(&self, manifest_path: &AnchoredSystemPath) -> Result<(), Error> {
        let abs_manifest_path = self.root.resolve(manifest_path);
        let original_dir = abs_manifest_path
            .parent()
            .ok_or_else(|| Error::WorkspaceAtFilesystemRoot)?;
        let metadata = original_dir.symlink_metadata()?;
        let relative_package_dir = AnchoredSystemPathBuf::new(&self.root, original_dir)?;
        let target_dir = self.full_directory.resolve(&relative_package_dir);
        target_dir.create_dir_all_with_permissions(metadata.permissions())?;

        turborepo_fs::recursive_copy(
            original_dir,
            &target_dir,
            self.use_gitignore,
            Some(&self.root),
        )?;

        if self.docker {
            let docker_package_dir = self.docker_directory().resolve(&relative_package_dir);
            docker_package_dir.ensure_dir()?;
            if let Some(manifest_name) = abs_manifest_path.file_name() {
                turborepo_fs::copy_file(
                    &abs_manifest_path,
                    docker_package_dir.join_component(manifest_name),
                )?;
            }
        }

        Ok(())
    }

    fn copy_workspace(
        &self,
        package_json_path: &AnchoredSystemPath,
        workspace_package_json: &PackageJson,
        excluded_dev_workspaces: &HashSet<String>,
    ) -> Result<(), Error> {
        let package_json_path = self.root.resolve(package_json_path);
        let pruned_package_json = if excluded_dev_workspaces.is_empty() {
            None
        } else {
            let mut value: serde_json::Value =
                serde_json::from_str(&package_json_path.read_to_string()?)?;
            if prune_package_json_dev_dependencies(&mut value, excluded_dev_workspaces) {
                let mut contents = serde_json::to_string_pretty(&value)?;
                contents.push('\n');
                Some(contents)
            } else {
                None
            }
        };
        let original_dir = package_json_path
            .parent()
            .ok_or_else(|| Error::WorkspaceAtFilesystemRoot)?;
        let metadata = original_dir.symlink_metadata()?;
        let relative_workspace_dir = AnchoredSystemPathBuf::new(&self.root, original_dir)?;
        let target_dir = self.full_directory.resolve(&relative_workspace_dir);
        target_dir.create_dir_all_with_permissions(metadata.permissions())?;

        turborepo_fs::recursive_copy(
            original_dir,
            &target_dir,
            self.use_gitignore,
            Some(&self.root),
        )?;
        if let Some(contents) = &pruned_package_json {
            target_dir
                .resolve(package_json())
                .create_with_contents(contents)?;
        }

        if self.docker {
            let docker_workspace_dir = self.docker_directory().resolve(&relative_workspace_dir);
            docker_workspace_dir.ensure_dir()?;
            let docker_package_json = docker_workspace_dir.resolve(package_json());
            if let Some(contents) = &pruned_package_json {
                docker_package_json.ensure_dir()?;
                docker_package_json.create_with_contents(contents)?;
            } else {
                turborepo_fs::copy_file(&package_json_path, docker_package_json)?;
            }
            self.create_docker_bin_stubs(
                workspace_package_json,
                original_dir,
                &docker_workspace_dir,
            )?;

            // Per-workspace lockfiles are a pnpm feature, so a package manager
            // is always present here.
            if let Some(package_manager) = self
                .package_graph
                .package_manager()
                .filter(|_| self.uses_per_workspace_lockfiles)
            {
                let lockfile_name = package_manager.lockfile_name();
                let ws_lockfile = original_dir.join_component(lockfile_name);
                if ws_lockfile.try_exists()? {
                    turborepo_fs::copy_file(
                        &ws_lockfile,
                        docker_workspace_dir.join_component(lockfile_name),
                    )?;
                }
            }
        }

        Ok(())
    }

    fn create_docker_bin_stubs(
        &self,
        package_json: &PackageJson,
        original_dir: &turbopath::AbsoluteSystemPath,
        docker_workspace_dir: &turbopath::AbsoluteSystemPath,
    ) -> Result<(), Error> {
        for bin_path in bin_paths(package_json) {
            let Ok(relative_bin_path) = RelativeUnixPathBuf::new(bin_path) else {
                trace!("bin entry {bin_path} is not relative, skipping stub");
                continue;
            };

            let original_bin_path = original_dir.join_unix_path(&relative_bin_path);
            if !original_bin_path.starts_with(original_dir.as_std_path()) {
                trace!("bin entry {bin_path} escapes workspace, skipping stub");
                continue;
            }

            let docker_bin_path = docker_workspace_dir.join_unix_path(relative_bin_path);
            if !docker_bin_path.try_exists()? {
                docker_bin_path.ensure_dir()?;
                docker_bin_path.create_with_contents("")?;
            }
        }

        Ok(())
    }

    /// Copy directories for `file:` protocol dependencies into the pruned
    /// output. These are local packages referenced by path that aren't
    /// workspaces, so they wouldn't otherwise be included.
    fn copy_file_dependencies(&self, workspace_names: &[String]) -> Result<(), Error> {
        let all_workspaces = std::iter::once(PackageName::Root).chain(
            workspace_names
                .iter()
                .map(|name| PackageName::Other(name.clone())),
        );

        for workspace in all_workspaces {
            let Some(info) = self.package_graph.package_info(&workspace) else {
                continue;
            };

            let workspace_abs_dir = self.root.resolve(info.package_path());

            for (_dep_name, dep_version) in info.package_json.all_dependencies() {
                let Some(path_str) = dep_version.strip_prefix("file:") else {
                    continue;
                };

                let Ok(relative_path) = RelativeUnixPathBuf::new(path_str) else {
                    continue;
                };

                // Resolve the file: path relative to the workspace directory.
                // join_unix_path normalizes the result (resolves ..)
                let abs_dep_path = workspace_abs_dir.join_unix_path(relative_path);

                // Skip if the path is outside the repo root
                let Ok(anchored) = AnchoredSystemPathBuf::new(&self.root, &abs_dep_path) else {
                    trace!(
                        "file: dependency {path_str} from {workspace} is outside repo root, \
                         skipping"
                    );
                    continue;
                };

                if !abs_dep_path.try_exists()? {
                    trace!("file: dependency {path_str} from {workspace} doesn't exist, skipping");
                    continue;
                }

                trace!(
                    "Copying file: dependency {path_str} from {workspace} -> {}",
                    anchored
                );

                self.copy_directory(&anchored, Some(CopyDestination::Docker))?;
            }
        }

        Ok(())
    }

    fn workspace_transitive_closure<'graph, 'node, I: IntoIterator<Item = &'node PackageNode>>(
        &'graph self,
        nodes: I,
    ) -> HashSet<&'graph PackageNode> {
        if self.production {
            self.package_graph.production_transitive_closure(nodes)
        } else {
            self.package_graph.transitive_closure(nodes)
        }
    }

    fn internal_dependencies(&self) -> Vec<PackageName> {
        let workspaces = std::iter::once(PackageNode::Workspace(PackageName::Root))
            .chain(
                self.scope
                    .iter()
                    .map(|workspace| PackageNode::Workspace(PackageName::Other(workspace.clone()))),
            )
            .collect::<Vec<_>>();
        let mut names = self
            .workspace_transitive_closure(workspaces.iter())
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Root => None,
                PackageNode::Workspace(workspace) => Some(workspace.clone()),
            })
            .collect::<HashSet<_>>();

        loop {
            let mut changed = false;
            for workspace in names.clone() {
                let Some(info) = self.package_graph.package_info(&workspace) else {
                    continue;
                };
                for (peer_name, _) in info.package_json.peer_dependencies.iter().flatten() {
                    if info.package_json.is_optional_peer_dependency(peer_name) {
                        continue;
                    }

                    let peer = PackageName::from(peer_name.as_str());
                    if self.package_graph.package_info(&peer).is_some() && names.insert(peer) {
                        changed = true;
                    }
                }
            }

            if !changed {
                break;
            }

            let workspace_nodes = names
                .iter()
                .cloned()
                .map(PackageNode::Workspace)
                .collect::<Vec<_>>();
            names.extend(
                self.workspace_transitive_closure(workspace_nodes.iter())
                    .into_iter()
                    .filter_map(|node| match node {
                        PackageNode::Root => None,
                        PackageNode::Workspace(workspace) => Some(workspace.clone()),
                    }),
            );
        }

        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort();
        names
    }

    fn lockfile_keys(&self, workspaces: &[PackageName]) -> Result<Vec<String>, Error> {
        let mut keys = self
            .package_graph
            .transitive_external_dependencies(workspaces.iter())
            .into_iter()
            .map(|pkg| pkg.key.clone())
            .collect::<HashSet<_>>();

        let lockfile = self
            .package_graph
            .lockfile()
            .ok_or(Error::MissingLockfile)?;

        for workspace in workspaces {
            let Some(info) = self.package_graph.package_info(workspace) else {
                continue;
            };

            let peer_dependencies = info
                .package_json
                .peer_dependencies
                .iter()
                .flatten()
                .filter(|(name, _)| !info.package_json.is_optional_peer_dependency(name))
                .filter(|(name, _)| {
                    self.package_graph
                        .package_info(&PackageName::from(name.as_str()))
                        .is_none()
                })
                .map(|(name, version)| (name.clone(), version.clone()))
                .collect::<BTreeMap<_, _>>();

            if peer_dependencies.is_empty() {
                continue;
            }

            let workspace_path = info.package_path().to_unix();
            keys.extend(
                turborepo_lockfiles::transitive_closure(
                    lockfile,
                    workspace_path.as_str(),
                    peer_dependencies,
                    false,
                )?
                .into_iter()
                .map(|pkg| pkg.key),
            );
        }

        let mut keys = keys.into_iter().collect::<Vec<_>>();
        keys.sort();
        Ok(keys)
    }

    /// Copy files matched by `globalDependencies` globs when the
    /// `pruneGlobalDependencies` future flag is enabled.
    fn copy_global_dependencies(&self) -> Result<(), Error> {
        let Some((mut turbo_json, _)) = self
            .get_turbo_json(turbo_json())
            .transpose()
            .or_else(|| self.get_turbo_json(turbo_jsonc()).transpose())
            .transpose()?
        else {
            return Ok(());
        };

        let prune_enabled = turbo_json
            .future_flags
            .as_ref()
            .map(|ff| ff.value.prune_includes_global_files)
            .unwrap_or(false);
        if !prune_enabled {
            return Ok(());
        }

        turbo_json.resolve_global_config();

        let Some(global_deps) = &turbo_json.global_dependencies else {
            return Ok(());
        };

        let global_dep_globs: Vec<&str> = global_deps.iter().map(|s| s.value.as_ref()).collect();
        if global_dep_globs.is_empty() {
            return Ok(());
        }

        let (raw_inclusions, raw_exclusions): (Vec<&str>, Vec<&str>) =
            global_dep_globs.iter().partition(|g| !g.starts_with('!'));

        let inclusions = raw_inclusions
            .iter()
            .map(|i| ValidatedGlob::from_str(i))
            .collect::<Result<Vec<_>, _>>()?;
        let exclusions = raw_exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e.strip_prefix('!').unwrap_or(e)))
            .collect::<Result<Vec<_>, _>>()?;

        let matched_files =
            globwalk::globwalk(&self.root, &inclusions, &exclusions, WalkType::Files)?;

        for file_path in &matched_files {
            let anchored = AnchoredSystemPathBuf::new(&self.root, file_path)?;
            // turbo.json is already written by copy_turbo_json as a pruned
            // version. Don't overwrite it with the original.
            if anchored.as_str() == CONFIG_FILE || anchored.as_str() == CONFIG_FILE_JSONC {
                continue;
            }
            trace!("Copying global dependency: {}", anchored);
            self.copy_file(&anchored, Some(CopyDestination::Docker))?;
        }

        Ok(())
    }

    fn copy_turbo_json(&self, workspaces: &[String]) -> Result<(), Error> {
        let Some((turbo_json, turbo_json_name)) = self
            .get_turbo_json(turbo_json())
            .transpose()
            .or_else(|| self.get_turbo_json(turbo_jsonc()).transpose())
            .transpose()?
        else {
            return Ok(());
        };

        let pruned_turbo_json = turbo_json.prune_tasks(workspaces);
        let new_turbo_path = self.full_directory.resolve(turbo_json_name);
        new_turbo_path.create_with_contents(serde_json::to_string_pretty(&pruned_turbo_json)?)?;

        Ok(())
    }

    fn get_turbo_json<'b>(
        &self,
        turbo_json_name: &'b AnchoredSystemPath,
    ) -> Result<Option<(RawTurboJson, &'b AnchoredSystemPath)>, Error> {
        let original_turbo_path = self.root.resolve(turbo_json_name);
        let Some(turbo_json_contents) = original_turbo_path.read_existing_to_string()? else {
            return Ok(None);
        };

        let turbo_json =
            RawRootTurboJson::parse(&turbo_json_contents, turbo_json_name.as_str())?.try_into()?;
        Ok(Some((turbo_json, turbo_json_name)))
    }
}

fn bin_paths(package_json: &PackageJson) -> Vec<&str> {
    match package_json.other.get("bin") {
        Some(serde_json::Value::String(path)) => vec![path.as_str()],
        Some(serde_json::Value::Object(entries)) => entries
            .values()
            .filter_map(serde_json::Value::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

/// Merge `pruned` values into `original`, preserving the key ordering from
/// `original`. Keys present in `original` but absent from `pruned` are dropped.
/// Keys present in `pruned` but absent from `original` are appended.
fn merge_preserving_key_order(
    original: &serde_json::Value,
    pruned: &serde_json::Value,
) -> serde_json::Value {
    match (original, pruned) {
        (serde_json::Value::Object(orig_map), serde_json::Value::Object(pruned_map)) => {
            let mut result = serde_json::Map::new();
            for (key, orig_val) in orig_map {
                if let Some(pruned_val) = pruned_map.get(key) {
                    result.insert(
                        key.clone(),
                        merge_preserving_key_order(orig_val, pruned_val),
                    );
                }
            }
            for (key, pruned_val) in pruned_map {
                if !orig_map.contains_key(key) {
                    result.insert(key.clone(), pruned_val.clone());
                }
            }
            serde_json::Value::Object(result)
        }
        (_, pruned) => pruned.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        fs,
    };

    use serde_json::json;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_graph::{PackageGraph, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };

    use super::{
        bin_paths, finalized_path_is_contained, merge_preserving_key_order,
        prune_package_json_workspaces, sync_prune_finalize_files, Prune, ADDITIONAL_FILES,
    };

    struct MockDiscovery;

    #[test]
    fn finalized_files_are_synchronized_nonfatally() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let full = root.join_component("full");
        let json = root.join_component("json");
        full.create_dir_all().unwrap();
        json.create_dir_all().unwrap();
        full.join_component("Cargo.lock")
            .create_with_contents("canonical")
            .unwrap();
        json.join_component("Cargo.lock")
            .create_with_contents("stale")
            .unwrap();

        sync_prune_finalize_files(&full, &json, vec!["missing".into(), "Cargo.lock".into()]);

        assert_eq!(
            json.join_component("Cargo.lock").read_to_string().unwrap(),
            "canonical"
        );
    }

    #[cfg(unix)]
    #[test]
    fn finalized_files_reject_path_escapes() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let full = root.join_component("full");
        let json = root.join_component("json");
        full.create_dir_all().unwrap();
        json.create_dir_all().unwrap();

        let traversal = full.join_component("..").join_component("outside.lock");
        assert!(!finalized_path_is_contained(&full, &traversal));

        let source_target = root.join_component("source-target.lock");
        fs::write(source_target.as_std_path(), "outside source").unwrap();
        let source_link = full.join_component("source-link.lock");
        std::os::unix::fs::symlink(source_target.as_std_path(), source_link.as_std_path()).unwrap();
        let source_copy = json.join_component("source-link.lock");
        fs::write(source_copy.as_std_path(), "stale").unwrap();

        let internal_source_target = full.join_component("internal-source-target.lock");
        fs::write(internal_source_target.as_std_path(), "inside source").unwrap();
        let internal_source_link = full.join_component("internal-source-link.lock");
        std::os::unix::fs::symlink(
            internal_source_target.as_std_path(),
            internal_source_link.as_std_path(),
        )
        .unwrap();
        let internal_source_copy = json.join_component("internal-source-link.lock");
        fs::write(internal_source_copy.as_std_path(), "stale").unwrap();

        let destination_target = root.join_component("destination-target.lock");
        fs::write(destination_target.as_std_path(), "outside destination").unwrap();
        full.join_component("destination-link.lock")
            .create_with_contents("canonical")
            .unwrap();
        let destination_link = json.join_component("destination-link.lock");
        std::os::unix::fs::symlink(
            destination_target.as_std_path(),
            destination_link.as_std_path(),
        )
        .unwrap();

        sync_prune_finalize_files(
            &full,
            &json,
            vec![
                "source-link.lock".into(),
                "internal-source-link.lock".into(),
                "destination-link.lock".into(),
            ],
        );

        assert_eq!(
            fs::read_to_string(source_target.as_std_path()).unwrap(),
            "outside source"
        );
        assert_eq!(
            fs::read_to_string(destination_target.as_std_path()).unwrap(),
            "outside destination"
        );
        assert_eq!(
            fs::read_to_string(source_copy.as_std_path()).unwrap(),
            "stale"
        );
        assert_eq!(
            fs::read_to_string(internal_source_copy.as_std_path()).unwrap(),
            "stale"
        );
    }

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            self.discover_packages().await
        }
    }

    #[test]
    fn bin_paths_reads_string_bin() {
        let package_json = PackageJson::from_value(json!({
            "name": "bin-package",
            "bin": "cli.js"
        }))
        .unwrap();

        assert_eq!(bin_paths(&package_json), vec!["cli.js"]);
    }

    #[test]
    fn bin_paths_reads_object_bin() {
        let package_json = PackageJson::from_value(json!({
            "name": "bin-package",
            "bin": {
                "one": "bin/one.js",
                "two": "bin/two.js"
            }
        }))
        .unwrap();

        assert_eq!(bin_paths(&package_json), vec!["bin/one.js", "bin/two.js"]);
    }

    #[test]
    fn merge_preserves_key_order() {
        let original: serde_json::Value = serde_json::from_str(
            r#"{"z_last": 1, "a_first": 2, "m_middle": {"nested_z": true, "nested_a": false}}"#,
        )
        .unwrap();
        let pruned =
            json!({"a_first": 2, "m_middle": {"nested_a": false, "nested_z": true}, "z_last": 1});

        let merged = merge_preserving_key_order(&original, &pruned);
        let keys: Vec<_> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["z_last", "a_first", "m_middle"]);

        let nested_keys: Vec<_> = merged["m_middle"].as_object().unwrap().keys().collect();
        assert_eq!(nested_keys, vec!["nested_z", "nested_a"]);
    }

    #[test]
    fn merge_drops_removed_keys() {
        let original: serde_json::Value =
            serde_json::from_str(r#"{"keep": 1, "drop": 2, "also_keep": 3}"#).unwrap();
        let pruned = json!({"keep": 1, "also_keep": 3});

        let merged = merge_preserving_key_order(&original, &pruned);
        let keys: Vec<_> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["keep", "also_keep"]);
    }

    #[test]
    fn merge_appends_new_keys() {
        let original: serde_json::Value = serde_json::from_str(r#"{"existing": 1}"#).unwrap();
        let pruned = json!({"existing": 1, "new_key": 2});

        let merged = merge_preserving_key_order(&original, &pruned);
        let keys: Vec<_> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["existing", "new_key"]);
    }

    #[test]
    fn prune_workspaces_replaces_top_level_workspace_list() {
        let mut package_json = json!({
            "name": "repo",
            "workspaces": ["app", "scripts", "packages/*"]
        });

        prune_package_json_workspaces(&mut package_json, &["app".into(), "packages/ui".into()]);

        assert_eq!(package_json["workspaces"], json!(["app", "packages/ui"]));
    }

    #[test]
    fn prune_workspaces_preserves_nested_workspace_metadata() {
        let mut package_json = json!({
            "name": "repo",
            "workspaces": {
                "packages": ["app", "scripts", "packages/*"],
                "catalog": {
                    "react": "latest"
                }
            }
        });

        prune_package_json_workspaces(&mut package_json, &["app".into()]);

        assert_eq!(package_json["workspaces"]["packages"], json!(["app"]));
        assert_eq!(
            package_json["workspaces"]["catalog"],
            json!({"react": "latest"})
        );
    }

    #[tokio::test]
    async fn internal_dependencies_includes_all_cycle_members() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let package_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({
                "name": "repo",
                "packageManager": "npm@10.5.0"
            }))
            .unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(HashMap::from([
            (
                root.join_components(&["packages", "pkg-a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-a".to_string())),
                    dependencies: Some(BTreeMap::from([("pkg-b".to_string(), "*".to_string())])),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "pkg-b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-b".to_string())),
                    dependencies: Some(BTreeMap::from([("pkg-a".to_string(), "*".to_string())])),
                    ..Default::default()
                },
            ),
        ])))
        .build()
        .await
        .unwrap();
        let scope = vec!["pkg-a".to_string()];
        let out_directory = root.join_component("out");
        let prune = Prune {
            package_graph,
            root: root.clone(),
            out_directory: out_directory.clone(),
            full_directory: out_directory,
            docker: false,
            production: false,
            scope: &scope,
            use_gitignore: false,
            uses_per_workspace_lockfiles: false,
        };

        assert_eq!(
            prune.internal_dependencies(),
            vec![
                PackageName::Root,
                PackageName::from("pkg-a"),
                PackageName::from("pkg-b")
            ]
        );
    }

    #[tokio::test]
    async fn internal_dependencies_excludes_dev_dependencies_with_production() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let package_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({
                "name": "repo",
                "packageManager": "npm@10.5.0",
                "devDependencies": {
                    "root-tooling": "workspace:*"
                }
            }))
            .unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(HashMap::from([
            (
                root.join_components(&["apps", "web", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("web".to_string())),
                    dependencies: Some(BTreeMap::from([(
                        "lib".to_string(),
                        "workspace:*".to_string(),
                    )])),
                    dev_dependencies: Some(BTreeMap::from([(
                        "tooling".to_string(),
                        "workspace:*".to_string(),
                    )])),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "lib", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("lib".to_string())),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "tooling", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("tooling".to_string())),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "root-tooling", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("root-tooling".to_string())),
                    ..Default::default()
                },
            ),
        ])))
        .build()
        .await
        .unwrap();
        let scope = vec!["web".to_string()];
        let out_directory = root.join_component("out");
        let prune = Prune {
            package_graph,
            root: root.clone(),
            out_directory: out_directory.clone(),
            full_directory: out_directory,
            docker: false,
            production: true,
            scope: &scope,
            use_gitignore: false,
            uses_per_workspace_lockfiles: false,
        };

        assert_eq!(
            prune.internal_dependencies(),
            vec![
                PackageName::Root,
                PackageName::from("lib"),
                PackageName::from("web"),
            ]
        );
    }

    #[tokio::test]
    async fn internal_dependencies_includes_non_optional_peer_workspaces() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let package_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({
                "name": "repo",
                "packageManager": "npm@10.5.0"
            }))
            .unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(HashMap::from([
            (
                root.join_components(&["packages", "pkg-b", "package.json"]),
                PackageJson::from_value(json!({
                    "name": "pkg-b",
                    "peerDependencies": {
                        "pkg-c": "*",
                        "pkg-e": "*"
                    },
                    "peerDependenciesMeta": {
                        "pkg-e": { "optional": true }
                    }
                }))
                .unwrap(),
            ),
            (
                root.join_components(&["packages", "pkg-c", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-c".to_string())),
                    dependencies: Some(BTreeMap::from([("pkg-d".to_string(), "*".to_string())])),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "pkg-d", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-d".to_string())),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "pkg-e", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-e".to_string())),
                    ..Default::default()
                },
            ),
        ])))
        .build()
        .await
        .unwrap();
        let scope = vec!["pkg-b".to_string()];
        let out_directory = root.join_component("out");
        let prune = Prune {
            package_graph,
            root: root.clone(),
            out_directory: out_directory.clone(),
            full_directory: out_directory,
            docker: false,
            production: false,
            scope: &scope,
            use_gitignore: false,
            uses_per_workspace_lockfiles: false,
        };

        assert_eq!(
            prune.internal_dependencies(),
            vec![
                PackageName::Root,
                PackageName::from("pkg-b"),
                PackageName::from("pkg-c"),
                PackageName::from("pkg-d")
            ]
        );
    }

    #[tokio::test]
    async fn internal_dependencies_includes_non_optional_peer_workspaces_with_production() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let package_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({
                "name": "repo",
                "packageManager": "npm@10.5.0"
            }))
            .unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(HashMap::from([
            (
                root.join_components(&["packages", "pkg-b", "package.json"]),
                PackageJson::from_value(json!({
                    "name": "pkg-b",
                    "peerDependencies": {
                        "pkg-c": "*",
                        "pkg-e": "*"
                    },
                    "peerDependenciesMeta": {
                        "pkg-e": { "optional": true }
                    }
                }))
                .unwrap(),
            ),
            (
                root.join_components(&["packages", "pkg-c", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-c".to_string())),
                    dependencies: Some(BTreeMap::from([("pkg-d".to_string(), "*".to_string())])),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "pkg-d", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-d".to_string())),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["packages", "pkg-e", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("pkg-e".to_string())),
                    ..Default::default()
                },
            ),
        ])))
        .build()
        .await
        .unwrap();
        let scope = vec!["pkg-b".to_string()];
        let out_directory = root.join_component("out");
        let prune = Prune {
            package_graph,
            root: root.clone(),
            out_directory: out_directory.clone(),
            full_directory: out_directory,
            docker: false,
            production: true,
            scope: &scope,
            use_gitignore: false,
            uses_per_workspace_lockfiles: false,
        };

        assert_eq!(
            prune.internal_dependencies(),
            vec![
                PackageName::Root,
                PackageName::from("pkg-b"),
                PackageName::from("pkg-c"),
                PackageName::from("pkg-d")
            ]
        );
    }

    #[test]
    fn additional_files_snapshot() {
        let file_names: Vec<&str> = ADDITIONAL_FILES
            .iter()
            .map(|(path, _)| path.as_str())
            .collect();
        // Update this list when adding new entries to ADDITIONAL_FILES.
        assert_eq!(
            file_names,
            vec![
                ".gitattributes",
                ".gitignore",
                ".npmrc",
                ".yarnrc.yml",
                "bunfig.toml"
            ],
            "ADDITIONAL_FILES changed — update this snapshot and the prune integration tests \
             (prune_test.rs) accordingly"
        );
    }
}
