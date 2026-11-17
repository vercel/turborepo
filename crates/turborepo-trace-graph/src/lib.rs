//! Query engine and language for the whole-repo graph of `turbo trace`.
//!
//! - [`GraphView`]: the read-only interface a composed graph implements (dense
//!   file nodes, packages, edges, uncertainties, probes, directory memberships,
//!   path lookup). [`MemGraph`] is the in-memory implementation, with a builder
//!   and the JSON format ([`json`], `SCHEMA.md`).
//! - [`parse`]: a small Bazel-query-like language (`deps`, `rdeps`, `somepath`,
//!   `allpaths`, filters, `+ - ^`, `let`, `affected`).
//! - [`Engine`]: evaluation over bitset [`NodeSet`]s with pooled BFS buffers, a
//!   lazily built reverse CSR adjacency, a sorted probe index and a directory →
//!   membership index.
//! - [`floor`]: the uncertainty floor report ([`Engine::floor`]).
//! - [`loads`]: module loads with an unknown target, bounded by the declared
//!   packages ([`Engine::bounded_loads`]).
//! - [`output`]: `list`, `json`, `dot` and `tree`.
//! - [`patch`]: derived facts layered on a graph ([`Patched`]): per-entry edges
//!   for references that depend on the entry that runs them, and hints.
//! - [`audit`]: the unlikely-path audit of an `affected` selection:
//!   plausibility signals per entry and the sources of implausible selections,
//!   ranked by trigger.
//!
//! ```
//! use turborepo_trace_graph::{Engine, MemGraph, RefKind};
//!
//! let mut b = MemGraph::builder();
//! let test = b.file("test/a.test.ts");
//! let a = b.file("src/a.ts");
//! let util = b.file("src/util.ts");
//! b.edge(test, a, RefKind::Import).edge(a, util, RefKind::Import);
//! let g = b.build().unwrap();
//!
//! let engine = Engine::new(&g);
//! let r = engine.query("rdeps(src/util.ts) ^ test/**").unwrap();
//! assert_eq!(r.nodes.iter().map(|n| g.path(n)).collect::<Vec<_>>(), ["test/a.test.ts"]);
//!
//! let r = engine.query("affected(modified(src/util.ts), test/**)").unwrap();
//! let report = r.affected.unwrap();
//! assert_eq!(report.entries[0].witness.len(), 3);
//! # use turborepo_trace_graph::GraphView;
//! ```

// Every public item is documented.
#![warn(missing_docs)]

pub mod attr;
pub mod audit;
pub mod barrel;
pub mod change;
mod engine;
pub mod floor;
pub mod glob;
mod index;
pub mod json;
pub mod loads;
mod mem;
pub mod names;
pub mod output;
mod parse;
pub mod patch;
mod set;
pub mod taskmode;
mod view;
pub mod vocab;

pub use change::{
    AffectedOptions, AffectedReport, Cause, Change, ChangeKind, ChangeList, ChangeSet,
    ChangeSource, Disposition, EntryResult, GivenChanges, Policy, ScopedPackage, UncertainSite,
    UncertaintyCounts, UncertaintyNote, Verdict,
};
pub use engine::{Engine, EngineOptions, FUNCTIONS, QueryResult, Value};
pub use floor::{
    DispositionCounts, FloorClass, FloorOrder, FloorPick, FloorRecord, FloorReport, FloorSite,
    UncertaintyClass,
};
pub use loads::{
    BoundedSite, BoundedSites, LoadReach, LoadStats, UndeclaredDependencies, UndeclaredDependency,
    Written,
};
pub use mem::{BuildError, EdgeSpec, GraphBuilder, MemGraph, guess_lang};
pub use names::{
    NameIndex, NameRef, NameReport, NamedBy, NamedResult, Packages, glob_name, join_rel,
    rest_matches,
};
pub use parse::{Expr, ExprKind, QueryError, SetOp, SrcSpan, parse};
pub use patch::{Patch, Patched};
pub use set::NodeSet;
pub use view::{
    Adjacency, Edge, GraphView, Membership, NodeId, PackageId, PackageInfo, PackageVersion, Span,
    Uncertainty,
};
pub use vocab::{EdgeAttr, EdgeFilter, Effect, Origin, Phase, RefFlags, RefKind, UncertaintyKind};
