//! Task mode: how build-task entries (`affected --tasks`) follow edges.
//!
//! The question a task entry answers is "does this command have to run again?".
//! Two kinds of edges say less than the default evaluation reads into them, and
//! the graph store marks both ([`RefFlags::BYTES`], [`RefFlags::LISTED`]):
//!
//! - **A file that reads another file's bytes.** A config script that reads
//!   every `package.json`, a check that lists a source directory: what the
//!   reader sees changes when the read file is modified, added or deleted, or
//!   when a task regenerates it. It does not change when a module that the read
//!   file imports changes, when a file inside the scope of one of the read
//!   file's own covered reads changes, and the read file's open uncertainty
//!   (what it would load if it ran) is not the reader's: nothing runs it here.
//!   The store sets `BYTES` on a `fs_read` or `fs_read_dir` edge between two
//!   files when nothing regenerates the target; an edge to a generated file
//!   stays plain, so its producer's inputs still reach the reader. A path that
//!   is only located (`asset` edges, `new URL(x, import.meta.url)`) is not a
//!   byte read: it may be run or bundled.
//! - **A link from a lister to a producer.** A file or hub that lists a
//!   directory above a task's output with wildcards alone (`**`, `*/*`) would
//!   see the produced files if they were in the checkout, and is linked to the
//!   producer for that reason. It names none of them: a lister that reads build
//!   output says so (`dist/**/*.js`), and that link is a full one.
//!
//! In task mode the evaluation
//!
//! 1. seeds, next to the usual seeds, the readers of every changed file over
//!    `BYTES` edges (the one hop a byte read carries);
//! 2. walks without `BYTES` and `LISTED` edges, for changes and for the
//!    uncertainty floor.
//!
//! Every other entry (tests, files) is evaluated as before: the flags are
//! ignored there, so test selection is what it was. The same reasoning holds
//! for tests; applying it to them is a separate decision, to be taken with a
//! recall check on CI history.
//!
//! What this assumes: a tracked read call (`readFileSync`, `open`,
//! `os.ReadFile`) does not execute or bundle its target. The extractors record
//! module loads, workers and child processes with their own kinds, located
//! paths as assets, and code evaluation as an uncertainty.

use crate::{
    NodeId,
    audit::TASK_LANG,
    change::{Cause, ChangeSet},
    engine::Engine,
    set::NodeSet,
    view::GraphView,
    vocab::{EdgeFilter, RefFlags},
};

/// The traversal filter of task mode over `base`.
pub fn filter(base: EdgeFilter) -> EdgeFilter {
    base.without_flags(RefFlags::BYTES | RefFlags::LISTED)
}

/// Whether `n` is a build-task node.
pub fn is_task<G: GraphView + ?Sized>(g: &G, n: NodeId) -> bool {
    g.lang(n) == Some(TASK_LANG)
}

/// Splits `entries` into the build tasks and the rest; `None` when there is no
/// task.
pub fn split<G: GraphView + ?Sized>(g: &G, entries: &NodeSet) -> Option<(NodeSet, NodeSet)> {
    let mut tasks = NodeSet::new(g.node_count());
    let mut rest = NodeSet::new(g.node_count());
    for n in entries.iter() {
        if is_task(g, n) {
            tasks.insert(n);
        } else {
            rest.insert(n);
        }
    }
    (!tasks.is_empty()).then_some((tasks, rest))
}

impl<G: GraphView> Engine<'_, G> {
    /// The readers, over [`RefFlags::BYTES`] edges that `base` accepts, of
    /// every node whose own bytes change: the changed files of `changes`,
    /// and the `seeds` whose cause is a change of the node itself
    /// (modified, deleted, or a file of a changed package). Readers already
    /// in `seeded` are left out. Each is a seed of a task-mode pass, with
    /// the change of what it reads as its cause.
    pub(crate) fn byte_readers(
        &self,
        changes: &ChangeSet,
        seeds: &[(NodeId, Cause)],
        base: EdgeFilter,
        seeded: &mut NodeSet,
    ) -> Vec<(NodeId, Cause)> {
        let g = self.graph();
        let rev = self.reverse();
        let mut out = Vec::new();
        let mut readers = |n: NodeId, cause: &dyn Fn() -> Cause| {
            let (srcs, attrs) = rev.neighbors(n);
            for (&s, &a) in srcs.iter().zip(attrs) {
                if a.flags().contains(RefFlags::BYTES)
                    && base.accepts(a)
                    && seeded.insert(NodeId(s))
                {
                    out.push((NodeId(s), cause()));
                }
            }
        };
        for c in &changes.files {
            if let Some(n) = g.node_by_path(&c.path) {
                readers(n, &|| Cause::Modified {
                    path: c.path.clone(),
                });
            }
        }
        for (n, cause) in seeds {
            if matches!(
                cause,
                Cause::Modified { .. } | Cause::Deleted { .. } | Cause::Package { .. }
            ) {
                readers(*n, &|| cause.clone());
            }
        }
        out
    }
}
