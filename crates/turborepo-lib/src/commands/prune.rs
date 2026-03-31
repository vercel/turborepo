#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    str::FromStr,
    sync::{LazyLock, OnceLock},
};

use globwalk::{ValidatedGlob, WalkType};
use miette::Diagnostic;
use tracing::trace;
use turbopath::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPath,
    RelativeUnixPathBuf,
};
use turborepo_repository::{
    package_graph::{self, PackageGraph, PackageName, PackageNode},
    package_json::PackageJson,
    package_manager::npmrc::NpmRc,
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
}

static ADDITIONAL_FILES: LazyLock<Vec<(&'static RelativeUnixPath, Option<CopyDestination>)>> =
    LazyLock::new(|| {
        vec![
            (RelativeUnixPath::new(".gitignore").unwrap(), None),
            (
                RelativeUnixPath::new(".npmrc").unwrap(),
                Some(CopyDestination::Docker),
            ),
            (
                RelativeUnixPath::new(".yarnrc.yml").unwrap(),
                Some(CopyDestination::Docker),
            ),
            (
                RelativeUnixPath::new("bunfig.toml").unwrap(),
                Some(CopyDestination::Docker),
            ),
        ]
    });
static ADDITIONAL_DIRECTORIES: LazyLock<Vec<(&'static RelativeUnixPath, Option<CopyDestination>)>> =
    LazyLock::new(|| {
        vec![
            (
                RelativeUnixPath::new(".yarn/plugins").unwrap(),
                Some(CopyDestination::Docker),
            ),
            (
                RelativeUnixPath::new(".yarn/releases").unwrap(),
                Some(CopyDestination::Docker),
            ),
        ]
    });

fn package_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| AnchoredSystemPath::new("package.json").unwrap())
}

fn turbo_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| AnchoredSystemPath::new(CONFIG_FILE).unwrap())
}

fn turbo_jsonc() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| AnchoredSystemPath::new(CONFIG_FILE_JSONC).unwrap())
}

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
    let workspaces = prune.internal_dependencies();
    let lockfile_keys: Vec<_> = prune
        .package_graph
        .transitive_external_dependencies(workspaces.iter())
        .into_iter()
        .map(|pkg| pkg.key.clone())
        .collect();
    for workspace in workspaces {
        let entry = prune
            .package_graph
            .package_info(&workspace)
            .ok_or_else(|| Error::MissingWorkspace(workspace.clone()))?;

        // We don't want to do any copying for the root workspace
        if let PackageName::Other(workspace) = workspace {
            prune.copy_workspace(entry.package_json_path())?;
            workspace_paths.push(
                entry
                    .package_json_path()
                    .parent()
                    .unwrap()
                    .to_unix()
                    .to_string(),
            );

            println!(" - Added {workspace}");
            workspace_names.push(workspace);
        }
    }
    prune.copy_file_dependencies(&workspace_names)?;

    trace!("new workspaces: {}", workspace_paths.join(", "));
    trace!("lockfile keys: {}", lockfile_keys.join(", "));

    let lockfile = prune
        .package_graph
        .lockfile()
        .expect("Lockfile presence already checked")
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

    let original_patches = prune
        .package_graph
        .lockfile()
        .expect("lockfile presence checked earlier")
        .patches()?;
    if !original_patches.is_empty() {
        let pruned_patches = lockfile.patches()?;
        trace!(
            "original patches: {:?}, pruned patches: {:?}",
            original_patches,
            pruned_patches
        );

        let repo_root = &prune.root;
        let package_manager = prune.package_graph.package_manager();

        let pruned_json = package_manager.prune_patched_packages(
            prune.package_graph.root_package_json(),
            &pruned_patches,
            repo_root,
        );

        // Read the original package.json as serde_json::Value to preserve key
        // ordering. Serializing the PackageJson struct directly would sort keys
        // alphabetically, producing a byte-different file that invalidates
        // cache hashes. See https://github.com/vercel/turborepo/issues/12369
        let original_contents = prune.root.resolve(package_json()).read_to_string()?;
        let original_value: serde_json::Value = serde_json::from_str(&original_contents)?;
        let pruned_value = serde_json::to_value(&pruned_json)?;
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

        for patch in &pruned_patches {
            prune.copy_file(
                &patch.to_anchored_system_path_buf(),
                Some(CopyDestination::Docker),
            )?;
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
                let docker_ws = prune.docker_directory().resolve(&ws_path);
                turborepo_repository::package_manager::pnpm::prune_workspace_patches(
                    &docker_ws,
                    &pruned_patches,
                )?;
            }
        }
    } else {
        prune.copy_file(package_json(), Some(CopyDestination::Docker))?;
    }

    Ok(())
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
        turborepo_fs::recursive_copy(&from_path, full_to, self.use_gitignore)?;
        if matches!(destination, Some(CopyDestination::All)) {
            let out_to = self.out_directory.resolve(path);
            turborepo_fs::recursive_copy(&from_path, out_to, self.use_gitignore)?;
        }
        if self.docker
            && matches!(
                destination,
                Some(CopyDestination::Docker) | Some(CopyDestination::All)
            )
        {
            let docker_to = self.docker_directory().resolve(path);
            turborepo_fs::recursive_copy(&from_path, docker_to, self.use_gitignore)?;
        }
        Ok(())
    }

    fn copy_workspace(&self, package_json_path: &AnchoredSystemPath) -> Result<(), Error> {
        let package_json_path = self.root.resolve(package_json_path);
        let original_dir = package_json_path
            .parent()
            .ok_or_else(|| Error::WorkspaceAtFilesystemRoot)?;
        let metadata = original_dir.symlink_metadata()?;
        let relative_workspace_dir = AnchoredSystemPathBuf::new(&self.root, original_dir)?;
        let target_dir = self.full_directory.resolve(&relative_workspace_dir);
        target_dir.create_dir_all_with_permissions(metadata.permissions())?;

        turborepo_fs::recursive_copy(original_dir, &target_dir, self.use_gitignore)?;

        if self.docker {
            let docker_workspace_dir = self.docker_directory().resolve(&relative_workspace_dir);
            docker_workspace_dir.ensure_dir()?;
            turborepo_fs::copy_file(
                &package_json_path,
                docker_workspace_dir.resolve(package_json()),
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
        let nodes = self.package_graph.transitive_closure(workspaces.iter());

        let mut names: Vec<_> = nodes
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Root => None,
                PackageNode::Workspace(workspace) => Some(workspace.clone()),
            })
            .collect();
        names.sort();
        names
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
    use serde_json::json;

    use super::merge_preserving_key_order;

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
}
