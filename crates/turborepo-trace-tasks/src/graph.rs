//! Resolving [`TaskRef`]s to [`TaskId`]s over the workspace (the task graph).
//!
//! | Reference | Resolves to |
//! |---|---|
//! | `Local { name }` | `task:<same package>#name` |
//! | `Upstream { name }` (`^name`) | `name` in every workspace package the task's package depends on, transitively |
//! | `Package { package, name }` | `name` in the package with that manifest name or root; globs (`@acme/*`, `./apps/*`) match names and roots |
//! | `All { name }` | `name` in every package that has it |
//! | `Id { id }` | `id` |
//!
//! References to tasks that don't exist resolve to nothing (a missing script
//! fails at run time, so it can't be a hidden input). Transitive `^` is a sound
//! over-approximation of turbo's semantics.

use std::collections::BTreeSet;

use crate::{
    TaskSet,
    sources::{Workspace, norm_rel},
    spec::{TaskId, TaskRef},
};

/// Fills [`crate::Task::deps`] for every task in `set`.
pub fn resolve(ws: &Workspace, set: &mut TaskSet) {
    let ids: BTreeSet<TaskId> = set.tasks.iter().map(|t| t.id.clone()).collect();
    let resolved: Vec<Vec<TaskId>> = set
        .tasks
        .iter()
        .map(|t| {
            let mut out = BTreeSet::new();
            for d in &t.spec.task_deps {
                for id in resolve_ref(ws, &ids, &t.package, &d.target) {
                    if id != t.id {
                        out.insert(id);
                    }
                }
            }
            out.into_iter().collect()
        })
        .collect();
    for (t, d) in set.tasks.iter_mut().zip(resolved) {
        t.deps = d;
    }
}

/// Resolves one reference made from package `from` (repo-relative root).
pub fn resolve_ref(ws: &Workspace, ids: &BTreeSet<TaskId>, from: &str, r: &TaskRef) -> Vec<TaskId> {
    let have = |id: TaskId| ids.contains(&id).then_some(id);
    match r {
        TaskRef::Local { name } => have(TaskId::script(from, name)).into_iter().collect(),
        TaskRef::Upstream { name } => {
            let Some(pkg) = ws.packages.iter().find(|p| p.rel == from) else {
                return Vec::new();
            };
            ws.transitive_deps(pkg)
                .into_iter()
                .filter_map(|p| have(TaskId::script(p.rel.clone(), name)))
                .collect()
        }
        TaskRef::Package { package, name } => {
            if crate::glob::is_glob(package) {
                let pat = norm_rel(package);
                ws.packages
                    .iter()
                    .filter(|p| {
                        p.name
                            .as_deref()
                            .is_some_and(|n| crate::glob::matches(package, n))
                            || crate::glob::matches(&pat, &p.rel)
                    })
                    .filter_map(|p| have(TaskId::script(p.rel.clone(), name)))
                    .collect()
            } else {
                ws.package(package)
                    .and_then(|p| have(TaskId::script(p.rel.clone(), name)))
                    .into_iter()
                    .collect()
            }
        }
        TaskRef::All { name } => ids
            .iter()
            .filter(|id| matches!(id, TaskId::Script { name: n, .. } if n == name))
            .cloned()
            .collect(),
        TaskRef::Id { id } => have(id.clone()).into_iter().collect(),
    }
}
