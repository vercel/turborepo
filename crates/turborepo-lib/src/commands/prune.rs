#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    str::FromStr,
    sync::{LazyLock, OnceLock},
};

use globwalk::{ValidatedGlob, WalkType};
use miette::Diagnostic;
use tracing::trace;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
    RelativeUnixPath, RelativeUnixPathBuf,
};
use turborepo_repository::{
    cargo,
    package_graph::{self, PackageGraph, PackageName, PackageNode, PackageToolchain},
    package_json::PackageJson,
    package_manager::{npmrc::NpmRc, PackageManager},
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
    #[error("Cannot prune a Cargo workspace without a Cargo.lock; run a build to generate it.")]
    MissingCargoLockfile,
    #[error(
        "'{0}' is the synthetic Cargo workspace package; prune a crate or an application package \
         instead."
    )]
    CargoWorkspacePackageNotPruneable(String),
    #[error(transparent)]
    Cargo(#[from] cargo::Error),
    #[error(transparent)]
    CargoLock(#[from] turborepo_lockfiles::CargoLockError),
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
    output_dir: &str,
    use_gitignore: bool,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_arg_usage("docker", docker);
    telemetry.track_arg_usage("out-dir", output_dir != DEFAULT_OUTPUT_DIR);

    let prune = Prune::new(base, scope, docker, output_dir, use_gitignore, telemetry).await?;

    println!(
        "Generating pruned monorepo for {} in {}",
        base.color_config.apply(BOLD.apply_to(scope.join(", "))),
        base.color_config.apply(BOLD.apply_to(&prune.out_directory)),
    );

    if let Some(workspace_config_path) = prune
        .package_graph
        .package_manager()
        .workspace_configuration_path()
    {
        prune.copy_file(
            &AnchoredSystemPathBuf::from_raw(workspace_config_path)?,
            Some(CopyDestination::All),
        )?;
    }

    let mut workspace_paths = Vec::new();
    let mut workspace_names = Vec::new();
    let mut cargo_crate_names = Vec::new();
    let workspaces = prune.internal_dependencies();
    let lockfile_keys = prune.lockfile_keys(&workspaces)?;
    for workspace in workspaces {
        let entry = prune
            .package_graph
            .package_info(&workspace)
            .ok_or_else(|| Error::MissingWorkspace(workspace.clone()))?;

        // We don't want to do any copying for the root workspace
        if let PackageName::Other(workspace) = workspace {
            if entry.toolchain == PackageToolchain::Cargo {
                // The synthetic workspace package has no directory of its
                // own (it is anchored at the root Cargo.toml); the root
                // manifest is handled by prune_cargo_workspace.
                if entry.cargo.as_ref().map(|details| details.kind)
                    == Some(cargo::CargoPackageKind::Workspace)
                {
                    continue;
                }
                prune.copy_cargo_crate(entry.package_json_path())?;
                println!(" - Added {workspace}");
                cargo_crate_names.push(workspace.clone());
                // Crates participate in turbo.json task pruning, but not in
                // the JS lockfile subgraph or package.json workspaces.
                workspace_names.push(workspace);
                continue;
            }
            prune.copy_workspace(entry.package_json_path(), &entry.package_json)?;
            let parent = entry
                .package_json_path()
                .parent()
                .expect("workspace package.json path should have a parent");
            workspace_paths.push(parent.to_unix().to_string());

            println!(" - Added {workspace}");
            workspace_names.push(workspace);
        }
    }
    prune.copy_file_dependencies(&workspace_names)?;

    if !cargo_crate_names.is_empty() {
        let extra_members = prune.prune_cargo_workspace(&cargo_crate_names)?;
        workspace_names.extend(extra_members);
    }

    trace!("new workspaces: {}", workspace_paths.join(", "));
    trace!("lockfile keys: {}", lockfile_keys.join(", "));

    let lockfile = prune
        .package_graph
        .lockfile()
        .ok_or(Error::MissingLockfile)?
        .subgraph(&workspace_paths, &lockfile_keys)?;

    let lockfile_name = prune.package_graph.package_manager().lockfile_name();

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

    let original_lockfile = prune
        .package_graph
        .lockfile()
        .ok_or(Error::MissingLockfile)?;
    let package_manager = prune.package_graph.package_manager();
    let original_patches = collect_patch_paths(
        original_lockfile,
        prune.package_graph.root_package_json(),
        &prune.root,
        package_manager,
    )?;
    let pruned_patches = if original_patches.is_empty() {
        Vec::new()
    } else {
        collect_patch_paths(
            lockfile.as_ref(),
            prune.package_graph.root_package_json(),
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
    if !original_patches.is_empty() || original_value.get("workspaces").is_some() {
        let pruned_json = if original_patches.is_empty() {
            prune.package_graph.root_package_json().clone()
        } else {
            package_manager.prune_patched_packages(
                prune.package_graph.root_package_json(),
                &pruned_patches,
                &prune.root,
            )
        };

        let mut pruned_value = serde_json::to_value(&pruned_json)?;
        prune_package_json_workspaces(&mut pruned_value, &workspace_paths);
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

        // Prune pnpm-workspace.yaml's patchedDependencies so it only
        // references patches that are actually in the pruned output.
        if matches!(
            package_manager,
            turborepo_repository::package_manager::PackageManager::Pnpm
                | turborepo_repository::package_manager::PackageManager::Pnpm6
                | turborepo_repository::package_manager::PackageManager::Pnpm9
        ) {
            let ws_config =
                turborepo_repository::package_manager::pnpm::WORKSPACE_CONFIGURATION_PATH;
            let ws_path = AnchoredSystemPathBuf::from_raw(ws_config)?;
            let full_ws = prune.full_directory.resolve(&ws_path);
            turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                &full_ws,
                &pruned_patches,
            )?;
            if prune.docker {
                let out_ws = prune.out_directory.resolve(&ws_path);
                turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                    &out_ws,
                    &pruned_patches,
                )?;
                let docker_ws = prune.docker_directory().resolve(&ws_path);
                turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                    &docker_ws,
                    &pruned_patches,
                )?;
            }
        }
    }

    Ok(())
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

        if matches!(
            package_manager,
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9
        ) {
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

        let package_graph = PackageGraph::builder(&base.repo_root, root_package_json)
            .with_allow_no_package_manager(allow_missing_package_manager)
            .with_cargo(crate::run::builder::cargo_enabled(
                &base.opts().future_flags,
            ))
            .build()
            .await?;

        let out_directory = AbsoluteSystemPathBuf::from_unknown(&base.repo_root, output_dir);

        let full_directory = match docker {
            true => out_directory.join_component("full"),
            false => out_directory.clone(),
        };

        trace!("scope: {}", scope.join(", "));
        trace!("docker: {}", docker);
        trace!("out directory: {}", &out_directory);

        for target in scope {
            let workspace = PackageName::Other(target.clone());
            let Some(info) = package_graph.package_info(&workspace) else {
                return Err(Error::MissingWorkspace(workspace));
            };
            // The synthetic workspace package has no directory; pruning it
            // would mean "copy the whole repository".
            if info.cargo.as_ref().map(|details| details.kind)
                == Some(cargo::CargoPackageKind::Workspace)
            {
                return Err(Error::CargoWorkspacePackageNotPruneable(target.clone()));
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

        if package_graph.lockfile().is_none() {
            return Err(Error::MissingLockfile);
        }

        let uses_per_workspace_lockfiles = matches!(
            package_graph.package_manager(),
            turborepo_repository::package_manager::PackageManager::Pnpm
                | turborepo_repository::package_manager::PackageManager::Pnpm6
                | turborepo_repository::package_manager::PackageManager::Pnpm9
        ) && NpmRc::from_file(&base.repo_root)
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

    fn copy_workspace(
        &self,
        package_json_path: &AnchoredSystemPath,
        workspace_package_json: &PackageJson,
    ) -> Result<(), Error> {
        let package_json_path = self.root.resolve(package_json_path);
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

        if self.docker {
            let docker_workspace_dir = self.docker_directory().resolve(&relative_workspace_dir);
            docker_workspace_dir.ensure_dir()?;
            turborepo_fs::copy_file(
                &package_json_path,
                docker_workspace_dir.resolve(package_json()),
            )?;
            self.create_docker_bin_stubs(
                workspace_package_json,
                original_dir,
                &docker_workspace_dir,
            )?;

            if self.uses_per_workspace_lockfiles {
                let lockfile_name = self.package_graph.package_manager().lockfile_name();
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

    /// Copy a Cargo crate's directory into the pruned output. Mirrors
    /// [`Self::copy_workspace`], except the docker "json" layer receives the
    /// crate's `Cargo.toml` (its manifest) rather than a `package.json`, and
    /// there are no npm bin stubs to create.
    fn copy_cargo_crate(&self, manifest_path: &AnchoredSystemPath) -> Result<(), Error> {
        let manifest_path = self.root.resolve(manifest_path);
        let original_dir = manifest_path
            .parent()
            .ok_or_else(|| Error::WorkspaceAtFilesystemRoot)?;
        let metadata = original_dir.symlink_metadata()?;
        let relative_crate_dir = AnchoredSystemPathBuf::new(&self.root, original_dir)?;
        let target_dir = self.full_directory.resolve(&relative_crate_dir);
        target_dir.create_dir_all_with_permissions(metadata.permissions())?;

        turborepo_fs::recursive_copy(
            original_dir,
            &target_dir,
            self.use_gitignore,
            Some(&self.root),
        )?;

        if self.docker {
            let docker_crate_dir = self.docker_directory().resolve(&relative_crate_dir);
            docker_crate_dir.ensure_dir()?;
            turborepo_fs::copy_file(
                &manifest_path,
                docker_crate_dir.join_component(cargo::CARGO_TOML),
            )?;
        }

        Ok(())
    }

    /// Prune the Cargo workspace machinery around the copied crates:
    ///
    /// * `Cargo.lock` is subset to the closure of the kept crates, so `cargo
    ///   build --locked` succeeds in the pruned output.
    /// * The lock walk may surface members beyond Turborepo's package-graph
    ///   closure (Cargo.lock merges dev-dependency edges, including
    ///   cycle-participating ones the package graph drops). Their manifests are
    ///   referenced by kept crates, so their directories are copied too.
    /// * The root `Cargo.toml` is rewritten: explicit `members`, filtered
    ///   `default-members`, and `[workspace.dependencies]` path entries to
    ///   removed crates dropped.
    /// * Toolchain and Cargo config files are carried over.
    ///
    /// Returns the names of any extra members added beyond `crate_names`.
    fn prune_cargo_workspace(&self, crate_names: &[String]) -> Result<Vec<String>, Error> {
        let lock_path = self.root.join_component(cargo::CARGO_LOCK);
        if !lock_path.try_exists()? {
            return Err(Error::MissingCargoLockfile);
        }
        let lock_contents = lock_path.read_to_string()?;
        let pruned_lock = turborepo_lockfiles::cargo_prune_lock(&lock_contents, crate_names)?;

        let mut kept_dirs = Vec::with_capacity(pruned_lock.members.len());
        let mut extra_members = Vec::new();
        for member in &pruned_lock.members {
            let name = PackageName::Other(member.clone());
            let info = self
                .package_graph
                .package_info(&name)
                .ok_or_else(|| Error::MissingWorkspace(name.clone()))?;
            let manifest_path = info.package_json_path();
            let dir = manifest_path
                .parent()
                .ok_or_else(|| Error::WorkspaceAtFilesystemRoot)?;
            kept_dirs.push(dir.to_unix().to_string());

            if !crate_names.contains(member) {
                self.copy_cargo_crate(manifest_path)?;
                println!(" - Added {member} (dev-dependency of a kept crate)");
                extra_members.push(member.clone());
            }
        }

        // The pruned lockfile goes to the full layer and, for docker, the
        // json layer — it is part of "everything needed to fetch
        // dependencies".
        self.full_directory
            .join_component(cargo::CARGO_LOCK)
            .create_with_contents(&pruned_lock.lockfile)?;

        let manifest_contents = self
            .root
            .join_component(cargo::CARGO_TOML)
            .read_to_string()?;
        let pruned_manifest = cargo::prune_root_manifest(&manifest_contents, &kept_dirs)?;
        self.full_directory
            .join_component(cargo::CARGO_TOML)
            .create_with_contents(&pruned_manifest)?;

        // Our lock subset is reachability-based, but Cargo's real resolution
        // is feature-aware: shrinking the workspace can deactivate features
        // that were the only reason some packages were in the closure.
        // Rather than reimplement feature unification, let Cargo minimally
        // sync its own lockfile (`--offline`: removals need no network, and
        // every retained pin is preserved) so `cargo build --locked` passes
        // in the pruned output. Failure is not fatal — the superset lock
        // still builds correctly, it just isn't `--locked`-clean.
        let sync = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--offline"])
            .current_dir(self.full_directory.as_std_path())
            .output();
        match sync {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                tracing::warn!(
                    "unable to canonicalize the pruned Cargo.lock; `cargo build --locked` may \
                     require a lockfile refresh: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(error) => {
                tracing::warn!(
                    "unable to run cargo to canonicalize the pruned Cargo.lock: {error}"
                );
            }
        }

        if self.docker {
            turborepo_fs::copy_file(
                self.full_directory.join_component(cargo::CARGO_LOCK),
                self.docker_directory().join_component(cargo::CARGO_LOCK),
            )?;
            self.docker_directory()
                .join_component(cargo::CARGO_TOML)
                .create_with_contents(&pruned_manifest)?;
        }

        for aux in [
            "rust-toolchain.toml",
            "rust-toolchain",
            ".cargo/config.toml",
            ".cargo/config",
        ] {
            let path = RelativeUnixPath::new(aux)?.to_anchored_system_path_buf();
            self.copy_file(&path, Some(CopyDestination::Docker))?;
        }

        Ok(extra_members)
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

    fn internal_dependencies(&self) -> Vec<PackageName> {
        let workspaces = std::iter::once(PackageNode::Workspace(PackageName::Root))
            .chain(
                self.scope
                    .iter()
                    .map(|workspace| PackageNode::Workspace(PackageName::Other(workspace.clone()))),
            )
            .collect::<Vec<_>>();
        let mut names = self
            .package_graph
            .transitive_closure(workspaces.iter())
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
                self.package_graph
                    .transitive_closure(workspace_nodes.iter())
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
        // Cargo packages' external dependencies live in Cargo.lock (rustc,
        // crates.io/git packages); their keys mean nothing to the JS
        // lockfile's subgraph and must not leak into it.
        let workspaces: Vec<PackageName> = workspaces
            .iter()
            .filter(|workspace| {
                self.package_graph
                    .package_info(workspace)
                    .is_none_or(|info| info.toolchain != PackageToolchain::Cargo)
            })
            .cloned()
            .collect();
        let workspaces = workspaces.as_slice();
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
    use std::collections::{BTreeMap, HashMap};

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
        bin_paths, merge_preserving_key_order, prune_package_json_workspaces, Prune,
        ADDITIONAL_FILES,
    };

    struct MockDiscovery;

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
