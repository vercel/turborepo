#![allow(clippy::result_large_err)]
//! Interface types for the turborepo query layer.
//!
//! This crate defines the traits that bridge `turborepo-lib` (the run
//! orchestrator) and `turborepo-query` (the GraphQL implementation).
//! By placing the interface here, `turborepo-lib` and `turborepo-query`
//! can compile in parallel — neither depends on the other.
//!
//! ```text
//! turborepo (binary)
//!   ├── turborepo-lib ──────► turborepo-query-api (traits)
//!   └── turborepo-query ────► turborepo-query-api (traits)
//! ```
//!
//! The binary crate implements `QueryServer` and passes it to
//! `turborepo_lib::main()`, connecting the two halves at runtime.
//!
//! Note: this crate's dependency list is larger than ideal for a pure
//! interface crate because `QueryRun` methods expose types from crates
//! like `turborepo-repository` and `turborepo-engine`. The benefit is
//! still realized because the heavy async-graphql/axum/oxc stack in
//! `turborepo-query` doesn't need to compile for `turborepo-lib`.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
};

use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;
use turborepo_boundaries::BoundariesResult;
use turborepo_engine::Built;
use turborepo_repository::{change_mapper::PackageInclusionReason, package_graph::PackageName};
use turborepo_types::TaskDefinition;

pub type BoundariesFuture<'a> = Pin<
    Box<
        dyn std::future::Future<Output = Result<BoundariesResult, turborepo_boundaries::Error>>
            + Send
            + 'a,
    >,
>;

/// The interface that the query layer requires from a "run" context.
///
/// Decouples the GraphQL query layer from the concrete `Run` type in
/// turborepo-lib, allowing the heavy async-graphql/axum/oxc dependencies
/// to compile in a separate crate.
pub trait QueryRun: Send + Sync + 'static {
    fn version(&self) -> &'static str;
    fn repo_root(&self) -> &turbopath::AbsoluteSystemPath;
    fn pkg_dep_graph(&self) -> &turborepo_repository::package_graph::PackageGraph;
    fn engine(&self) -> &turborepo_engine::Engine<Built, TaskDefinition>;
    fn scm(&self) -> &turborepo_scm::SCM;
    fn root_turbo_json(&self) -> &turborepo_turbo_json::TurboJson;

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

    fn check_boundaries(&self, show_progress: bool) -> BoundariesFuture<'_>;
}

#[derive(Debug, Error)]
pub enum AffectedPackagesError {
    #[error(transparent)]
    Resolution(#[from] turborepo_scope::filter::ResolutionError),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Error, Debug, miette::Diagnostic)]
pub enum Error {
    #[error(transparent)]
    Boundaries(#[from] turborepo_boundaries::Error),
    #[error("Failed to start GraphQL server.")]
    Server(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error("Failed to calculate affected packages: {0}")]
    AffectedPackages(#[from] AffectedPackagesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolution(#[from] turborepo_scope::filter::ResolutionError),
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
/// `turborepo-lib` uses this trait to dispatch query operations without
/// depending on `turborepo-query` directly. The concrete implementation
/// lives in the binary crate, which depends on both `turborepo-lib` and
/// `turborepo-query`.
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

    /// Start the Web UI server that serves the TUI-integrated query interface.
    ///
    /// The shared state is used to stream build events to the UI.
    fn run_web_ui_server(
        &self,
        state: turborepo_ui::wui::query::SharedState,
        run: Arc<dyn QueryRun>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + '_>>;
}

// Compile-time assertions that both traits remain object-safe.
const _: () = {
    fn _assert_query_run_object_safe(_: &dyn QueryRun) {}
    fn _assert_query_server_object_safe(_: &dyn QueryServer) {}
};
