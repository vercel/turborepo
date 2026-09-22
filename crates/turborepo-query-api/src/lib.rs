#![allow(clippy::result_large_err)]
//! Interface types for the turborepo query layer.
//!
//! This crate defines the traits that bridge `turborepo-run` (the run
//! orchestrator) and `turborepo-query` (the GraphQL implementation).
//! By placing the interface here, `turborepo-run` and `turborepo-query`
//! can compile in parallel — neither depends on the other.
//!
//! ```text
//! turborepo (binary)
//!   ├── turborepo-cli ──────► turborepo-run ──► turborepo-query-api (traits)
//!   └── turborepo-query ────► turborepo-query-api (traits)
//! ```
//!
//! The binary crate implements `QueryServer` and passes it to
//! `turborepo_cli::main()`, connecting the two halves at runtime.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
};

use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;
use turborepo_repository::{change_mapper::PackageInclusionReason, package_graph::PackageName};
use turborepo_run_context::RepoContext;
use turborepo_types::TaskDefinition;

/// A task identity used by the query contract without exposing the engine's
/// typestate or graph representation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct QueryTaskId {
    pub package: String,
    pub task: String,
}

impl QueryTaskId {
    pub fn new(package: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            task: task.into(),
        }
    }

    pub fn full_name(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for QueryTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.package, self.task)
    }
}

/// A boundary violation projected into the data needed by the query schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryDiagnostic {
    pub message: String,
    pub reason: Option<String>,
    pub path: Option<String>,
    pub import: Option<String>,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

pub type BoundariesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<BoundaryDiagnostic>, Error>> + Send + 'a>>;

/// The interface that the query layer requires from a run context.
///
/// Repository data is exposed through `RepoContext`, while engine data is
/// projected into task IDs, definitions, and graph traversals. This keeps the
/// engine's built-state type and graph implementation out of the interface.
pub trait QueryRun: Send + Sync + 'static {
    fn repo_context(&self) -> &RepoContext;

    fn task_ids(&self) -> Vec<QueryTaskId>;
    fn task_ids_for_package(&self, package: &str) -> Vec<QueryTaskId>;
    fn task_definition(&self, task_id: &QueryTaskId) -> Option<&TaskDefinition>;
    fn task_dependencies(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId>;
    fn task_dependents(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId>;
    fn transitive_task_dependencies(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId>;
    fn transitive_task_dependents(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId>;
    fn collect_task_dependencies(&self, task_ids: &HashSet<QueryTaskId>) -> HashSet<QueryTaskId>;

    fn calculate_affected_packages(
        &self,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, AffectedPackagesError>;

    /// Returns the set of files that changed between two git refs.
    /// Used by `affectedTasks` to match changed files against task input globs.
    fn changed_files(
        &self,
        base: Option<&str>,
        head: Option<&str>,
    ) -> Result<HashSet<AnchoredSystemPathBuf>, AffectedPackagesError>;

    /// Matches changed files against task inputs without exposing the engine.
    fn match_tasks_against_changed_files(
        &self,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> Result<HashMap<QueryTaskId, String>, AffectedPackagesError>;

    fn check_boundaries(&self, show_progress: bool) -> BoundariesFuture<'_>;
}

#[derive(Debug, Error)]
pub enum AffectedPackagesError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Error, Debug, miette::Diagnostic)]
pub enum Error {
    #[error(transparent)]
    Boundaries(Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to start GraphQL server.")]
    Server(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("Failed to calculate affected packages: {0}")]
    AffectedPackages(#[from] AffectedPackagesError),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
    /// Opaque error from the query implementation crate.
    #[error(transparent)]
    Query(Box<dyn std::error::Error + Send + Sync>),
}

/// An error with source location information from a GraphQL query.
pub struct QueryErrorLocation {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// The result of executing a GraphQL query.
pub struct QueryResult {
    pub result_json: String,
    pub errors: Vec<QueryErrorLocation>,
}

/// The standard GraphQL introspection query used by `turbo query --schema`.
pub const SCHEMA_QUERY: &str = include_str!("schema_query.graphql");

/// Abstraction over the query execution layer.
///
/// `turborepo-run` uses this trait to dispatch query operations without
/// depending on `turborepo-query` directly. The concrete implementation
/// lives in the binary crate, which depends on `turborepo-cli`,
/// `turborepo-run`, and `turborepo-query`.
pub trait QueryServer: Send + Sync {
    /// Execute a single GraphQL query and return the result as JSON.
    ///
    /// `variables_json` is an optional JSON string of query variables.
    fn execute_query<'a>(
        &'a self,
        run: Arc<dyn QueryRun>,
        query: &'a str,
        variables_json: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<QueryResult, Error>> + Send + 'a>>;

    /// Start an interactive GraphiQL server on localhost.
    ///
    /// Blocks until the signal handler fires. Opens the browser automatically.
    fn run_query_server(
        &self,
        run: Arc<dyn QueryRun>,
        signal: turborepo_signals::SignalHandler,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + '_>>;
}

// Compile-time assertions that both traits remain object-safe.
const _: () = {
    fn _assert_query_run_object_safe(_: &dyn QueryRun) {}
    fn _assert_query_server_object_safe(_: &dyn QueryServer) {}
};
