use std::{backtrace::Backtrace, io::Write};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_api_client::{APIClient, Response};

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    signature_authentication::ArtifactSignatureAuthenticator,
    CacheError, CacheOpts, CacheResponse, CacheSource,
};

pub struct HTTPCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthenticator>,
    repo_root: AbsoluteSystemPathBuf,
    token: String,
}

pub struct APIAuth {
    pub team_id: String,
    pub token: String,
}

impl HTTPCache {
    pub fn new(
        client: APIClient,
        opts: &CacheOpts,
        repo_root: AbsoluteSystemPathBuf,
        api_auth: APIAuth,
    ) -> HTTPCache {
        let signer_verifier = if opts
            .remote_cache_opts
            .as_ref()
            .map_or(false, |remote_cache_opts| remote_cache_opts.signature)
        {
            Some(ArtifactSignatureAuthenticator {
                team_id: api_auth.team_id.as_bytes().to_vec(),
                secret_key_override: None,
            })
        } else {
            None
        };

        HTTPCache {
            client,
            signer_verifier,
            repo_root,
            token: api_auth.token,
        }
    }

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
            .put_artifact(hash, &artifact_body, duration, tag.as_deref(), &self.token)
            .await?;

        Ok(())
    }

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

    pub async fn exists(
        &self,
        hash: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<CacheResponse, CacheError> {
        let response = self
            .client
            .artifact_exists(hash, &self.token, team_id, team_slug)
            .await?;

        let duration = Self::get_duration_from_response(&response)?;

        Ok(CacheResponse {
            source: CacheSource::Remote,
            time_saved: duration,
        })
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

    pub async fn fetch(
        &self,
        hash: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<(CacheResponse, Vec<AnchoredSystemPathBuf>), CacheError> {
        let response = self
            .client
            .fetch_artifact(hash, &self.token, team_id, team_slug)
            .await?;

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

        Ok((
            CacheResponse {
                source: CacheSource::Remote,
                time_saved: duration,
            },
            files,
        ))
    }

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
    use turborepo_api_client::APIClient;
    use vercel_api_mock::start_test_server;

    use crate::{
        http::{APIAuth, HTTPCache},
        test_cases::{get_test_cases, TestCase},
        CacheOpts, CacheSource,
    };

    #[tokio::test]
    async fn test_http_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));

        try_join_all(
            get_test_cases()
                .into_iter()
                .map(|test_case| round_trip_test(test_case, port)),
        )
        .await?;

        handle.abort();
        Ok(())
    }

    async fn round_trip_test(test_case: TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let TestCase {
            hash,
            files,
            duration,
        } = test_case;

        let api_client = APIClient::new(&format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let opts = CacheOpts::default();
        let api_auth = APIAuth {
            team_id: "my-team".to_string(),
            token: "my-token".to_string(),
        };

        let cache = HTTPCache::new(api_client, &opts, repo_root_path.to_owned(), api_auth);

        let anchored_files: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        cache
            .put(&repo_root_path, hash, &anchored_files, duration)
            .await?;

        let cache_response = cache.exists(hash, "", None).await?;

        assert_eq!(cache_response.time_saved, duration);
        assert_eq!(cache_response.source, CacheSource::Remote);

        let (cache_response, received_files) = cache.fetch(hash, "", None).await?;
        assert_eq!(cache_response.time_saved, duration);

        for (test_file, received_file) in files.iter().zip(received_files) {
            assert_eq!(received_file, test_file.path);
            let file_path = repo_root_path.resolve(&received_file);
            assert_eq!(std::fs::read_to_string(file_path)?, test_file.contents);
        }

        Ok(())
    }
}
