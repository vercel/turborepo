use std::{backtrace::Backtrace, io::Write};

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
        hash: &str,
        files: Vec<AnchoredSystemPathBuf>,
        duration: u32,
        token: &str,
    ) -> Result<(), CacheError> {
        let mut artifact_body = Vec::new();
        self.write(&mut artifact_body, anchor, files).await?;

        let tag = self
            .signer_verifier
            .as_ref()
            .map(|signer| signer.generate_tag(hash.as_bytes(), &artifact_body))
            .transpose()?;

        self.client
            .put_artifact(hash, &artifact_body, duration, tag.as_deref(), token)
            .await?;

        Ok(())
    }

    async fn write(
        &self,
        writer: impl Write,
        anchor: &AbsoluteSystemPath,
        files: Vec<AnchoredSystemPathBuf>,
    ) -> Result<(), CacheError> {
        let mut cache_archive = CacheWriter::from_writer(writer, true)?;
        for file in files {
            cache_archive.add_file(anchor, &file)?;
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
            let is_valid = signer_verifier.validate(hash.as_bytes(), &body, &expected_tag)?;

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

        let files = Self::restore_tar(&self.repo_root, &body)?;

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

#[cfg(test)]
mod test {
    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_api_client::APIClient;

    use crate::http::HttpCache;

    const DEFAULT_API_URL: &str = "https://vercel.com/api";

    struct TestFile {
        path: AnchoredSystemPathBuf,
        contents: &'static str,
    }

    #[test_case(vec![
        TestFile {
            path: AnchoredSystemPathBuf::new("package.json"),
            contents: "{}"
        }
    ])]
    fn test_round_trip(files: Vec<TestFile>) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::try_from(repo_root.path())?;

        for file in &files {
            let file_path = repo_root_path.resolve(&file.path);
            std::fs::create_dir_all(file_path.parent().unwrap())?;
            std::fs::write(file_path, file.contents)?;
        }
        let api_client = APIClient::new(DEFAULT_API_URL, 200, "2.0.0", true)?;

        let cache = HttpCache::new(api_client, None, repo_root_path.to_owned());

        cache.put(
            &repo_root_path,
            "this-is-my-hash",
            files.iter().map(|f| f.path.clone()).collect(),
            0,
            "",
        )?;
    }
}
