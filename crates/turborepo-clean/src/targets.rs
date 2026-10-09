//! Which tasks have outputs to clean, and which files those outputs are.

use std::collections::{BTreeMap, BTreeSet};

use turbopath::AbsoluteSystemPathBuf;
use turborepo_engine::{Built, Engine, task_has_command};
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::TaskDefinition;

use crate::{Error, names::fold, patterns};

/// Package directories, the repository root among them, folded. Neither
/// they nor any of their ancestors may be removed or swept by a glob.
pub(crate) struct ProtectedDirectories {
    directories: Vec<Vec<String>>,
}

pub(crate) fn path_segments(path: &str) -> Vec<String> {
    path.split(['/', std::path::MAIN_SEPARATOR])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_owned)
        .collect()
}

impl ProtectedDirectories {
    pub(crate) fn new(package_graph: &PackageGraph) -> Self {
        let mut directories = vec![Vec::new()];
        directories.extend(
            package_graph
                .package_task_contexts()
                .map(|context| Self::folded(context.directory().as_str())),
        );
        Self { directories }
    }

    #[cfg(test)]
    pub(crate) fn from_dirs(dirs: &[&str]) -> Self {
        let mut directories = vec![Vec::new()];
        directories.extend(dirs.iter().map(|dir| Self::folded(dir)));
        Self { directories }
    }

    fn folded(directory: &str) -> Vec<String> {
        path_segments(directory)
            .iter()
            .map(|segment| fold(segment))
            .collect()
    }

    /// Whether `components` names a protected directory or an ancestor of
    /// one.
    pub(crate) fn covers(&self, components: &[String]) -> bool {
        self.directories.iter().any(|directory| {
            directory.len() >= components.len()
                && directory
                    .iter()
                    .zip(components)
                    .all(|(protected, component)| *protected == fold(component))
        })
    }
}

/// Resolves the output files of every task in the engine. Tasks that never
/// write cacheable outputs, and output patterns outside the allowlist, are
/// reported and skipped.
pub(crate) fn output_files(
    engine: &Engine<Built, TaskDefinition>,
    package_graph: &PackageGraph,
    protected: &ProtectedDirectories,
) -> Result<(BTreeSet<AbsoluteSystemPathBuf>, Vec<String>), Error> {
    let mut task_ids: Vec<&TaskId<'static>> = engine.task_ids().collect();
    task_ids.sort_by_key(|task_id| task_id.to_string());

    let mut files = BTreeSet::new();
    let mut skipped: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut refused = Vec::new();
    for task_id in task_ids {
        // A package without the script never runs the task, so it has no
        // outputs to clean. This is the common case and stays quiet.
        if !task_has_command(engine, package_graph, task_id) {
            continue;
        }
        let (Some(definition), Some(context)) = (
            engine.task_definition(task_id),
            package_graph.package_task_context(&PackageName::from(task_id.package())),
        ) else {
            continue;
        };
        let task = task_id.to_string();
        if definition.persistent {
            skipped.entry("persistent").or_default().push(task);
            continue;
        }
        if !definition.cache {
            skipped.entry("caching disabled").or_default().push(task);
            continue;
        }
        if definition.outputs.inclusions.is_empty() {
            skipped.entry("no outputs").or_default().push(task);
            continue;
        }

        let package_directory = path_segments(context.directory().as_str());
        let mut definition = definition.clone();
        definition.outputs.inclusions.retain(|pattern| {
            match patterns::refusal(pattern, &package_directory, protected) {
                Some(reason) => {
                    refused.push(format!("• {task}: not cleaning `{pattern}` ({reason})"));
                    false
                }
                None => true,
            }
        });
        if definition.outputs.inclusions.is_empty() {
            continue;
        }

        let outputs = turborepo_run_cache::task_output_files(&definition, &context, task_id)
            .map_err(|source| Error::Outputs {
                task: task.clone(),
                source: Box::new(source),
            })?;
        // The task log is turbo's own bookkeeping, not a build artifact; a
        // cache restore rewrites it.
        files.extend(
            outputs
                .files
                .into_iter()
                .filter(|path| *path != outputs.log_file),
        );
    }
    let notices = skipped
        .into_iter()
        .map(|(reason, tasks)| format!("• Skipping tasks ({reason}): {}", tasks.join(", ")))
        .chain(refused)
        .collect();
    Ok((files, notices))
}
