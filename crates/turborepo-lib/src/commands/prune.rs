#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::OnceLock;

use lazy_static::lazy_static;
use miette::Diagnostic;
use tracing::trace;
use turbopath::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPath,
};
use turborepo_repository::{
    package_graph::{self, PackageGraph, PackageName, PackageNode},
    package_json::PackageJson,
};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::BOLD;

use super::CommandBase;
use crate::turbo_json::RawTurboJson;

pub const DEFAULT_OUTPUT_DIR: &str = "out";

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("io error while pruning: {0}")]
    Io(#[from] std::io::Error),
    #[error("File system error while pruning. The error from the operating system is: {0}")]
    Fs(#[from] turborepo_fs::Error),
    #[error("json error while pruning: {0}")]
    Json(#[from] serde_json::Error),
    #[error("path error while pruning: {0}")]
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
    #[error("turbo doesn't support workspaces at file system root")]
    WorkspaceAtFilesystemRoot,
    #[error("at least one target must be specified")]
    NoWorkspaceSpecified,
    #[error("invalid scope: package with name {0} in package.json not found")]
    MissingWorkspace(PackageName),
    #[error("Cannot prune without parsed lockfile")]
    MissingLockfile,
    #[error("Prune is not supported for Bun")]
    BunUnsupported,
    #[error("Unable to read config: {0}")]
    Config(#[from] crate::config::Error),
}

// Files that should be copied from root and if they're required for install
lazy_static! {
    static ref ADDITIONAL_FILES: Vec<(&'static RelativeUnixPath, Option<CopyDestination>)> = vec![
        (RelativeUnixPath::new(".gitignore").unwrap(), None),
        (
            RelativeUnixPath::new(".npmrc").unwrap(),
            Some(CopyDestination::Docker)
        ),
        (
            RelativeUnixPath::new(".yarnrc.yml").unwrap(),
            Some(CopyDestination::Docker)
        ),
    ];
    static ref ADDITIONAL_DIRECTORIES: Vec<(&'static RelativeUnixPath, Option<CopyDestination>)> = vec![
        (
            RelativeUnixPath::new(".yarn/plugins").unwrap(),
            Some(CopyDestination::Docker)
        ),
        (
            RelativeUnixPath::new(".yarn/releases").unwrap(),
            Some(CopyDestination::Docker)
        ),
    ];
}

fn package_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| AnchoredSystemPath::new("package.json").unwrap())
}

fn turbo_json() -> &'static AnchoredSystemPath {
    static PATH: OnceLock<&'static AnchoredSystemPath> = OnceLock::new();
    PATH.get_or_init(|| AnchoredSystemPath::new("turbo.json").unwrap())
}

pub async fn prune(
    base: &CommandBase,
    scope: &[String],
    docker: bool,
    output_dir: &str,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_arg_usage("docker", docker);
    telemetry.track_arg_usage("out-dir", output_dir != DEFAULT_OUTPUT_DIR);

    let prune = Prune::new(base, scope, docker, output_dir, telemetry).await?;

    if matches!(
        prune.package_graph.package_manager(),
        turborepo_repository::package_manager::PackageManager::Bun
    ) {
        return Err(Error::BunUnsupported);
    }

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
    trace!("new workspaces: {}", workspace_paths.join(", "));
    trace!("lockfile keys: {}", lockfile_keys.join(", "));

    let lockfile = prune
        .package_graph
        .lockfile()
        .expect("Lockfile presence already checked")
        .subgraph(&workspace_paths, &lockfile_keys)?;

    let lockfile_contents = lockfile.encode()?;
    let lockfile_name = prune.package_graph.package_manager().lockfile_name();
    let lockfile_path = prune.out_directory.join_component(lockfile_name);
    lockfile_path.create_with_contents(&lockfile_contents)?;
    if prune.docker {
        prune
            .docker_directory()
            .join_component(lockfile_name)
            .create_with_contents(&lockfile_contents)?;
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
        let pruned_json = prune
            .package_graph
            .package_manager()
            .prune_patched_packages(prune.package_graph.root_package_json(), &pruned_patches);
        let mut pruned_json_contents = serde_json::to_string_pretty(&pruned_json)?;
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

        for patch in pruned_patches {
            prune.copy_file(
                &patch.to_anchored_system_path_buf(),
                Some(CopyDestination::Docker),
            )?;
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
        telemetry: CommandEventBuilder,
    ) -> Result<Self, Error> {
        let allow_missing_package_manager = base.config()?.allow_no_package_manager();
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
                info.package_json.name.as_deref().unwrap_or_default()
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
        turborepo_fs::recursive_copy(&from_path, full_to)?;
        if matches!(destination, Some(CopyDestination::All)) {
            let out_to = self.out_directory.resolve(path);
            turborepo_fs::recursive_copy(&from_path, out_to)?;
        }
        if self.docker
            && matches!(
                destination,
                Some(CopyDestination::Docker) | Some(CopyDestination::All)
            )
        {
            let docker_to = self.docker_directory().resolve(path);
            turborepo_fs::recursive_copy(&from_path, docker_to)?;
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

        turborepo_fs::recursive_copy(original_dir, &target_dir)?;

        if self.docker {
            let docker_workspace_dir = self.docker_directory().resolve(&relative_workspace_dir);
            docker_workspace_dir.ensure_dir()?;
            turborepo_fs::copy_file(
                package_json_path,
                docker_workspace_dir.resolve(package_json()),
            )?;
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

    fn copy_turbo_json(&self, workspaces: &[String]) -> Result<(), Error> {
        let anchored_turbo_path = turbo_json();
        let original_turbo_path = self.root.resolve(anchored_turbo_path);
        let new_turbo_path = self.full_directory.resolve(anchored_turbo_path);

        let turbo_json_contents = match original_turbo_path.read_to_string() {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // If turbo.json doesn't exist skip copying
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };

        let turbo_json = RawTurboJson::parse(&turbo_json_contents, anchored_turbo_path)?;

        let pruned_turbo_json = turbo_json.prune_tasks(workspaces);
        new_turbo_path.create_with_contents(serde_json::to_string_pretty(&pruned_turbo_json)?)?;

        Ok(())
    }
}
