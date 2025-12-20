//! Cache management for task outputs
//! Provides local and remote caching capabilities and a "multiplexed" cache
//! which operates over both. When both are in use local is preferred and remote
//! writes are done asynchronously. Under the hood cache artifacts are stored a
//! gzipped tarballs.

#![feature(error_generic_member_access)]
#![feature(assert_matches)]
#![feature(box_patterns)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]

/// A wrapper for the cache that uses a worker pool to perform cache operations
mod async_cache;
/// The core cache creation and restoration logic.
pub mod cache_archive;
pub mod config;
/// File system cache
pub mod fs;
/// Remote cache
pub mod http;
/// A wrapper that allows reads and writes from the file system and remote
/// cache.
mod multiplexer;
/// Cache signature authentication lets users provide a private key to sign
/// their cache payloads.
pub mod signature_authentication;
#[cfg(test)]
mod test_cases;
mod upload_progress;

use std::{backtrace, backtrace::Backtrace};

pub use async_cache::AsyncCache;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::signature_authentication::SignatureError;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error, #[backtrace] backtrace::Backtrace),
    #[error(
        "artifact verification failed: Downloaded artifact is missing required x-artifact-tag \
         header"
    )]
    ArtifactTagMissing(#[backtrace] Backtrace),
    #[error("invalid artifact verification tag")]
    InvalidTag(#[backtrace] Backtrace),
    #[error("cannot untar file to {0}")]
    InvalidFilePath(String, #[backtrace] Backtrace),
    #[error("failed to contact remote cache: {0}")]
    ApiClientError(Box<turborepo_api_client::Error>, #[backtrace] Backtrace),
    #[error("the cache artifact for {0} was too large to upload within the timeout")]
    TimeoutError(String),
    #[error("could not connect to the cache")]
    ConnectError,
    #[error("signing artifact failed: {0}")]
    SignatureError(#[from] SignatureError, #[backtrace] Backtrace),
    #[error("invalid duration")]
    InvalidDuration(#[backtrace] Backtrace),
    #[error("Invalid file path: {0}")]
    PathError(#[from] turbopath::PathError, #[backtrace] Backtrace),
    #[error("links in the cache are cyclic")]
    CycleDetected(#[backtrace] Backtrace),
    #[error("Invalid file path, link target does not exist: {0}")]
    LinkTargetDoesNotExist(String, #[backtrace] Backtrace),
    #[error("Invalid tar, link target does not exist on header")]
    LinkTargetNotOnHeader(#[backtrace] Backtrace),
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error("attempted to restore unsupported file type: {0:?}")]
    RestoreUnsupportedFileType(tar::EntryType, #[backtrace] Backtrace),
    // We don't pass the `FileType` because there's no simple
    // way to display it nicely.
    #[error("attempted to create unsupported file type")]
    CreateUnsupportedFileType(#[backtrace] Backtrace),
    #[error("tar file is malformed")]
    MalformedTar(#[backtrace] Backtrace),
    #[error("file name is not Windows-safe: {0}")]
    WindowsUnsafeName(String, #[backtrace] Backtrace),
    #[error("tar attempts to write outside of directory: {0}")]
    LinkOutsideOfDirectory(String, #[backtrace] Backtrace),
    #[error("Invalid cache metadata file")]
    InvalidMetadata(serde_json::Error, #[backtrace] Backtrace),
    #[error("Failed to write cache metadata file")]
    MetadataWriteFailure(serde_json::Error, #[backtrace] Backtrace),
    #[error("Unable to perform write as cache is shutting down")]
    CacheShuttingDown,
    #[error("Unable to determine config cache base")]
    ConfigCacheInvalidBase,
    #[error("Unable to hash config cache inputs")]
    ConfigCacheError,
    #[error("Insufficient permissions to write to remote cache. Please verify that your role has write access for Remote Cache Artifact at https://vercel.com/docs/accounts/team-members-and-roles/access-roles/team-level-roles?resource=Remote+Cache+Artifact")]
    ForbiddenRemoteCacheWrite,
}

impl From<turborepo_api_client::Error> for CacheError {
    fn from(value: turborepo_api_client::Error) -> Self {
        CacheError::ApiClientError(Box::new(value), Backtrace::capture())
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum CacheSource {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct CacheHitMetadata {
    pub source: CacheSource,
    pub time_saved: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize)]
pub struct CacheActions {
    pub read: bool,
    pub write: bool,
}

impl CacheActions {
    pub fn should_use(&self) -> bool {
        self.read || self.write
    }

    pub fn disabled() -> Self {
        Self {
            read: false,
            write: false,
        }
    }

    pub fn enabled() -> Self {
        Self {
            read: true,
            write: true,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, Serialize)]
pub struct CacheConfig {
    pub local: CacheActions,
    pub remote: CacheActions,
}

impl CacheConfig {
    pub fn skip_writes(&self) -> bool {
        !self.local.write && !self.remote.write
    }

    pub fn remote_only() -> Self {
        Self {
            local: CacheActions::disabled(),
            remote: CacheActions::enabled(),
        }
    }

    pub fn remote_read_only() -> Self {
        Self {
            local: CacheActions::disabled(),
            remote: CacheActions {
                read: true,
                write: false,
            },
        }
    }
}

impl Default for CacheActions {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CacheOpts {
    pub cache_dir: Utf8PathBuf,
    pub cache: CacheConfig,
    pub workers: u32,
    pub remote_cache_opts: Option<RemoteCacheOpts>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteCacheOpts {
    unused_team_id: Option<String>,
    signature: bool,
}

impl RemoteCacheOpts {
    pub fn new(unused_team_id: Option<String>, signature: bool) -> Self {
        Self {
            unused_team_id,
            signature,
        }
    }
}
