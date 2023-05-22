use std::{backtrace::Backtrace, io::Write, sync::Mutex};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_api_client::APIClient;

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    signature_authentication::ArtifactSignatureAuthenticator,
    CacheError,
};

pub struct HttpCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthenticator>,
    repo_root: AbsoluteSystemPathBuf,
}

impl HttpCache {
    pub fn new(
        client: APIClient,
        signer_verifier: Option<ArtifactSignatureAuthenticator>,
        repo_root: AbsoluteSystemPathBuf,
    ) -> HttpCache {
        HttpCache {
            client,
            signer_verifier,
            repo_root,
        }
    }

    pub async fn put(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: String,
        duration: u64,
        files: Vec<AnchoredSystemPathBuf>,
        ci_constant: Option<&str>,
    ) -> Result<(), CacheError> {
        let mut artifact_body = Vec::new();
        self.write(&mut artifact_body, anchor, files).await?;

        let tag = self
            .signer_verifier
            .map(|signer| signer.generate_tag(&hash, &artifact_body))
            .transpose()?;

        self.client
            .put_artifact(&hash, &artifact_body, duration, tag, ci_constant)
            .await?;

        Ok(())
    }

    async fn write(
        &self,
        writer: impl Write,
        anchor: &AbsoluteSystemPath,
        files: Vec<AnchoredSystemPathBuf>,
    ) -> Result<(), CacheError> {
        let mut cache_archive = CacheWriter::from_writer(writer)?;
        for file in files {
            cache_archive.add_file(anchor, file.as_anchored_path())?;
        }

        Ok(())
    }

    pub async fn retrieve(
        &self,
        hash: &str,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
        use_preflight: bool,
    ) -> Result<(Vec<AnchoredSystemPathBuf>, u64), CacheError> {
        let response = self
            .client
            .fetch_artifact(hash, token, team_id, team_slug, use_preflight)
            .await?;

        let duration = if let Some(duration) = response.headers().get("x-artifact-duration") {
            let duration = duration
                .to_str()
                .map_err(|_| CacheError::InvalidDuration(Backtrace::capture()))?;
            duration
                .parse::<u64>()
                .map_err(|_| CacheError::InvalidDuration(Backtrace::capture()))?
        } else {
            0
        };

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
                    turborepo_api_client::Error::ReqwestError(e),
                    Backtrace::capture(),
                )
            })?;
            let is_valid = signer_verifier.validate(hash, &body, &expected_tag)?;

            if !is_valid {
                return Err(CacheError::InvalidTag(Backtrace::capture()));
            }

            body
        } else {
            response.bytes().await.map_err(|e| {
                CacheError::ApiClientError(
                    turborepo_api_client::Error::ReqwestError(e),
                    Backtrace::capture(),
                )
            })?
        };

        let files = Self::restore_tar(self.repo_root.as_absolute_path(), &body)?;

        Ok((files, duration))
    }

    pub(crate) fn restore_tar(
        root: &AbsoluteSystemPath,
        body: &[u8],
    ) -> Result<Vec<AnchoredSystemPathBuf>, CacheError> {
        let mut cache_reader = CacheReader::from_reader(body, true)?;
        cache_reader.restore(root)
    }
}
