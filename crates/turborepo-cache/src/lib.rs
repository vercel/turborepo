#![feature(error_generic_member_access)]
#![feature(provide_any)]

pub mod cache_archive;
pub mod signature_authentication;

use std::{backtrace, backtrace::Backtrace};

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
    #[error("attempted to restore unsupported file type: {0:?}")]
    UnsupportedFileType(tar::EntryType, #[backtrace] Backtrace),
    #[error("file name is malformed: {0}")]
    MalformedName(String, #[backtrace] Backtrace),
    #[error("file name is not Windows-safe: {0}")]
    WindowsUnsafeName(String, #[backtrace] Backtrace),
    #[error("tar attempts to write outside of directory: {0}")]
    LinkOutsideOfDirectory(String, #[backtrace] Backtrace),
}
