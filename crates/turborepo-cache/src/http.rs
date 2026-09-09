use std::{
    backtrace::Backtrace,
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};

use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{
    APIAuth, APIClient, CacheClient, Response,
    analytics::{self, AnalyticsEvent},
};
use turborepo_types::SecretString;

use crate::{
    CacheError, CacheHitMetadata, CacheOpts, CacheSource, LazyScmState,
    artifact_body::{ARTIFACT_MEMORY_THRESHOLD, ArtifactBody},
    cache_archive::CacheReader,
    signature_authentication::{ArtifactSignatureAuthenticator, SignatureError},
    upload_progress::{UploadProgress, UploadProgressQuery},
};

pub type UploadMap = HashMap<String, UploadProgressQuery<10, 100>>;

fn replace_api_auth_token(api_auth: &mut APIAuth, token: SecretString) -> bool {
    if api_auth.token.expose() == token.expose() {
        return false;
    }

    api_auth.token = token;
    true
}

pub struct HTTPCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthenticator>,
    repo_root: AbsoluteSystemPathBuf,
    api_auth: Arc<Mutex<APIAuth>>,
    analytics_recorder: Option<AnalyticsSender>,
    uploads: Arc<Mutex<UploadMap>>,
    scm_state: LazyScmState,
}

impl HTTPCache {
    #[tracing::instrument(skip_all)]
    pub fn new(
        client: APIClient,
        opts: &CacheOpts,
        repo_root: AbsoluteSystemPathBuf,
        api_auth: APIAuth,
        analytics_recorder: Option<AnalyticsSender>,
        scm_state: LazyScmState,
    ) -> Result<HTTPCache, CacheError> {
        let remote_cache_opts = opts.remote_cache_opts.as_ref();
        let wants_signature = remote_cache_opts.is_some_and(|o| o.signature);
        let enforce_key_length =
            remote_cache_opts.is_some_and(|o| o.enforce_signature_key_length());

        let signer_verifier = if wants_signature {
            let authenticator = ArtifactSignatureAuthenticator {
                team_id: api_auth
                    .team_id
                    .as_deref()
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec(),
                secret_key_override: None,
            };

            if let Err(e) = authenticator.validate_key_length()
                && matches!(e, SignatureError::SignatureKeyTooShort { .. })
            {
                if enforce_key_length {
                    return Err(e.into());
                }
                warn!(
                    "{e} This will become a fatal error in the next major version of Turborepo. \
                     Enable `futureFlags.longerSignatureKey` in turbo.json to enforce this now."
                );
            }

            Some(authenticator)
        } else {
            None
        };

        Ok(HTTPCache {
            client,
            signer_verifier,
            repo_root,
            uploads: Arc::new(Mutex::new(HashMap::new())),
            api_auth: Arc::new(Mutex::new(api_auth)),
            analytics_recorder,
            scm_state,
        })
    }

    /// Attempts to refresh the auth token when a cache operation encounters a
    /// 403 forbidden error. Returns true if the token was successfully
    /// refreshed, false otherwise.
    async fn try_refresh_token(&self) -> bool {
        let current_token = match self.api_auth.lock() {
            Ok(auth) => auth.token.clone(),
            Err(_) => {
                warn!("Failed to acquire lock for reading auth token");
                return false;
            }
        };

        match turborepo_auth::recover_token_after_forbidden(&current_token).await {
            Ok(Some(new_token)) => {
                // Update the API auth with the new token
                if let Ok(mut auth) = self.api_auth.lock() {
                    if replace_api_auth_token(&mut auth, new_token) {
                        debug!("Successfully recovered auth token for cache operations");
                        true
                    } else {
                        debug!("Recovered auth token matched the current token; skipping retry");
                        false
                    }
                } else {
                    warn!("Failed to acquire lock for updating auth token");
                    false
                }
            }
            Ok(None) => {
                debug!("No replacement token available after forbidden response");
                false
            }
            Err(e) => {
                warn!("Failed to recover token after forbidden response: {:?}", e);
                false
            }
        }
    }

    /// Helper method to execute a cache operation with automatic token refresh
    /// on 403 errors.
    async fn execute_with_token_refresh<T, F, Fut>(
        &self,
        hash: &str,
        operation: F,
    ) -> Result<T, CacheError>
    where
        F: Fn(APIAuth) -> Fut,
        Fut: std::future::Future<Output = Result<T, turborepo_api_client::Error>>,
    {
        // Try the operation with the current token
        let api_auth = self
            .api_auth
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match operation(api_auth.clone()).await {
            Ok(result) => Ok(result),
            Err(turborepo_api_client::Error::UnknownStatus { code, .. }) if code == "forbidden" => {
                // Try to refresh the token
                if self.try_refresh_token().await {
                    // Retry the operation with the refreshed token
                    let refreshed_auth = self
                        .api_auth
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone();
                    operation(refreshed_auth)
                        .await
                        .map_err(|err| Self::convert_api_error(hash, err))
                } else {
                    // Token refresh failed, return the original error
                    Err(CacheError::ForbiddenRemoteCacheWrite)
                }
            }
            Err(e) => Err(Self::convert_api_error(hash, e)),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn put(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        files: &[AnchoredSystemPathBuf],
        duration: u64,
    ) -> Result<(), CacheError> {
        // Spool the compressed artifact: small artifacts stay in memory while
        // large ones roll to an anonymous temporary file, so retained memory
        // is bounded regardless of artifact size.
        let body = ArtifactBody::from_files(anchor, files)?;
        self.put_body(hash, body, duration).await
    }

    /// Uploads an already-built archive. Shared with the local cache so a
    /// combined local+remote write builds and compresses the artifact exactly
    /// once.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn put_body(
        &self,
        hash: &str,
        body: ArtifactBody,
        duration: u64,
    ) -> Result<(), CacheError> {
        let body = Arc::new(body);
        let body_len = body.len();

        let tag = self
            .signer_verifier
            .as_ref()
            .map(|signer| body.generate_tag(signer, hash))
            .transpose()?;

        let resolved_scm = self.scm_state.get_resolved().await;
        let sha = resolved_scm.and_then(|s| s.sha.clone());
        let dirty_hash = resolved_scm.and_then(|s| s.dirty_hash.clone());

        tracing::debug!("uploading {}", hash);

        let tag_clone = tag.clone();
        let uploads_clone = self.uploads.clone();
        let sha_clone = sha.clone();
        let dirty_hash_clone = dirty_hash.clone();

        self.execute_with_token_refresh(hash, |api_auth| {
            let client = &self.client;
            let tag_ref = tag_clone.as_deref();
            let body_ref = body.clone();
            let uploads_ref = uploads_clone.clone();
            let sha_ref = sha_clone.clone();
            let dirty_hash_ref = dirty_hash_clone.clone();

            async move {
                // Each attempt gets a fresh bounded stream over the same
                // spooled bytes, so retries send identical content.
                let stream = body_ref.stream()?;

                let (progress, query) = UploadProgress::<10, 100, _>::new(stream, Some(body_len));

                {
                    let mut uploads = uploads_ref
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    uploads.insert(hash.to_string(), query);
                }

                client
                    .put_artifact(
                        hash,
                        progress,
                        body_len,
                        duration,
                        tag_ref,
                        &api_auth.token,
                        api_auth.team_id.as_deref(),
                        api_auth.team_slug.as_deref(),
                        sha_ref.as_deref(),
                        dirty_hash_ref.as_deref(),
                    )
                    .await
            }
        })
        .await?;

        tracing::debug!("uploaded {}", hash);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub async fn exists(&self, hash: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        let response = self
            .execute_with_token_refresh(hash, |api_auth| {
                let client = &self.client;
                async move {
                    client
                        .artifact_exists(
                            hash,
                            &api_auth.token,
                            api_auth.team_id.as_deref(),
                            api_auth.team_slug.as_deref(),
                        )
                        .await
                }
            })
            .await?;

        let Some(response) = response else {
            return Ok(None);
        };

        let duration = Self::get_duration_from_response(&response)?;
        let sha = Self::get_header_string(&response, "x-artifact-sha");
        let dirty_hash = Self::get_header_string(&response, "x-artifact-dirty-hash");

        Ok(Some(CacheHitMetadata {
            source: CacheSource::Remote,
            time_saved: duration,
            sha,
            dirty_hash,
        }))
    }

    fn get_header_string(response: &Response, name: &str) -> Option<String> {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    fn get_duration_from_response(response: &Response) -> Result<u64, CacheError> {
        if let Some(duration_value) = response.headers().get("x-artifact-duration") {
            let duration = duration_value
                .to_str()
                .map_err(|_| CacheError::InvalidDuration(Backtrace::capture()))?;

            duration
                .parse::<u64>()
                .map_err(|_| CacheError::InvalidDuration(Backtrace::capture()))
        } else {
            Ok(0)
        }
    }

    fn log_fetch(&self, event: analytics::CacheEvent, hash: &str, duration: u64) {
        // If analytics fails to record, it's not worth failing the cache
        if let Some(analytics_recorder) = &self.analytics_recorder {
            let analytics_event = AnalyticsEvent {
                session_id: None,
                source: analytics::CacheSource::Remote,
                event,
                hash: hash.to_string(),
                duration,
            };
            debug!("logging fetch: {analytics_event:?}");
            let _ = analytics_recorder.send(analytics_event);
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn fetch(
        &self,
        hash: &str,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
        Ok(self
            .fetch_with_archive(hash)
            .await?
            .map(|(metadata, files, _body)| (metadata, files)))
    }

    /// Fetches and restores the artifact, also returning the verified archive
    /// bytes so a lower-priority local cache can install the exact same bytes
    /// without re-encoding them. The signature check completes before both
    /// the restore and the handoff, so a rejected artifact is never restored
    /// or installed locally.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn fetch_with_archive(
        &self,
        hash: &str,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>, ArtifactBody)>, CacheError>
    {
        let response = self
            .execute_with_token_refresh(hash, |api_auth| {
                let client = &self.client;
                async move {
                    client
                        .fetch_artifact(
                            hash,
                            &api_auth.token,
                            api_auth.team_id.as_deref(),
                            api_auth.team_slug.as_deref(),
                        )
                        .await
                }
            })
            .await?;

        let Some(response) = response else {
            self.log_fetch(analytics::CacheEvent::Miss, hash, 0);
            return Ok(None);
        };

        let duration = Self::get_duration_from_response(&response)?;
        let sha = Self::get_header_string(&response, "x-artifact-sha");
        let dirty_hash = Self::get_header_string(&response, "x-artifact-dirty-hash");

        let expected_tag = if self.signer_verifier.is_some() {
            let tag = response
                .headers()
                .get("x-artifact-tag")
                .ok_or(CacheError::ArtifactTagMissing(Backtrace::capture()))?;

            Some(
                tag.to_str()
                    .map_err(|_| CacheError::InvalidTag(Backtrace::capture()))?
                    .to_string(),
            )
        } else {
            None
        };

        // Stream the response into a spool instead of collecting the whole
        // body in memory. Small artifacts stay in memory; large ones roll to
        // an anonymous temporary file that is cleaned up on drop. When the
        // Content-Length is known (the common case) the signature is computed
        // incrementally as chunks arrive.
        let mut streaming_tag = match (&self.signer_verifier, response.content_length()) {
            (Some(signer), Some(len)) => Some(signer.start_streaming_tag(hash.as_bytes(), len)?),
            _ => None,
        };
        let mut spool = tempfile::spooled_tempfile(ARTIFACT_MEMORY_THRESHOLD);
        let mut body_len: u64 = 0;
        {
            use tokio_stream::StreamExt;
            let mut body_stream = response.bytes_stream();
            while let Some(chunk) = body_stream.next().await {
                let chunk = chunk.map_err(|e| {
                    CacheError::ApiClientError(
                        Box::new(turborepo_api_client::Error::ReqwestError(e)),
                        Backtrace::capture(),
                    )
                })?;
                if let Some(tag) = &mut streaming_tag {
                    tag.update(&chunk);
                }
                spool.write_all(&chunk)?;
                body_len += chunk.len() as u64;
            }
        }

        // Verify the signature before any extraction; a rejected artifact is
        // dropped with the spool and nothing is restored.
        if let (Some(signer_verifier), Some(expected_tag)) = (&self.signer_verifier, &expected_tag)
        {
            let is_valid = match streaming_tag {
                Some(tag) => tag.verify(expected_tag)?,
                // The body length was not known up front, so verify from the
                // spooled bytes now that the total is known.
                None => {
                    spool.seek(SeekFrom::Start(0))?;
                    signer_verifier.validate_reader(
                        hash.as_bytes(),
                        &mut spool,
                        body_len,
                        expected_tag,
                    )?
                }
            };

            if !is_valid {
                return Err(CacheError::InvalidTag(Backtrace::capture()));
            }
        }

        spool.seek(SeekFrom::Start(0))?;
        let body = ArtifactBody::from_spool(spool)?;
        let files = Self::restore_tar(&self.repo_root, body.reader()?)?;

        self.log_fetch(analytics::CacheEvent::Hit, hash, duration);
        Ok(Some((
            CacheHitMetadata {
                source: CacheSource::Remote,
                time_saved: duration,
                sha,
                dirty_hash,
            },
            files,
            body,
        )))
    }

    pub fn requests(&self) -> Arc<Mutex<UploadMap>> {
        self.uploads.clone()
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn restore_tar(
        root: &AbsoluteSystemPath,
        body: impl Read,
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut cache_reader = CacheReader::from_reader(body, true)?;
        let (files, _manifest) = cache_reader.restore(root, None)?;
        Ok(files)
    }

    fn convert_api_error(hash: &str, err: turborepo_api_client::Error) -> CacheError {
        match err {
            turborepo_api_client::Error::ReqwestError(e) if e.is_timeout() => {
                CacheError::TimeoutError(hash.to_string())
            }
            turborepo_api_client::Error::ReqwestError(e) if e.is_connect() => {
                CacheError::ConnectError
            }
            turborepo_api_client::Error::UnknownStatus { code, .. } if code == "forbidden" => {
                CacheError::ForbiddenRemoteCacheWrite
            }
            e => e.into(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{backtrace::Backtrace, time::Duration};

    use anyhow::Result;
    use futures::future::try_join_all;
    use insta::assert_snapshot;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_analytics::start_analytics;
    use turborepo_api_client::{APIClient, analytics};
    use turborepo_types::SecretString;
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        CacheOpts, CacheSource, LazyScmState,
        http::{APIAuth, HTTPCache},
        test_cases::{TestCase, get_test_cases, validate_analytics},
    };

    #[tokio::test]
    async fn test_http_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for the server to be ready (with timeout)
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start within timeout"))??;

        let test_cases = get_test_cases();

        try_join_all(
            test_cases
                .iter()
                .map(|test_case| round_trip_test(test_case, port)),
        )
        .await?;

        validate_analytics(&test_cases, analytics::CacheSource::Remote, port).await?;
        handle.abort();
        Ok(())
    }

    async fn round_trip_test(test_case: &TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = test_case.hash;
        let files = &test_case.files;
        let duration = test_case.duration;

        let api_client = APIClient::new(
            format!("http://localhost:{port}"),
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };
        let (analytics_recorder, analytics_handle) =
            start_analytics(api_auth.clone(), api_client.clone());

        let cache = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path.to_owned(),
            api_auth,
            Some(analytics_recorder),
            LazyScmState::resolved(None),
        )
        .unwrap();

        // Should be a cache miss at first
        let miss = cache.fetch(hash).await?;
        assert!(miss.is_none());

        let anchored_files: Vec<_> = files.iter().map(|f| f.path().to_owned()).collect();
        cache
            .put(&repo_root_path, hash, &anchored_files, duration)
            .await?;

        let cache_response = cache.exists(hash).await?.unwrap();

        assert_eq!(cache_response.time_saved, duration);
        assert_eq!(cache_response.source, CacheSource::Remote);

        let (cache_response, received_files) = cache.fetch(hash).await?.unwrap();

        assert_eq!(cache_response.time_saved, duration);

        for (test_file, received_file) in files.iter().zip(received_files) {
            assert_eq!(&*received_file, test_file.path());
            let file_path = repo_root_path.resolve(&received_file);
            if let Some(contents) = test_file.contents() {
                assert_eq!(std::fs::read_to_string(file_path)?, contents);
            } else {
                assert!(file_path.exists());
            }
        }

        analytics_handle.close_with_timeout().await;

        Ok(())
    }

    /// An artifact larger than the in-memory threshold must round-trip through
    /// the disk-spooled upload and download paths byte-for-byte.
    #[tokio::test]
    async fn test_http_cache_large_artifact_spools_to_disk() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start within timeout"))??;

        // 16 MiB of incompressible pseudo-random data, so the compressed
        // artifact exceeds ARTIFACT_MEMORY_THRESHOLD and rolls to disk.
        let mut contents = Vec::with_capacity(16 * 1024 * 1024);
        let mut state: u64 = 0x9E3779B97F4A7C15;
        while contents.len() < 16 * 1024 * 1024 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            contents.extend_from_slice(&state.to_le_bytes());
        }

        let put_root = tempdir()?;
        let put_root_path = AbsoluteSystemPathBuf::try_from(put_root.path())?;
        let file_path = AnchoredSystemPathBuf::from_raw("big.bin")?;
        std::fs::write(put_root_path.resolve(&file_path), &contents)?;

        let hash = "large-artifact-hash";
        let duration = 42;
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };

        let make_cache = |root: &AbsoluteSystemPathBuf| {
            HTTPCache::new(
                APIClient::new(
                    format!("http://localhost:{port}"),
                    Some(Duration::from_secs(200)),
                    None,
                    "2.0.0",
                    true,
                )
                .unwrap(),
                &opts,
                root.to_owned(),
                api_auth.clone(),
                None,
                LazyScmState::resolved(None),
            )
            .unwrap()
        };

        let put_cache = make_cache(&put_root_path);
        put_cache
            .put(
                &put_root_path,
                hash,
                std::slice::from_ref(&file_path),
                duration,
            )
            .await?;

        // Restore into a different root so the bytes must actually travel.
        let fetch_root = tempdir()?;
        let fetch_root_path = AbsoluteSystemPathBuf::try_from(fetch_root.path())?;
        let fetch_cache = make_cache(&fetch_root_path);

        let (metadata, received_files) = fetch_cache.fetch(hash).await?.unwrap();
        assert_eq!(metadata.time_saved, duration);
        assert_eq!(received_files.len(), 1);
        let restored = std::fs::read(fetch_root_path.resolve(&received_files[0]))?;
        assert_eq!(restored, contents);

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_http_cache_scm_metadata_round_trip() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start within timeout"))??;

        let test_case = &get_test_cases()[0];
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-scm", test_case.hash);

        let api_client = APIClient::new(
            format!("http://localhost:{port}"),
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        let scm_state = LazyScmState::resolved(Some(crate::CacheScmState {
            sha: Some("abc123def456".to_string()),
            dirty_hash: Some("dirty789".to_string()),
        }));

        let cache = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path.to_owned(),
            api_auth,
            None,
            scm_state,
        )
        .unwrap();

        let anchored_files: Vec<_> = test_case
            .files
            .iter()
            .map(|f| f.path().to_owned())
            .collect();
        cache
            .put(&repo_root_path, &hash, &anchored_files, test_case.duration)
            .await?;

        // Verify exists returns scm metadata
        let exists_response = cache.exists(&hash).await?.unwrap();
        assert_eq!(exists_response.source, CacheSource::Remote);
        assert_eq!(exists_response.sha.as_deref(), Some("abc123def456"));
        assert_eq!(exists_response.dirty_hash.as_deref(), Some("dirty789"));

        // Verify fetch returns scm metadata
        let (fetch_response, _files) = cache.fetch(&hash).await?.unwrap();
        assert_eq!(fetch_response.source, CacheSource::Remote);
        assert_eq!(fetch_response.sha.as_deref(), Some("abc123def456"));
        assert_eq!(fetch_response.dirty_hash.as_deref(), Some("dirty789"));

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_http_cache_no_scm_metadata() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start within timeout"))??;

        let test_case = &get_test_cases()[0];
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-no-scm", test_case.hash);

        let api_client = APIClient::new(
            format!("http://localhost:{port}"),
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        // No SCM state available
        let cache = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path.to_owned(),
            api_auth,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        let anchored_files: Vec<_> = test_case
            .files
            .iter()
            .map(|f| f.path().to_owned())
            .collect();
        cache
            .put(&repo_root_path, &hash, &anchored_files, test_case.duration)
            .await?;

        // Without SCM state, sha and dirty_hash should be None
        let exists_response = cache.exists(&hash).await?.unwrap();
        assert_eq!(exists_response.sha, None);
        assert_eq!(exists_response.dirty_hash, None);

        let (fetch_response, _files) = cache.fetch(&hash).await?.unwrap();
        assert_eq!(fetch_response.sha, None);
        assert_eq!(fetch_response.dirty_hash, None);

        handle.abort();
        Ok(())
    }

    #[test]
    fn test_forbidden_error() {
        let err = HTTPCache::convert_api_error(
            "hash",
            turborepo_api_client::Error::UnknownStatus {
                code: "forbidden".into(),
                message: "Not authorized".into(),
                backtrace: Backtrace::capture(),
            },
        );
        assert_snapshot!(err.to_string(), @"Insufficient permissions to write to remote cache. Please verify that your role has write access for Remote Cache Artifact at https://vercel.com/docs/accounts/team-members-and-roles/access-roles/team-level-roles?resource=Remote+Cache+Artifact");
    }

    #[test]
    fn test_unknown_status() {
        let err = HTTPCache::convert_api_error(
            "hash",
            turborepo_api_client::Error::UnknownStatus {
                code: "unknown".into(),
                message: "Special message".into(),
                backtrace: Backtrace::capture(),
            },
        );
        assert_snapshot!(err.to_string(), @"failed to contact remote cache: Unknown status unknown: Special message");
    }

    #[test]
    fn test_cache_disabled() {
        let err = HTTPCache::convert_api_error(
            "hash",
            turborepo_api_client::Error::CacheDisabled {
                status: turborepo_vercel_api::CachingStatus::Disabled,
                message: "Cache disabled".into(),
            },
        );
        assert_snapshot!(err.to_string(), @"failed to contact remote cache: Cache disabled");
    }

    #[tokio::test]
    async fn test_token_refresh_on_403() {
        // This test verifies that the HTTPCache can handle token refresh when
        // encountering 403 errors. Note: This is an integration test that would
        // need a mock server setup to fully verify the token refresh flow, but
        // the logic structure is tested through the build validation.
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("expired-token".to_string()),
            team_slug: None,
        };

        let cache = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        // Verify that the cache has the token refresh capability
        // The actual token refresh would be tested in integration tests with a proper
        // mock server. The vca_ prefix check is now handled in the auth layer.
        // The result depends on whether there are any tokens available in the system
        //
        // The result can be true or false depending on system state, but the method
        // should not panic. The test will fail if it does.
        cache.try_refresh_token().await;
    }

    #[tokio::test]
    async fn test_cache_token_update_after_refresh() {
        // Test that the cache properly updates its internal token after a successful
        // refresh
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };

        let initial_api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("initial-token".to_string()),
            team_slug: None,
        };

        let cache = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            initial_api_auth,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        // Verify initial token
        let initial_auth = cache.api_auth.lock().unwrap().clone();
        assert_eq!(initial_auth.token.expose(), "initial-token");

        // Test the token recovery mechanism (without actual HTTP call)
        // In a real scenario, try_refresh_token would call
        // turborepo_auth::recover_token_after_forbidden and update the internal
        // token if successful.
        let refresh_result = cache.try_refresh_token().await;

        // The result depends on system state - could be true or false
        let final_auth = cache.api_auth.lock().unwrap().clone();

        if refresh_result {
            // If refresh succeeded, token should have been updated
            assert_ne!(final_auth.token.expose(), "initial-token");
        } else {
            // If refresh failed, token should remain unchanged
            assert_eq!(final_auth.token.expose(), "initial-token");
        }
    }

    #[test]
    fn test_cache_auth_mutex_thread_safety() {
        // Test that the Arc<Mutex<APIAuth>> is properly thread-safe
        use std::{sync::Arc, thread};

        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();
        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: None,
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("thread-test-token".to_string()),
            team_slug: None,
        };

        let cache = Arc::new(
            HTTPCache::new(
                api_client,
                &opts,
                repo_root_path,
                api_auth,
                None,
                LazyScmState::resolved(None),
            )
            .unwrap(),
        );

        // Test concurrent access to the auth mutex
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let cache_clone = Arc::clone(&cache);
                thread::spawn(move || {
                    let auth = cache_clone.api_auth.lock().unwrap();
                    assert_eq!(auth.token.expose(), "thread-test-token");
                    assert_eq!(auth.team_id, Some("my-team".to_string()));
                    // Simulate some work
                    thread::sleep(std::time::Duration::from_millis(10));
                    format!("thread-{i}")
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.starts_with("thread-"));
        }
    }

    #[test]
    fn test_replace_api_auth_token_requires_a_new_token() {
        let mut api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("initial-token".to_string()),
            team_slug: None,
        };

        assert!(!super::replace_api_auth_token(
            &mut api_auth,
            SecretString::new("initial-token".to_string())
        ));
        assert_eq!(api_auth.token.expose(), "initial-token");

        assert!(super::replace_api_auth_token(
            &mut api_auth,
            SecretString::new("replacement-token".to_string())
        ));
        assert_eq!(api_auth.token.expose(), "replacement-token");
    }

    #[test]
    fn test_short_signature_key_rejected_when_enforced() {
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();

        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: Some(crate::RemoteCacheOpts::new(
                None, true, // signature enabled
                true, // enforce key length
            )),
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        // SAFETY: test-only, no other thread reads this env var concurrently
        unsafe {
            std::env::set_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY", "short");
        }
        let result = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
            LazyScmState::resolved(None),
        );
        unsafe {
            std::env::remove_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY");
        }

        let err = result.err().expect("expected an error for short key");
        assert_snapshot!(err.to_string(), @"artifact signature error");

        // Verify the source chain carries the detail
        let source = std::error::Error::source(&err).expect("should have a source error");
        assert_snapshot!(
            source.to_string(),
            @"TURBO_REMOTE_CACHE_SIGNATURE_KEY is too short (5 bytes). A minimum of 32 bytes is required for cryptographic strength."
        );
    }

    #[test]
    fn test_short_signature_key_accepted_without_enforcement() {
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();

        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: Some(crate::RemoteCacheOpts::new(
                None, true,  // signature enabled
                false, // enforcement OFF (no future flag)
            )),
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        // SAFETY: test-only, no other thread reads this env var concurrently
        unsafe {
            std::env::set_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY", "short");
        }
        let result = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
            LazyScmState::resolved(None),
        );
        unsafe {
            std::env::remove_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY");
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_signature_key_accepted_when_enforced() {
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();

        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: Some(crate::RemoteCacheOpts::new(
                None, true, // signature enabled
                true, // enforce key length
            )),
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        // SAFETY: test-only, no other thread reads this env var concurrently
        unsafe {
            std::env::set_var(
                "TURBO_REMOTE_CACHE_SIGNATURE_KEY",
                "this-key-is-at-least-32-bytes-!!", // exactly 32 bytes
            );
        }
        let result = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
            LazyScmState::resolved(None),
        );
        unsafe {
            std::env::remove_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY");
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_signature_key_not_rejected_when_enforced() {
        let repo_root = tempfile::tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let api_client = APIClient::new(
            "http://localhost:8000",
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            false,
        )
        .unwrap();

        let opts = CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: Default::default(),
            workers: 0,
            remote_cache_opts: Some(crate::RemoteCacheOpts::new(
                None, true, // signature enabled
                true, // enforce key length
            )),
            cache_max_age: None,
            cache_max_size: None,
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };

        // SAFETY: test-only, ensure env var is NOT set
        unsafe {
            std::env::remove_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY");
        }
        let result = HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
            LazyScmState::resolved(None),
        );

        // The flag should not force an error when no key exists at all.
        // It only enforces minimum length on keys that ARE set.
        assert!(result.is_ok());
    }
}
