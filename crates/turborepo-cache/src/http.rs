use std::{backtrace::Backtrace, io::Write};

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{
    analytics, analytics::AnalyticsEvent, APIAuth, APIClient, Client, Response,
};

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    signature_authentication::ArtifactSignatureAuthenticator,
    CacheError, CacheHitMetadata, CacheOpts, CacheSource,
};

pub struct HTTPCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthenticator>,
    repo_root: AbsoluteSystemPathBuf,
    api_auth: APIAuth,
    analytics_recorder: Option<AnalyticsSender>,
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
            .map_or(false, |remote_cache_opts| remote_cache_opts.signature)
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
            api_auth,
            analytics_recorder,
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

        let tag = self
            .signer_verifier
            .as_ref()
            .map(|signer| signer.generate_tag(hash.as_bytes(), &artifact_body))
            .transpose()?;

        self.client
            .put_artifact(
                hash,
                &artifact_body,
                duration,
                tag.as_deref(),
                &self.api_auth.token,
                self.api_auth.team_id.as_deref(),
                self.api_auth.team_slug.as_deref(),
            )
            .await?;

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
        let Some(response) = self
            .client
            .artifact_exists(
                hash,
                &self.api_auth.token,
                self.api_auth.team_id.as_deref(),
                self.api_auth.team_slug.as_deref(),
            )
            .await?
        else {
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
        let Some(response) = self
            .client
            .fetch_artifact(
                hash,
                &self.api_auth.token,
                self.api_auth.team_id.as_deref(),
                self.api_auth.team_slug.as_deref(),
            )
            .await?
        else {
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

    #[tracing::instrument(skip_all)]
    pub(crate) fn restore_tar(
        root: &AbsoluteSystemPath,
        body: &[u8],
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut cache_reader = CacheReader::from_reader(body, true)?;
        cache_reader.restore(root)
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_analytics::start_analytics;
    use turborepo_api_client::{analytics, APIClient};
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        http::{APIAuth, HTTPCache},
        test_cases::{get_test_cases, validate_analytics, TestCase},
        CacheOpts, CacheSource,
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

        let api_client = APIClient::new(format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let opts = CacheOpts::default();
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
}
