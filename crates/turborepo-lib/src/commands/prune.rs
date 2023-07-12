use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use tracing::trace;
use turbopath::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPath,
};

use super::CommandBase;
use crate::{
    config::RawTurboJSON,
    package_graph::{PackageGraph, WorkspaceName, WorkspaceNode},
    package_json::PackageJson,
    ui::BOLD,
};

// Files that should be copied from root and if they're required for install
// All paths should be given as relative unix paths
const ADDITIONAL_FILES: &[(&str, bool)] = [(".gitignore", false), (".npmrc", true)].as_slice();

pub fn prune(base: &CommandBase, scope: &[String], docker: bool, output_dir: &str) -> Result<()> {
    let prune = Prune::new(base, scope, docker, output_dir)?;

    println!(
        "Generating pruned monorepo for {} in {}",
        base.ui.apply(BOLD.apply_to(scope.join(", "))),
        base.ui.apply(BOLD.apply_to(&prune.out_directory)),
    );

    std::fs::create_dir_all(prune.out_directory.as_path())?;

    if let Some(workspace_config_path) = prune
        .package_graph
        .package_manager()
        .workspace_configuration_path()
    {
        prune.copy_file(
            &AnchoredSystemPathBuf::from_raw(workspace_config_path)?,
            true,
        )?;
    }

    let mut lockfile_keys = HashSet::new();
    let mut workspace_paths = Vec::new();
    let mut workspaces = Vec::new();
    for workspace in prune.internal_dependencies()? {
        let entry = prune
            .package_graph
            .workspace_info(&workspace)
            .ok_or_else(|| anyhow!("Workspace '{workspace}' not in package graph"))?;

        if let Some(transitive_deps) = &entry.transitive_dependencies {
            lockfile_keys.extend(transitive_deps.iter().map(|pkg| pkg.key.clone()))
        }
        // We don't want to do any copying for the root workspace
        if let WorkspaceName::Other(workspace) = workspace {
            prune.copy_workspace(entry.package_json_path())?;
            workspace_paths.push(
                entry
                    .package_json_path()
                    .parent()
                    .unwrap()
                    .to_unix()?
                    .to_string(),
            );

            println!(" - Added {workspace}");
            workspaces.push(workspace);
        }
    }
    trace!("new workspaces: {}", workspace_paths.join(", "));

    let lockfile_keys = lockfile_keys.into_iter().collect::<Vec<_>>();
    let lockfile = prune
        .package_graph
        .lockfile()
        .expect("Lockfile presence already checked")
        .subgraph(&workspace_paths, &lockfile_keys)?;

    let lockfile_contents = lockfile.encode()?;
    let lockfile_name = prune.package_graph.package_manager().lockfile_name();
    let lockfile_path = prune.out_directory.join_component(lockfile_name);
    std::fs::write(lockfile_path, &lockfile_contents)?;
    if prune.docker {
        std::fs::write(
            prune.docker_directory().join_component(lockfile_name),
            &lockfile_contents,
        )?;
    }

    for (relative_path, required_for_install) in ADDITIONAL_FILES {
        let path = RelativeUnixPath::new(relative_path)?.to_system_path();
        prune.copy_file(&path, *required_for_install)?;
    }

    prune.copy_turbo_json(&workspaces)?;

    let original_patches = prune
        .package_graph
        .lockfile()
        .expect("lockfile presence checked earlier")
        .patches()?;
    if !original_patches.is_empty() {
        let pruned_patches = lockfile.patches()?;
        let pruned_json = prune
            .package_graph
            .package_manager()
            .prune_patched_packages(prune.package_graph.root_package_json(), &pruned_patches);
        let pruned_json_contents = serde_json::to_string_pretty(&pruned_json)?;
        let original = prune.root.join_component("package.json");
        let permissions = std::fs::metadata(original)?.permissions();
        let new_package_json_path = prune.root.resolve(AnchoredSystemPath::new("package.json")?);
        new_package_json_path.create_with_contents(&pruned_json_contents)?;
        std::fs::set_permissions(&new_package_json_path, permissions)?;
        if prune.docker {
            turborepo_fs::copy_file(
                new_package_json_path,
                prune.docker_directory().join_component("package.json"),
            )?;
        }

        for patch in pruned_patches {
            prune.copy_file(&patch.to_system_path(), true)?;
        }
    } else {
        prune.copy_file(&AnchoredSystemPathBuf::from_raw("package.json")?, true)?;
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

impl<'a> Prune<'a> {
    fn new(
        base: &CommandBase,
        scope: &'a [String],
        docker: bool,
        output_dir: &str,
    ) -> Result<Self> {
        if scope.is_empty() {
            bail!("at least one target must be specified");
        }

        let root_package_json_path = base.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(&root_package_json_path)?;

        let package_graph = PackageGraph::builder(&base.repo_root, root_package_json).build()?;

        let out_directory = AbsoluteSystemPathBuf::from_unknown(&base.repo_root, output_dir);

        let full_directory = match docker {
            true => out_directory.join_component("full"),
            false => out_directory.clone(),
        };

        trace!("scope: {}", scope.join(", "));
        trace!("docker: {}", docker);
        trace!("out directory: {}", &out_directory);

        for target in scope {
            let workspace = WorkspaceName::Other(target.clone());
            let Some(info) = package_graph.workspace_info(&workspace) else {
                bail!("invalid scope: package {} not found", target);
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
            bail!("Cannot prune without parsed lockfile")
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

    fn copy_file(&self, path: &AnchoredSystemPathBuf, required_for_install: bool) -> Result<()> {
        let from_path = self.root.resolve(path);
        let full_to = self.full_directory.resolve(path);
        turborepo_fs::copy_file(&from_path, full_to)?;
        if self.docker && required_for_install {
            let docker_to = self.docker_directory().resolve(path);
            turborepo_fs::copy_file(&from_path, docker_to)?;
        }
        Ok(())
    }

    fn copy_workspace(&self, package_json_path: &AnchoredSystemPathBuf) -> Result<()> {
        let package_json_path = self.root.resolve(package_json_path);
        let original_dir = package_json_path
            .parent()
            .ok_or_else(|| anyhow!("turbo doesn't support workspaces at file system root"))?;
        let metadata = std::fs::metadata(original_dir.as_path())?;
        let target_dir = self
            .out_directory
            .resolve(&AnchoredSystemPathBuf::new(&self.root, original_dir)?);
        target_dir.create_dir_all_with_permissions(metadata.permissions())?;

        turborepo_fs::recursive_copy(original_dir, &target_dir)?;

        if self.docker {
            let docker_dir = self.docker_directory();
            docker_dir.ensure_dir()?;
            // TODO: Recursive copy usage here matches Go, but is probably unnecessary
            turborepo_fs::recursive_copy(package_json_path, docker_dir)?;
        }

        Ok(())
    }

    fn internal_dependencies(&self) -> Result<HashSet<WorkspaceName>> {
        let workspaces =
            std::iter::once(WorkspaceNode::Root)
                .chain(self.scope.iter().map(|workspace| {
                    WorkspaceNode::Workspace(WorkspaceName::Other(workspace.clone()))
                }))
                .collect::<Vec<_>>();
        let nodes = self.package_graph.transitive_closure(workspaces.iter());

        Ok(nodes
            .into_iter()
            .filter_map(|node| match node {
                WorkspaceNode::Root => None,
                WorkspaceNode::Workspace(workspace) => Some(workspace.clone()),
            })
            .collect())
    }

    fn copy_turbo_json(&self, workspaces: &[String]) -> Result<()> {
        let turbo_segment = AnchoredSystemPath::new("turbo.json")?;
        let original_turbo_path = self.root.resolve(turbo_segment);
        let new_turbo_path = self.out_directory.resolve(turbo_segment);

        let turbo_json_contents = original_turbo_path.read()?;
        let turbo_json: RawTurboJSON = serde_json::from_slice(&turbo_json_contents)?;

        let pruned_turbo_json = turbo_json.prune_tasks(workspaces);
        new_turbo_path.create_with_contents(&serde_json::to_string_pretty(&pruned_turbo_json)?)?;

        Ok(())
    }
}
