//! Measurement tooling (not a product feature): a recorder for what each
//! evaluation pass of [`Engine::affected_with`] used, so that a selection can
//! be attributed to its causes outside the engine
//! (`TURBO_TRACE_DEBUG_ATTR_DUMP`, where debug variables are compiled in).
//!
//! A pass is one multi-source reverse search: its seeds with their causes, the
//! candidates whose seeding barrel pruning deferred, the open uncertainty
//! sites, and the edge filter. With those and the edges of the view, a script
//! can reproduce the selection, cut edges or seeds, and compute dominators.

use std::sync::Mutex;

use crate::{
    change::{AffectedOptions, Cause, ChangeKind, Disposition},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::{EdgeFilter, RefFlags},
};

/// One recorded pass.
#[derive(Clone, Debug, Default)]
pub struct PassRecord {
    /// Whether the pass skipped type-only edges.
    pub runtime_only: bool,
    /// Whether the pass skipped data edges for changes (never, today).
    pub data_excluded: bool,
    /// The entries of the pass.
    pub entries: Vec<u32>,
    /// The seeds: node, cause kind, cause detail.
    pub seeds: Vec<(u32, &'static str, String)>,
    /// Candidates whose seeding was deferred to barrel and erase pruning.
    pub deferred: Vec<u32>,
    /// Sites with an open record that counts for every entry.
    pub open: Vec<u32>,
    /// Sites whose open records all are resolved per entry.
    pub per_entry: Vec<u32>,
    /// The entries those per-entry sites don't count for.
    pub exempt: Vec<u32>,
    /// Whether the policy of the pass counts open uncertainty.
    pub floor: bool,
}

static REC: Mutex<Option<Vec<PassRecord>>> = Mutex::new(None);

/// Starts recording passes.
pub fn start() {
    *REC.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Vec::new());
}

/// Stops recording and returns the recorded passes.
pub fn take() -> Vec<PassRecord> {
    REC.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .unwrap_or_default()
}

fn enabled() -> bool {
    REC.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
}

/// A cause as `(kind, detail)`.
pub fn cause_fields<G: GraphView>(g: &G, node: NodeId, c: &Cause) -> (&'static str, String) {
    let kind = |k: ChangeKind| match k {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
    };
    match c {
        Cause::Modified { path } if g.path(node) == path => ("modified", path.clone()),
        Cause::Deleted { path } if g.path(node) == path => ("deleted", path.clone()),
        Cause::Modified { path } | Cause::Deleted { path } => ("input", path.clone()),
        Cause::Probe { added, probe } => ("probe", format!("{added}\u{1f}{probe}")),
        Cause::Membership {
            path,
            kind: k,
            dir,
            pattern,
        } => (
            "membership",
            format!("{path}\u{1f}{}\u{1f}{dir}\u{1f}{pattern}", kind(*k)),
        ),
        Cause::Package { package } => {
            let info = g.package_info(*package);
            (
                "package",
                format!("{}@{}", info.name, info.version.as_str()),
            )
        }
        Cause::Scope {
            path,
            kind: k,
            dir,
            pattern,
        } => (
            "scope",
            format!("{path}\u{1f}{}\u{1f}{dir}\u{1f}{pattern}", kind(*k)),
        ),
        Cause::Named { path, by, .. } => ("named", format!("{path}\u{1f}{by}")),
    }
}

impl<G: GraphView> Engine<'_, G> {
    /// Records one pass when recording is on (see the [module
    /// documentation](self)).
    pub(crate) fn attr_record(
        &self,
        opts: &AffectedOptions,
        filter: EdgeFilter,
        entries: &NodeSet,
        seeds: &[(NodeId, Cause)],
        deferred: impl Iterator<Item = NodeId>,
    ) {
        if !enabled() {
            return;
        }
        let g = self.graph();
        let mut rec = PassRecord {
            runtime_only: filter.exclude_flags.contains(RefFlags::TYPE_ONLY),
            data_excluded: filter.exclude_flags.contains(RefFlags::DATA),
            entries: entries.iter().map(|n| n.0).collect(),
            seeds: seeds
                .iter()
                .map(|(n, c)| {
                    let (k, d) = cause_fields(g, *n, c);
                    (n.0, k, d)
                })
                .collect(),
            deferred: deferred.map(|n| n.0).collect(),
            floor: opts.policy != crate::change::Policy::Ignore,
            ..PassRecord::default()
        };
        // As in `Engine::affected_pass`.
        let exempt = g.resolved_entries();
        for &n in self.uncertain_nodes() {
            if g.is_external(n) {
                continue;
            }
            let (mut open, mut resolved) = (false, false);
            for u in g.uncertainties(n) {
                if self.disposition_of(n, &u, opts) != Disposition::Open {
                    continue;
                }
                if exempt.is_some() && g.is_entry_resolved(n, &u) {
                    resolved = true;
                } else {
                    open = true;
                    break;
                }
            }
            if open {
                rec.open.push(n.0);
            } else if resolved {
                rec.per_entry.push(n.0);
            }
        }
        if let Some(x) = exempt {
            rec.exempt = x.iter().map(|n| n.0).collect();
        }
        if let Some(v) = REC
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            v.push(rec);
        }
    }
}
