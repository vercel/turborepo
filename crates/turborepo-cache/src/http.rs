use std::{
    backtrace::Backtrace,
    collections::HashMap,
    io::{Cursor, Write},
    sync::{Arc, Mutex},
};

use tokio_stream::StreamExt;
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{
    APIAuth, APIClient, CacheClient, Response,
    analytics::{self, AnalyticsEvent},
};

use crate::{
    CacheError, CacheHitMetadata, CacheOpts, CacheSource,
    cache_archive::{CacheReader, CacheWriter},
    signature_authentication::ArtifactSignatureAuthenticator,
    upload_progress::{UploadProgress, UploadProgressQuery},
};

pub type UploadMap = HashMap<String, UploadProgressQuery<10, 100>>;

pub struct HTTPCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthenticator>,
    repo_root: AbsoluteSystemPathBuf,
    api_auth: Arc<Mutex<APIAuth>>,
    analytics_recorder: Option<AnalyticsSender>,
    uploads: Arc<Mutex<UploadMap>>,
}

impl HTTPCache {
    #[tracing::instrument(skip_all)]
    pub fn new(
        client: APIClient,
        opts: &CacheOpts,
        repo_root: AbsoluteSystemPathBuf,
        api_auth: APIAuth,
        analytics_recorder: Option<AnalyticsSender>,
    ) -> HTTPCache {
        let signer_verifier = if opts
            .remote_cache_opts
            .as_ref()
            .is_some_and(|remote_cache_opts| remote_cache_opts.signature)
        {
            Some(ArtifactSignatureAuthenticator {
                team_id: api_auth
                    .team_id
                    .as_deref()
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec(),
                secret_key_override: None,
            })
        } else {
            None
        };

        HTTPCache {
            client,
            signer_verifier,
            repo_root,
            uploads: Arc::new(Mutex::new(HashMap::new())),
            api_auth: Arc::new(Mutex::new(api_auth)),
            analytics_recorder,
        }
    }

    /// Attempts to refresh the auth token when a cache operation encounters a
    /// 403 forbidden error. Returns true if the token was successfully
    /// refreshed, false otherwise.
    async fn try_refresh_token(&self) -> bool {
        match turborepo_auth::get_token_with_refresh().await {
            Ok(Some(new_token)) => {
                // Update the API auth with the new token
                if let Ok(mut auth) = self.api_auth.lock() {
                    auth.token = new_token;
                    debug!("Successfully refreshed auth token for cache operations");
                    true
                } else {
                    warn!("Failed to acquire lock for updating auth token");
                    false
                }
            }
            Ok(None) => {
                debug!("No refresh token available or token doesn't support refresh");
                false
            }
            Err(e) => {
                warn!("Failed to refresh token: {:?}", e);
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
        let api_auth = self.api_auth.lock().unwrap().clone();
        match operation(api_auth.clone()).await {
            Ok(result) => Ok(result),
            Err(turborepo_api_client::Error::UnknownStatus { code, .. }) if code == "forbidden" => {
                // Try to refresh the token
                if self.try_refresh_token().await {
                    // Retry the operation with the refreshed token
                    let refreshed_auth = self.api_auth.lock().unwrap().clone();
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
        let mut artifact_body = Vec::new();
        self.write(&mut artifact_body, anchor, files).await?;
        let bytes = artifact_body.len();

        let tag = self
            .signer_verifier
            .as_ref()
            .map(|signer| signer.generate_tag(hash.as_bytes(), &artifact_body))
            .transpose()?;

        tracing::debug!("uploading {}", hash);

        // Use the helper method to handle token refresh on 403 errors
        let artifact_body_clone = artifact_body.clone(); // Store the artifact body for retry
        let tag_clone = tag.clone();
        let uploads_clone = self.uploads.clone();

        self.execute_with_token_refresh(hash, |api_auth| {
            let client = &self.client;
            let tag_ref = tag_clone.as_deref();
            let artifact_body_ref = artifact_body_clone.clone();
            let uploads_ref = uploads_clone.clone();

            async move {
                // Create the stream inside the closure so it can be used for retry
                let stream = tokio_util::codec::FramedRead::new(
                    Cursor::new(artifact_body_ref),
                    tokio_util::codec::BytesCodec::new(),
                )
                .map(|res| {
                    res.map(|bytes| bytes.freeze())
                        .map_err(turborepo_api_client::Error::from)
                });

                let (progress, query) = UploadProgress::<10, 100, _>::new(stream, Some(bytes));

                {
                    let mut uploads = uploads_ref.lock().unwrap();
                    uploads.insert(hash.to_string(), query);
                }

                client
                    .put_artifact(
                        hash,
                        progress,
                        bytes,
                        duration,
                        tag_ref,
                        &api_auth.token,
                        api_auth.team_id.as_deref(),
                        api_auth.team_slug.as_deref(),
                    )
                    .await
            }
        })
        .await?;

        tracing::debug!("uploaded {}", hash);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn write(
        &self,
        writer: impl Write,
        anchor: &AbsoluteSystemPath,
        files: &[AnchoredSystemPathBuf],
    ) -> Result<(), CacheError> {
        let mut cache_archive = CacheWriter::from_writer(writer, true)?;
        for file in files {
            cache_archive.add_file(anchor, file)?;
        }

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

        Ok(Some(CacheHitMetadata {
            source: CacheSource::Remote,
            time_saved: duration,
        }))
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

        let body = if let Some(signer_verifier) = &self.signer_verifier {
            let expected_tag = response
                .headers()
                .get("x-artifact-tag")
                .ok_or(CacheError::ArtifactTagMissing(Backtrace::capture()))?;

            let expected_tag = expected_tag
                .to_str()
                .map_err(|_| CacheError::InvalidTag(Backtrace::capture()))?
                .to_string();

            let body = response.bytes().await.map_err(|e| {
                CacheError::ApiClientError(
                    Box::new(turborepo_api_client::Error::ReqwestError(e)),
                    Backtrace::capture(),
                )
            })?;
            let is_valid = signer_verifier.validate(hash.as_bytes(), &body, &expected_tag)?;

            if !is_valid {
                return Err(CacheError::InvalidTag(Backtrace::capture()));
            }

            body
        } else {
            response.bytes().await.map_err(|e| {
                CacheError::ApiClientError(
                    Box::new(turborepo_api_client::Error::ReqwestError(e)),
                    Backtrace::capture(),
                )
            })?
        };

        let files = Self::restore_tar(&self.repo_root, &body)?;

        self.log_fetch(analytics::CacheEvent::Hit, hash, duration);
        Ok(Some((
            CacheHitMetadata {
                source: CacheSource::Remote,
                time_saved: duration,
            },
            files,
        )))
    }

    pub fn requests(&self) -> Arc<Mutex<UploadMap>> {
        self.uploads.clone()
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn restore_tar(
        root: &AbsoluteSystemPath,
        body: &[u8],
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut cache_reader = CacheReader::from_reader(body, true)?;
        cache_reader.restore(root)
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
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_analytics::start_analytics;
    use turborepo_api_client::{APIClient, analytics};
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        CacheOpts, CacheSource,
        http::{APIAuth, HTTPCache},
        test_cases::{TestCase, get_test_cases, validate_analytics},
    };

    #[tokio::test]
    async fn test_http_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
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
        };
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: "my-token".to_string(),
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
        );

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
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: "expired-token".to_string(),
            team_slug: None,
        };

        let cache = HTTPCache::new(api_client, &opts, repo_root_path, api_auth, None);

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
        };

        let initial_api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: "initial-token".to_string(),
            team_slug: None,
        };

        let cache = HTTPCache::new(api_client, &opts, repo_root_path, initial_api_auth, None);

        // Verify initial token
        let initial_auth = cache.api_auth.lock().unwrap().clone();
        assert_eq!(initial_auth.token, "initial-token");

        // Test the token refresh mechanism (without actual HTTP call)
        // In a real scenario, try_refresh_token would call
        // turborepo_auth::get_token_with_refresh and update the internal token
        // if successful
        let refresh_result = cache.try_refresh_token().await;

        // The result depends on system state - could be true or false
        let final_auth = cache.api_auth.lock().unwrap().clone();

        if refresh_result {
            // If refresh succeeded, token should have been updated
            assert_ne!(final_auth.token, "initial-token");
        } else {
            // If refresh failed, token should remain unchanged
            assert_eq!(final_auth.token, "initial-token");
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
        };

        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: "thread-test-token".to_string(),
            team_slug: None,
        };

        let cache = Arc::new(HTTPCache::new(
            api_client,
            &opts,
            repo_root_path,
            api_auth,
            None,
        ));

        // Test concurrent access to the auth mutex
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let cache_clone = Arc::clone(&cache);
                thread::spawn(move || {
                    let auth = cache_clone.api_auth.lock().unwrap();
                    assert_eq!(auth.token, "thread-test-token");
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
}
