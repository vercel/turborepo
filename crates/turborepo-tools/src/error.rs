use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("request to {url} failed: {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("request to {url} failed with HTTP {status}")]
    HttpStatus { url: String, status: u16 },
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("no checksum for {file} was published at {url}")]
    ChecksumMissing { url: String, file: String },
    #[error("invalid checksum {value:?}: {reason}")]
    InvalidChecksum { value: String, reason: String },
    #[error("unable to extract {archive}: {reason}")]
    Archive { archive: String, reason: String },
    #[error("unable to parse {path}: {reason}")]
    Parse { path: String, reason: String },
    #[error("unexpected response from {url}: {reason}")]
    Response { url: String, reason: String },
    #[error("{tool}: no release satisfies {requested} (from {declared_in})")]
    NoMatchingVersion {
        tool: String,
        requested: String,
        declared_in: String,
    },
    #[error("{tool}: invalid version {version:?} declared in {declared_in}: {reason}")]
    InvalidVersion {
        tool: String,
        version: String,
        declared_in: String,
        reason: String,
    },
    #[error(
        "turbo setup does not support {os}/{arch}. Install the declared toolchain manually and \
         make it available on PATH."
    )]
    UnsupportedPlatform { os: String, arch: String },
    #[error("{tool}: {reason}")]
    Unsupported { tool: String, reason: String },
    #[error("`{program}` exited with {status}{stderr}")]
    CommandFailed {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("unable to run `{program}`: {source}")]
    CommandSpawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("{0} is outside the tools directory")]
    OutsideToolsDir(String),
    #[error(
        "No toolchain declarations found in this repository.\n\nturbo setup installs the tools a \
         repository declares. Declare a version in one of:\n  - package.json `packageManager` or \
         `devEngines` (npm, pnpm, yarn, bun, and Node.js)\n  - .nvmrc, .node-version, or \
         package.json `engines.node` (Node.js)\n  - rust-toolchain.toml (Rust)\n  - \
         pyproject.toml `[tool.uv] required-version` or uv.toml (uv), .python-version (Python)\n  \
         - go.work or go.mod `toolchain` / `go` directives (Go)"
    )]
    NoDeclarations,
    #[error("unable to build HTTP client: {0}")]
    Client(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn io(path: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
