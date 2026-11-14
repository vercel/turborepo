//! The error of the commands.

/// What can go wrong in `turbo trace`.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    /// Off Unix.
    #[error("`turbo trace` is not supported on Windows yet")]
    #[diagnostic(code(turbo_trace::unsupported))]
    Unsupported,
    /// A request the commands cannot serve (a bad combination of options, an
    /// unknown test).
    #[error("{0}")]
    #[diagnostic(code(turbo_trace::invalid_request))]
    InvalidRequest(String),
    /// The command failed (reading the repository, building the graph, ...),
    /// with the chain of causes.
    #[error("{0}")]
    #[diagnostic(code(turbo_trace::failed))]
    Failed(String),
    /// Writing the output.
    #[error("cannot write output: {0}")]
    #[diagnostic(code(turbo_trace::io))]
    Io(#[from] std::io::Error),
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Failed(format!("{e:#}"))
    }
}
