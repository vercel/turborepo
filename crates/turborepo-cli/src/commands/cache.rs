use miette::Diagnostic;
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::CacheClient;
use turborepo_cache::{
    CacheError, LazyScmState,
    fs::{FSCache, LocalArtifact},
    http::HTTPCache,
};
use turborepo_log::{Source, Subsystem};
use turborepo_run_opts::RemoteCacheDisabledReason;
use turborepo_ui::LogSinks;
use turborepo_vercel_api::CachingStatus;

use super::CommandBase;
use crate::config;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Remote Cache is not configured for this repository.")]
    #[diagnostic(help(
        "Run `turbo login` and `turbo link`, or set both TURBO_TOKEN and TURBO_TEAM."
    ))]
    NotLinked,
    #[error("Remote Cache is disabled in configuration.")]
    #[diagnostic(help("Remove `remoteCache.enabled: false` from turbo.json to push artifacts."))]
    DisabledInConfig,
    #[error("Remote Cache writes are disabled by configuration.")]
    #[diagnostic(help(
        "Unset TURBO_REMOTE_CACHE_READ_ONLY, or make TURBO_CACHE allow remote writes (e.g. \
         `remote:rw`)."
    ))]
    WritesDisabled,
    #[error("No local cache artifact found for {hashes} in {cache_dir}.")]
    #[diagnostic(help(
        "Run the task locally first. Task hashes are listed by `turbo run <task> --dry=json`."
    ))]
    NotInLocalCache { hashes: String, cache_dir: String },
    #[error("Remote Caching is disabled for this team.")]
    #[diagnostic(help("A team owner can enable it at https://vercel.com/dashboard."))]
    RemoteCachingDisabledForTeam,
    #[error("Remote Cache usage limit reached for this team.")]
    UsageLimitExceeded,
    #[error("Remote Cache spending is paused for this team.")]
    SpendingPaused,
    #[error("Failed to check Remote Cache status: {0}")]
    Status(#[source] turborepo_api_client::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] config::Error),
    #[error(transparent)]
    Cache(#[from] CacheError),
}

/// Uploads locally cached task artifacts to the Remote Cache. Returns the
/// process exit code: 0 if every artifact was uploaded, 1 otherwise.
pub async fn push(base: &CommandBase, hashes: &[String]) -> Result<i32, Error> {
    let sinks = LogSinks::new(base.color_config);
    sinks.init_logger();
    sinks.enable_for_stream();

    let opts = &base.opts;
    let api_auth = match base.api_auth()? {
        Some(auth) if auth.is_linked() => auth,
        _ => return Err(Error::NotLinked),
    };
    match opts.remote_cache_disabled_reason {
        Some(RemoteCacheDisabledReason::InConfig) => return Err(Error::DisabledInConfig),
        Some(_) => return Err(Error::WritesDisabled),
        None if !opts.cache_opts.cache.remote.write => return Err(Error::WritesDisabled),
        None => {}
    }

    // Resolve every hash before touching the network so a typo uploads
    // nothing.
    let fs_cache = FSCache::new(
        &opts.cache_opts.cache_dir,
        &base.repo_root,
        None,
        LazyScmState::resolved(None),
    )?;
    let mut artifacts: Vec<(&str, LocalArtifact)> = Vec::with_capacity(hashes.len());
    let mut missing = Vec::new();
    for hash in hashes {
        match fs_cache.local_artifact(hash)? {
            Some(artifact) => artifacts.push((hash, artifact)),
            None => missing.push(hash.as_str()),
        }
    }
    if !missing.is_empty() {
        return Err(Error::NotInLocalCache {
            hashes: missing.join(", "),
            cache_dir: AbsoluteSystemPathBuf::from_unknown(
                &base.repo_root,
                &opts.cache_opts.cache_dir,
            )
            .to_string(),
        });
    }

    let api_client = base.api_client()?;
    let status = api_client
        .get_caching_status(
            &api_auth.token,
            api_auth.team_id.as_deref(),
            api_auth.team_slug.as_deref(),
        )
        .await
        .map_err(Error::Status)?;
    match status.status {
        CachingStatus::Enabled => {}
        CachingStatus::Disabled => return Err(Error::RemoteCachingDisabledForTeam),
        CachingStatus::OverLimit => return Err(Error::UsageLimitExceeded),
        CachingStatus::Paused => return Err(Error::SpendingPaused),
    }

    let http_cache = HTTPCache::new(
        api_client,
        &opts.cache_opts,
        base.repo_root.clone(),
        api_auth,
        None,
        LazyScmState::resolved(None),
    )?;

    let source = Source::turbo(Subsystem::Cache);
    let mut failures = 0usize;
    for (hash, artifact) in &artifacts {
        match http_cache.put_local_artifact(hash, artifact).await {
            Ok(()) => {
                turborepo_log::info(source.clone(), format!("Pushed {hash}")).emit();
            }
            Err(err) => {
                failures += 1;
                let message = match err {
                    CacheError::TimeoutError(_) => format!(
                        "Failed to push {hash}: upload timed out{}. Retry with a larger \
                         `--upload-timeout <SECONDS>`, or `--upload-timeout 0` to disable the \
                         timeout.",
                        upload_timeout_suffix(base)
                    ),
                    err => format!("Failed to push {hash}: {err}"),
                };
                turborepo_log::error(source.clone(), message).emit();
            }
        }
    }

    let pushed = artifacts.len() - failures;
    let summary = format!(
        "Pushed {pushed} of {} artifact{} to the Remote Cache",
        artifacts.len(),
        if artifacts.len() == 1 { "" } else { "s" }
    );
    if failures == 0 {
        turborepo_log::info(source, summary).emit();
    } else {
        turborepo_log::error(source, summary).emit();
    }
    turborepo_log::flush();

    Ok(if failures == 0 { 0 } else { 1 })
}

/// Describes the effective upload deadline, mirroring how the API client
/// falls back from the upload timeout to the general request timeout.
fn upload_timeout_suffix(base: &CommandBase) -> String {
    let client_opts = &base.opts.api_client_opts;
    match (client_opts.upload_timeout, client_opts.timeout) {
        (0, 0) => String::new(),
        (0, timeout) => format!(" after {timeout}s"),
        (upload_timeout, _) => format!(" after {upload_timeout}s"),
    }
}
