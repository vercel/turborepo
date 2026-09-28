//! `turbo archive` moves a package out of the workspace into `_archived/`, and
//! `turbo unarchive` puts it back from the record written next to it.

mod workspace_definition;

use std::{
    cmp::Reverse, collections::BTreeSet, fmt, io::ErrorKind, process::Command, str::FromStr,
};

use globwalk::{ValidatedGlob, WalkType};
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPathBuf};
use turborepo_engine::EngineBuilder;
use turborepo_log::{Source, Subsystem};
use turborepo_microfrontends_config::UnifiedTurboJsonLoader;
use turborepo_repository::{
    package_graph::{self, PackageGraph, PackageGraphNodeKind, PackageName, PackageNode},
    package_json::{self, PackageJson},
    package_manager::{self, PackageManager},
};
use turborepo_run::engine_loader::EngineTurboJsonLoader;
use turborepo_task_id::TaskName;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_turbo_json::{TurboJson, TurboJsonReader};
use turborepo_ui::{BOLD, LogSinks};
use workspace_definition::WorkspaceDefinition;

use super::CommandBase;

const ARCHIVE_DIR: &str = "_archived";
const RECORD_FILE: &str = ".turbo-archive.json";

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("Invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Rewrite(#[from] turborepo_json_rewrite::RewriteError),
    #[error(transparent)]
    PackageJson(#[from] package_json::Error),
    #[error(transparent)]
    PackageGraph(#[from] package_graph::Error),
    #[error(transparent)]
    PackageManager(#[from] package_manager::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] turborepo_config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Engine(#[from] turborepo_engine::BuilderError),
    #[error("Invalid output glob: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error("Failed to find task outputs: {0}")]
    Walk(#[from] globwalk::WalkError),
    #[error("`turbo archive` requires a JavaScript package manager.")]
    NoPackageManager,
    #[error("`{0}` is not a valid package name.")]
    InvalidPackageName(String),
    #[error("Package `{0}` not found.")]
    PackageNotFound(String),
    #[error("Cannot archive `{0}`: only packages with their own package.json can be archived.")]
    NotArchivable(String),
    #[error("`{name}` is already archived at {path}.")]
    AlreadyArchived { name: String, path: String },
    #[error(
        "Cannot archive `{name}`: {nested} inside {path} would move with it. Archive those \
         packages first."
    )]
    NestedPackages {
        name: String,
        path: String,
        nested: String,
    },
    #[error(
        "The workspace globs in {file} still match {path}, so `{name}` would stay in the \
         workspace."
    )]
    #[diagnostic(help("Add `!{ARCHIVE_DIR}/**` to the workspaces in {file}."))]
    ArchiveDirMatchesWorkspaceGlobs {
        name: String,
        path: String,
        file: String,
    },
    #[error("{}", describe_dependents(name, dependents))]
    HasDependents {
        name: String,
        dependents: Vec<Dependent>,
    },
    #[error("`{name}` is not archived: {path} does not exist.")]
    NotArchived { name: String, path: String },
    #[error("Invalid archive record {path}: {reason}")]
    InvalidRecord { path: String, reason: String },
    #[error("Cannot restore `{name}`: {path} already exists.")]
    PathOccupied { name: String, path: String },
    #[error("Cannot edit the workspace definition in {file}: {reason}")]
    UnsupportedWorkspaceDefinition { file: String, reason: String },
    #[error("Failed to locate the git exclude file: {0}")]
    GitExclude(String),
}

/// Written to `_archived/<name>/.turbo-archive.json`; everything `unarchive`
/// needs.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveRecord {
    name: String,
    /// Package directory relative to the repository root, forward slashes.
    original_path: RelativeUnixPathBuf,
    /// Present when the package was listed literally in the workspace
    /// definition.
    workspace_entry: Option<WorkspaceEntry>,
    /// Lines this archive added to the git exclude file.
    #[serde(default)]
    git_exclude: Vec<String>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    /// Workspace definition file relative to the repository root.
    file: RelativeUnixPathBuf,
    /// The literal entry removed, exactly as it appeared.
    entry: String,
}

/// Something that stops working once the package leaves the workspace.
#[derive(Debug, Clone, PartialEq)]
pub enum Dependent {
    Package(PackageName),
    /// A package turbo.json lists the package in `extends`.
    Extends {
        config: String,
    },
    /// A task lists one of the package's tasks in `dependsOn` or `with`.
    TaskReference {
        config: String,
        task: String,
        field: TaskField,
        reference: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskField {
    DependsOn,
    With,
}

impl fmt::Display for TaskField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TaskField::DependsOn => "dependsOn",
            TaskField::With => "with",
        })
    }
}

impl fmt::Display for Dependent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Dependent::Package(package) => write!(f, "package `{package}` depends on it"),
            Dependent::Extends { config } => write!(f, "{config} lists it in `extends`"),
            Dependent::TaskReference {
                config,
                task,
                field,
                reference,
            } => write!(
                f,
                "task `{task}` in {config} lists `{reference}` in `{field}`"
            ),
        }
    }
}

fn describe_dependents(name: &str, dependents: &[Dependent]) -> String {
    let mut message = format!("`{name}` is still in use:");
    for dependent in dependents {
        message.push_str(&format!("\n  - {dependent}"));
    }
    message.push_str("\nPass `--force` to archive it anyway.");
    message
}

/// Where `<name>` lives while archived, relative to the repository root.
/// Package names come from package.json and the command line, so reject
/// anything that is not one or two plain path segments.
fn archived_path(name: &str) -> Result<RelativeUnixPathBuf, Error> {
    let segments: Vec<&str> = name.split('/').collect();
    let well_formed = match segments.as_slice() {
        [name] => is_plain_segment(name),
        [scope, name] => {
            scope.starts_with('@') && is_plain_segment(scope) && is_plain_segment(name)
        }
        _ => false,
    };
    if !well_formed {
        return Err(Error::InvalidPackageName(name.to_owned()));
    }
    Ok(RelativeUnixPathBuf::new(format!("{ARCHIVE_DIR}/{name}"))?)
}

fn is_plain_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains('\\')
}

fn is_plain_relative_path(path: &str) -> bool {
    path.split('/').all(is_plain_segment)
}

pub async fn archive(
    base: &CommandBase,
    name: &str,
    force: bool,
    git_exclude: bool,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_arg_usage("force", force);
    telemetry.track_arg_usage("git-exclude", git_exclude);

    let repo_root = &base.repo_root;
    let archived = archived_path(name)?;
    let archived_dir = repo_root.join_unix_path(&archived);
    if archived_dir.exists() {
        return Err(Error::AlreadyArchived {
            name: name.to_owned(),
            path: archived.to_string(),
        });
    }

    let package = PackageName::from(name);
    let package_graph = build_package_graph(base).await?;
    let package_manager = package_graph
        .package_manager()
        .ok_or(Error::NoPackageManager)?;

    let view = package_graph
        .package_view(&package)
        .ok_or_else(|| Error::PackageNotFound(name.to_owned()))?;
    let (Some(package_dir), Some(definition_path)) = (view.directory(), view.definition_path())
    else {
        return Err(Error::NotArchivable(name.to_owned()));
    };
    if view.kind() != PackageGraphNodeKind::Package
        || !view.is_package_json_scope()
        || package_dir.components().next().is_none()
    {
        return Err(Error::NotArchivable(name.to_owned()));
    }
    let original_path = package_dir.to_unix();
    let package_dir = repo_root.resolve(package_dir);
    let package_definition = repo_root.resolve(definition_path);

    let nested: Vec<String> = package_graph
        .package_scope_directories()
        .filter(|(other, directory)| {
            *other != package && package_dir.contains(&repo_root.resolve(directory))
        })
        .map(|(other, _)| format!("`{other}`"))
        .collect();
    if !nested.is_empty() {
        return Err(Error::NestedPackages {
            name: name.to_owned(),
            path: original_path.to_string(),
            nested: nested.join(", "),
        });
    }
    let workspace_definition = WorkspaceDefinition::for_package_manager(package_manager, repo_root);
    if package_manager
        .get_workspace_globs(repo_root)?
        .target_is_workspace(repo_root, &archived_dir)?
    {
        return Err(Error::ArchiveDirMatchesWorkspaceGlobs {
            name: name.to_owned(),
            path: archived.to_string(),
            file: display_path(repo_root, workspace_definition.path()),
        });
    }

    let loader = UnifiedTurboJsonLoader::workspace(
        TurboJsonReader::new(repo_root.clone()).with_future_flags(base.opts().future_flags),
        base.opts().repo_opts.root_turbo_json_path.clone(),
        package_graph.package_scope_directories(),
    );
    let dependents = find_dependents(&package_graph, &loader, &package)?;
    if !dependents.is_empty() {
        if !force {
            return Err(Error::HasDependents {
                name: name.to_owned(),
                dependents,
            });
        }
        LogSinks::new(base.color_config).init_logger();
        for dependent in &dependents {
            turborepo_log::warn(
                Source::turbo(Subsystem::Archive),
                format!("archiving `{name}` anyway: {dependent}"),
            )
            .emit();
        }
    }
    let outputs = output_globs(base, &package_graph, &loader, &package)?;

    let workspace_entry = workspace_definition
        .find_entry(original_path.as_str())?
        .map(|entry| -> Result<_, Error> {
            Ok(WorkspaceEntry {
                file: repo_root.anchor(workspace_definition.path())?.to_unix(),
                entry,
            })
        })
        .transpose()?;
    let exclude_plan = if git_exclude {
        let exclude = GitExcludeFile::locate(repo_root)?;
        let wanted = [
            exclude.line_for(original_path.as_str()),
            exclude.line_for(archived.as_str()),
        ];
        let lines = missing_lines(&exclude.read()?, &wanted);
        Some((exclude, lines))
    } else {
        None
    };
    let record = ArchiveRecord {
        name: name.to_owned(),
        original_path: original_path.clone(),
        workspace_entry,
        git_exclude: exclude_plan
            .as_ref()
            .map(|(_, lines)| lines.clone())
            .unwrap_or_default(),
    };

    println!("Archiving {}", base.color_config.apply(BOLD.apply_to(name)));
    let removed = delete_outputs(&package_dir, &outputs, &package_definition)?;
    if removed > 0 {
        println!(" - Removed {removed} task output{}", plural(removed));
    }
    for directory in ["node_modules", ".turbo"] {
        if remove_path(&package_dir.join_component(directory))? {
            println!(" - Removed {directory}");
        }
    }

    archived_dir
        .parent()
        .ok_or_else(|| Error::InvalidPackageName(name.to_owned()))?
        .create_dir_all()?;
    package_dir.rename(&archived_dir)?;
    println!(" - Moved {original_path} to {archived}");

    archived_dir
        .join_component(RECORD_FILE)
        .create_with_contents(format!("{}\n", serde_json::to_string_pretty(&record)?))?;

    if let Some(workspace_entry) = &record.workspace_entry {
        workspace_definition.remove_entry(&workspace_entry.entry)?;
        println!(
            " - Removed \"{}\" from {}",
            workspace_entry.entry, workspace_entry.file
        );
    }

    if let Some((exclude, lines)) = &exclude_plan {
        exclude.append(lines)?;
        if !lines.is_empty() {
            println!(
                " - Added {} to {}",
                lines.join(" and "),
                display_path(repo_root, &exclude.file)
            );
        }
    }

    println!(
        "Run `{} install` so the lockfile no longer lists `{name}`.",
        package_manager.command()
    );
    Ok(())
}

pub fn unarchive(base: &CommandBase, name: &str) -> Result<(), Error> {
    let repo_root = &base.repo_root;
    let archived = archived_path(name)?;
    let archived_dir = repo_root.join_unix_path(&archived);
    let record_path = archived_dir.join_component(RECORD_FILE);
    let record = read_record(repo_root, &record_path, name)?;

    let original_dir = repo_root.join_unix_path(&record.original_path);
    if original_dir.exists() {
        return Err(Error::PathOccupied {
            name: name.to_owned(),
            path: record.original_path.to_string(),
        });
    }
    println!("Restoring {}", base.color_config.apply(BOLD.apply_to(name)));

    // Edits run before the move and skip work already done, so a failed
    // unarchive can be retried while the record is still in `_archived/`.
    if let Some(workspace_entry) = &record.workspace_entry {
        WorkspaceDefinition::from_file(repo_root.join_unix_path(&workspace_entry.file))
            .add_entry(&workspace_entry.entry)?;
        println!(
            " - Added \"{}\" to {}",
            workspace_entry.entry, workspace_entry.file
        );
    }
    if !record.git_exclude.is_empty() {
        let exclude = GitExcludeFile::locate(repo_root)?;
        exclude.remove(&record.git_exclude)?;
        println!(
            " - Removed {} from {}",
            record.git_exclude.join(" and "),
            display_path(repo_root, &exclude.file)
        );
    }

    original_dir
        .parent()
        .ok_or_else(|| Error::InvalidRecord {
            path: display_path(repo_root, &record_path),
            reason: "`originalPath` is empty".into(),
        })?
        .create_dir_all()?;
    archived_dir.rename(&original_dir)?;
    original_dir.join_component(RECORD_FILE).remove_file()?;
    println!(" - Moved {archived} to {}", record.original_path);

    let archive_root = repo_root.join_component(ARCHIVE_DIR);
    for directory in archived_dir
        .ancestors()
        .skip(1)
        .take_while(|directory| archive_root.contains(directory))
    {
        let _ = directory.remove_dir();
    }

    println!(
        "Run `{} install` to link `{name}` again.",
        install_command(repo_root)
    );
    Ok(())
}

/// The install command to suggest. Unarchive must work in a repository whose
/// package manager cannot be detected, so this only falls back to a generic
/// hint rather than failing.
fn install_command(repo_root: &AbsoluteSystemPath) -> &'static str {
    PackageJson::load(&repo_root.join_component("package.json"))
        .ok()
        .and_then(|root| PackageManager::read_or_detect_package_manager(&root, repo_root).ok())
        .map_or("<package manager>", |package_manager| {
            package_manager.command()
        })
}

async fn build_package_graph(base: &CommandBase) -> Result<PackageGraph, Error> {
    let repo_root = &base.repo_root;
    let features = turborepo_package_watcher::repository_graph::RepositoryGraphFeatures::new(
        &base.opts().future_flags,
    );
    let root_package_json = features.load_root_package_json(repo_root)?;
    let builder = PackageGraph::builder_optional(repo_root, root_package_json)
        .with_allow_no_package_manager(base.opts().repo_opts.allow_no_package_manager);
    Ok(features.configure(builder).build().await?)
}

fn read_record(
    repo_root: &AbsoluteSystemPath,
    record_path: &AbsoluteSystemPath,
    name: &str,
) -> Result<ArchiveRecord, Error> {
    let display = display_path(repo_root, record_path);
    let Some(contents) = record_path.read_existing_to_string()? else {
        return Err(Error::NotArchived {
            name: name.to_owned(),
            path: display,
        });
    };
    let invalid = |reason: String| Error::InvalidRecord {
        path: display.clone(),
        reason,
    };
    let record: ArchiveRecord =
        serde_json::from_str(&contents).map_err(|error| invalid(error.to_string()))?;
    if record.name != name {
        return Err(invalid(format!("it records package `{}`", record.name)));
    }
    let paths = std::iter::once(&record.original_path)
        .chain(record.workspace_entry.iter().map(|entry| &entry.file));
    for path in paths {
        if !is_plain_relative_path(path.as_str()) {
            return Err(invalid(format!(
                "`{path}` is not a path inside the repository"
            )));
        }
    }
    Ok(record)
}

fn find_dependents(
    package_graph: &PackageGraph,
    loader: &UnifiedTurboJsonLoader,
    package: &PackageName,
) -> Result<Vec<Dependent>, Error> {
    let mut packages: Vec<PackageName> = package_graph
        .immediate_ancestors(&PackageNode::Workspace(package.clone()))
        .into_iter()
        .flatten()
        .map(|node| node.as_package_name().clone())
        .collect();
    packages.sort();
    let mut dependents: Vec<Dependent> = packages.into_iter().map(Dependent::Package).collect();

    let mut config_owners: Vec<PackageName> = package_graph
        .package_scope_directories()
        .map(|(owner, _)| owner)
        .filter(|owner| owner != package)
        .collect();
    config_owners.push(PackageName::Root);
    config_owners.sort();
    config_owners.dedup();
    for owner in config_owners {
        match loader.load(&owner) {
            Ok(turbo_json) => dependents.extend(config_dependents(package.as_str(), turbo_json)),
            Err(error) if error.is_no_turbo_json() => {}
            Err(error) => return Err(turborepo_config::Error::from(error).into()),
        }
    }
    Ok(dependents)
}

/// References to `package` in one turbo.json. Configurations synthesized
/// without a file on disk cannot name other packages, so they have none.
fn config_dependents(package: &str, turbo_json: &TurboJson) -> Vec<Dependent> {
    let Some(config) = turbo_json.path() else {
        return Vec::new();
    };
    let config = config.to_string();
    let mut dependents = Vec::new();
    if turbo_json
        .extends
        .as_inner()
        .iter()
        .any(|extended| extended == package)
    {
        dependents.push(Dependent::Extends {
            config: config.clone(),
        });
    }
    // A `<package>#<task>` key alone is harmless: turbo ignores configuration
    // for packages that are not in the workspace. References are not.
    for (task, definition) in &turbo_json.tasks.0 {
        let definition = definition.as_inner();
        let depends_on = definition
            .depends_on
            .iter()
            .flat_map(|entries| entries.as_inner())
            .map(|entry| (TaskField::DependsOn, entry));
        let with = definition
            .with
            .iter()
            .flatten()
            .map(|entry| (TaskField::With, entry));
        for (field, entry) in depends_on.chain(with) {
            let reference: &str = entry.as_inner().as_ref();
            if TaskName::from(reference).package() == Some(package) {
                dependents.push(Dependent::TaskReference {
                    config: config.clone(),
                    task: task.to_string(),
                    field,
                    reference: reference.to_owned(),
                });
            }
        }
    }
    dependents
}

#[derive(Debug, Default, PartialEq)]
struct OutputGlobs {
    inclusions: Vec<String>,
    exclusions: Vec<String>,
}

fn output_globs(
    base: &CommandBase,
    package_graph: &PackageGraph,
    loader: &UnifiedTurboJsonLoader,
    package: &PackageName,
) -> Result<OutputGlobs, Error> {
    let engine_loader = EngineTurboJsonLoader::new(loader);
    let engine = EngineBuilder::new(&base.repo_root, package_graph, &engine_loader, false)
        .with_future_flags(base.opts().future_flags)
        .with_workspaces(vec![package.clone()])
        .add_all_tasks()
        .do_not_validate_engine()
        .build()?;

    let mut inclusions = BTreeSet::new();
    let mut exclusions = BTreeSet::new();
    for (task_id, definition) in engine.task_definitions() {
        if task_id.package() == package.as_str() {
            inclusions.extend(definition.outputs.inclusions.iter().cloned());
            exclusions.extend(definition.outputs.exclusions.iter().cloned());
        }
    }
    let (inclusions, dropped_inclusions) = keep_package_local(inclusions);
    let (exclusions, dropped_exclusions) = keep_package_local(exclusions);
    if !dropped_inclusions.is_empty() || !dropped_exclusions.is_empty() {
        tracing::debug!(
            "ignoring output globs outside {package}: {:?}",
            [dropped_inclusions, dropped_exclusions].concat()
        );
    }
    Ok(OutputGlobs {
        inclusions,
        exclusions,
    })
}

/// Splits globs into those confined to the package directory and those with a
/// `..` segment, which reach other packages (`$TURBO_ROOT$` resolves to one).
fn keep_package_local(globs: impl IntoIterator<Item = String>) -> (Vec<String>, Vec<String>) {
    globs
        .into_iter()
        .partition(|glob| !glob.split('/').any(|segment| segment == ".."))
}

/// Removes files and directories the task outputs match, deepest first. A
/// matched directory that still holds excluded files is kept.
fn delete_outputs(
    package_dir: &AbsoluteSystemPath,
    outputs: &OutputGlobs,
    package_definition: &AbsoluteSystemPath,
) -> Result<usize, Error> {
    if outputs.inclusions.is_empty() {
        return Ok(0);
    }
    let parse = |globs: &[String]| -> Result<Vec<ValidatedGlob>, Error> {
        Ok(globs
            .iter()
            .map(|glob| ValidatedGlob::from_str(glob))
            .collect::<Result<_, _>>()?)
    };
    let mut matches: Vec<AbsoluteSystemPathBuf> = globwalk::globwalk(
        package_dir,
        &parse(&outputs.inclusions)?,
        &parse(&outputs.exclusions)?,
        WalkType::All,
    )?
    .into_iter()
    .filter(|path| {
        path.as_str() != package_dir.as_str() && path.as_str() != package_definition.as_str()
    })
    .collect();
    matches.sort_by_key(|path| Reverse(path.components().count()));

    let mut removed = 0;
    for path in matches {
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.is_io_error(ErrorKind::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        let result = if metadata.is_dir() {
            path.remove_dir()
        } else {
            path.remove_file()
        };
        match result {
            Ok(()) => removed += 1,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(removed)
}

/// Removes a file, symlink, or whole directory. Returns whether it existed.
fn remove_path(path: &AbsoluteSystemPath) -> Result<bool, Error> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.is_io_error(ErrorKind::NotFound) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() {
        path.remove_dir_all()?;
    } else {
        path.remove_file()?;
    }
    Ok(true)
}

/// The repository's `info/exclude` file, plus the location of the turbo
/// repository inside the git work tree, which exclude patterns are relative to.
struct GitExcludeFile {
    file: AbsoluteSystemPathBuf,
    prefix: String,
}

impl GitExcludeFile {
    fn locate(repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-prefix", "--git-path", "info/exclude"])
            .current_dir(repo_root)
            .output()
            .map_err(|error| Error::GitExclude(format!("could not run git: {error}")))?;
        if !output.status.success() {
            return Err(Error::GitExclude(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let (Some(prefix), Some(file)) = (lines.next(), lines.next()) else {
            return Err(Error::GitExclude(format!(
                "unexpected output from `git rev-parse`: {stdout}"
            )));
        };
        Ok(Self {
            file: AbsoluteSystemPathBuf::from_unknown(repo_root, file),
            prefix: prefix.to_owned(),
        })
    }

    /// An anchored pattern for a directory relative to the turbo repository.
    fn line_for(&self, directory: &str) -> String {
        format!("/{}{directory}/", self.prefix)
    }

    fn read(&self) -> Result<String, Error> {
        Ok(self.file.read_existing_to_string()?.unwrap_or_default())
    }

    fn append(&self, lines: &[String]) -> Result<(), Error> {
        if lines.is_empty() {
            return Ok(());
        }
        let contents = self.read()?;
        self.file.ensure_dir()?;
        self.file
            .create_with_contents(append_lines(&contents, lines))?;
        Ok(())
    }

    fn remove(&self, lines: &[String]) -> Result<(), Error> {
        let contents = self.read()?;
        let updated = remove_lines(&contents, lines);
        if updated != contents {
            self.file.create_with_contents(updated)?;
        }
        Ok(())
    }
}

fn missing_lines(contents: &str, wanted: &[String]) -> Vec<String> {
    wanted
        .iter()
        .filter(|line| {
            !contents
                .lines()
                .any(|existing| existing.trim_end() == *line)
        })
        .cloned()
        .collect()
}

fn append_lines(contents: &str, lines: &[String]) -> String {
    let mut updated = contents.to_owned();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for line in lines {
        updated.push_str(line);
        updated.push('\n');
    }
    updated
}

fn remove_lines(contents: &str, lines: &[String]) -> String {
    contents
        .split_inclusive('\n')
        .filter(|existing| !lines.iter().any(|line| existing.trim_end() == line))
        .collect()
}

fn display_path(repo_root: &AbsoluteSystemPath, path: &AbsoluteSystemPath) -> String {
    repo_root.anchor(path).map_or_else(
        |_| path.to_string(),
        |anchored| anchored.to_unix().to_string(),
    )
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use turborepo_turbo_json::{RawPackageTurboJson, RawTurboJson};

    use super::*;

    #[test]
    fn archived_path_accepts_plain_and_scoped_names() {
        assert_eq!(archived_path("util").unwrap().as_str(), "_archived/util");
        assert_eq!(
            archived_path("@acme/util").unwrap().as_str(),
            "_archived/@acme/util"
        );
    }

    #[test]
    fn archived_path_rejects_names_that_escape_the_archive() {
        for name in [
            "",
            "..",
            "../util",
            "a/b",
            "@acme/../x",
            "@acme/",
            "a\\b",
            "//",
        ] {
            assert!(
                matches!(archived_path(name), Err(Error::InvalidPackageName(_))),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    fn record_round_trips_as_camel_case_json() {
        let record = ArchiveRecord {
            name: "util".into(),
            original_path: RelativeUnixPathBuf::new("packages/util").unwrap(),
            workspace_entry: Some(WorkspaceEntry {
                file: RelativeUnixPathBuf::new("package.json").unwrap(),
                entry: "./packages/util".into(),
            }),
            git_exclude: vec!["/packages/util/".into()],
        };
        let serialized = serde_json::to_value(&record).unwrap();
        assert_eq!(
            serialized,
            json!({
                "name": "util",
                "originalPath": "packages/util",
                "workspaceEntry": { "file": "package.json", "entry": "./packages/util" },
                "gitExclude": ["/packages/util/"]
            })
        );
        assert_eq!(
            serde_json::from_value::<ArchiveRecord>(serialized).unwrap(),
            record
        );
    }

    #[test]
    fn keep_package_local_drops_globs_that_leave_the_package() {
        let (kept, dropped) = keep_package_local([
            "dist/**".to_string(),
            "../../coverage/**".to_string(),
            "a/../b".to_string(),
            "..foo/bar".to_string(),
        ]);
        assert_eq!(kept, ["dist/**", "..foo/bar"]);
        assert_eq!(dropped, ["../../coverage/**", "a/../b"]);
    }

    #[test]
    fn exclude_lines_are_appended_once_and_removed_exactly() {
        let wanted = vec![
            "/packages/util/".to_string(),
            "/_archived/util/".to_string(),
        ];
        let existing =
            "# git ls-files --others --exclude-from=.git/info/exclude\n/_archived/util/\n*.log";
        let missing = missing_lines(existing, &wanted);
        assert_eq!(missing, ["/packages/util/"]);

        let appended = append_lines(existing, &missing);
        assert_eq!(
            appended,
            "# git ls-files --others \
             --exclude-from=.git/info/exclude\n/_archived/util/\n*.log\n/packages/util/\n"
        );
        assert_eq!(
            remove_lines(&appended, &missing),
            "# git ls-files --others --exclude-from=.git/info/exclude\n/_archived/util/\n*.log\n"
        );
    }

    fn root_turbo_json(value: serde_json::Value) -> TurboJson {
        TurboJson::try_from(RawTurboJson::parse_from_serde(value).unwrap()).unwrap()
    }

    #[test]
    fn config_dependents_finds_task_references_but_not_task_keys() {
        let turbo_json = root_turbo_json(json!({
            "tasks": {
                "util#dev": {},
                "build": { "dependsOn": ["^build", "util#codegen"] },
                "test": { "with": ["util#serve", "utility#serve"] }
            }
        }));
        assert_eq!(
            config_dependents("util", &turbo_json),
            [
                Dependent::TaskReference {
                    config: "turbo.json".into(),
                    task: "build".into(),
                    field: TaskField::DependsOn,
                    reference: "util#codegen".into(),
                },
                Dependent::TaskReference {
                    config: "turbo.json".into(),
                    task: "test".into(),
                    field: TaskField::With,
                    reference: "util#serve".into(),
                },
            ]
        );
    }

    #[test]
    fn config_dependents_finds_extends() {
        let raw = RawPackageTurboJson::parse(
            r#"{ "extends": ["//", "util"], "tasks": {} }"#,
            "apps/web/turbo.json",
        )
        .unwrap();
        let turbo_json = TurboJson::try_from(RawTurboJson::from(raw)).unwrap();
        assert_eq!(
            config_dependents("util", &turbo_json),
            [Dependent::Extends {
                config: "apps/web/turbo.json".into()
            }]
        );
        assert!(config_dependents("web", &turbo_json).is_empty());
    }

    #[test]
    fn has_dependents_message_lists_each_dependent_and_the_escape_hatch() {
        let error = Error::HasDependents {
            name: "util".into(),
            dependents: vec![
                Dependent::Package(PackageName::from("my-app")),
                Dependent::TaskReference {
                    config: "turbo.json".into(),
                    task: "build".into(),
                    field: TaskField::DependsOn,
                    reference: "util#dev".into(),
                },
            ],
        };
        assert_eq!(
            error.to_string(),
            "`util` is still in use:\n  - package `my-app` depends on it\n  - task `build` in \
             turbo.json lists `util#dev` in `dependsOn`\nPass `--force` to archive it anyway."
        );
    }
}
