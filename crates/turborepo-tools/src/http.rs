//! Streaming downloads with checksum verification.

use std::{path::Path, time::Duration};

use base64::Engine;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha224, Sha256, Sha512};
use tokio::io::AsyncWriteExt;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha224,
    Sha256,
    Sha512,
}

impl HashAlgorithm {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "sha224" => Some(Self::Sha224),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    fn digest_len(&self) -> usize {
        match self {
            Self::Sha224 => 28,
            Self::Sha256 => 32,
            Self::Sha512 => 64,
        }
    }
}

/// An expected digest for a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: HashAlgorithm,
    pub digest: Vec<u8>,
}

impl Checksum {
    pub fn sha256_hex(hex_digest: &str) -> Result<Self, Error> {
        Self::from_hex(HashAlgorithm::Sha256, hex_digest)
    }

    pub fn from_hex(algorithm: HashAlgorithm, hex_digest: &str) -> Result<Self, Error> {
        let hex_digest = hex_digest.trim();
        let digest = hex::decode(hex_digest).map_err(|err| Error::InvalidChecksum {
            value: hex_digest.to_string(),
            reason: err.to_string(),
        })?;
        Self::with_digest(algorithm, digest, hex_digest)
    }

    /// Parses a Subresource Integrity string such as `sha512-<base64>` as
    /// published by the npm registry in `dist.integrity`. Unsupported
    /// algorithms yield `Ok(None)` so callers can fall back.
    pub fn from_sri(sri: &str) -> Result<Option<Self>, Error> {
        // Integrity strings may list several hashes separated by whitespace.
        for entry in sri.split_whitespace() {
            let Some((algorithm, encoded)) = entry.split_once('-') else {
                continue;
            };
            let Some(algorithm) = HashAlgorithm::parse(algorithm) else {
                continue;
            };
            let digest = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|err| Error::InvalidChecksum {
                    value: entry.to_string(),
                    reason: err.to_string(),
                })?;
            return Self::with_digest(algorithm, digest, entry).map(Some);
        }
        Ok(None)
    }

    /// Parses the hash Corepack appends to `packageManager` declarations,
    /// e.g. the `sha512.<hex>` in `pnpm@9.0.0+sha512.<hex>`. Returns
    /// `Ok(None)` for algorithms we cannot verify (sha1).
    pub fn from_build_metadata(metadata: &str) -> Result<Option<Self>, Error> {
        let Some((algorithm, hex_digest)) = metadata.split_once('.') else {
            return Ok(None);
        };
        let Some(algorithm) = HashAlgorithm::parse(algorithm) else {
            return Ok(None);
        };
        Self::from_hex(algorithm, hex_digest).map(Some)
    }

    /// Finds `file_name` in a `SHASUMS256.txt`-style listing
    /// (`<hex>  <name>` per line).
    pub fn from_shasums(listing: &str, file_name: &str) -> Option<Result<Self, Error>> {
        listing.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            let digest = parts.next()?;
            let name = parts.next()?;
            let name = name.trim_start_matches('*');
            (name == file_name || name.rsplit('/').next() == Some(file_name))
                .then(|| Self::sha256_hex(digest))
        })
    }

    fn with_digest(algorithm: HashAlgorithm, digest: Vec<u8>, raw: &str) -> Result<Self, Error> {
        if digest.len() != algorithm.digest_len() {
            return Err(Error::InvalidChecksum {
                value: raw.to_string(),
                reason: format!(
                    "expected {} bytes for {}, got {}",
                    algorithm.digest_len(),
                    algorithm.name(),
                    digest.len()
                ),
            });
        }
        Ok(Self { algorithm, digest })
    }

    fn describe(&self) -> String {
        format!("{}-{}", self.algorithm.name(), hex::encode(&self.digest))
    }
}

enum Hasher {
    Sha224(Sha224),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Sha224 => Self::Sha224(Sha224::new()),
            HashAlgorithm::Sha256 => Self::Sha256(Sha256::new()),
            HashAlgorithm::Sha512 => Self::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha224(h) => h.update(bytes),
            Self::Sha256(h) => h.update(bytes),
            Self::Sha512(h) => h.update(bytes),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Sha224(h) => h.finalize().to_vec(),
            Self::Sha256(h) => h.finalize().to_vec(),
            Self::Sha512(h) => h.finalize().to_vec(),
        }
    }
}

/// HTTP client for fetching release metadata and archives.
#[derive(Debug, Clone)]
pub struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    /// Builds a client with turbo's TLS configuration.
    pub fn new() -> Result<Self, Error> {
        let client =
            turborepo_api_client::APIClient::build_http_client(Some(Duration::from_secs(30)))
                .map_err(|err| Error::Client(err.to_string()))?;
        Ok(Self { client })
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response, Error> {
        let response = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, "turbo-setup")
            .send()
            .await
            .map_err(|source| Error::Http {
                url: url.to_string(),
                source,
            })?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                url: url.to_string(),
                status: response.status().as_u16(),
            });
        }
        Ok(response)
    }

    pub async fn get_text(&self, url: &str) -> Result<String, Error> {
        self.get(url)
            .await?
            .text()
            .await
            .map_err(|source| Error::Http {
                url: url.to_string(),
                source,
            })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, Error> {
        self.get_json_with_accept(url, None).await
    }

    /// GET with an explicit `Accept` header (the npm registry serves a much
    /// smaller packument when asked for the abbreviated format).
    pub async fn get_json_with_accept<T: DeserializeOwned>(
        &self,
        url: &str,
        accept: Option<&str>,
    ) -> Result<T, Error> {
        let mut request = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, "turbo-setup");
        if let Some(accept) = accept {
            request = request.header(reqwest::header::ACCEPT, accept);
        }
        let response = request.send().await.map_err(|source| Error::Http {
            url: url.to_string(),
            source,
        })?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                url: url.to_string(),
                status: response.status().as_u16(),
            });
        }
        let body = response.bytes().await.map_err(|source| Error::Http {
            url: url.to_string(),
            source,
        })?;
        serde_json::from_slice(&body).map_err(|err| Error::Response {
            url: url.to_string(),
            reason: err.to_string(),
        })
    }

    /// Streams `url` into `dest`, verifying `checksum` when provided. `dest`
    /// is only left in place when the download completed and verified.
    pub async fn download(
        &self,
        url: &str,
        dest: &Path,
        checksum: Option<&Checksum>,
    ) -> Result<(), Error> {
        let response = self.get(url).await?;
        let dest_display = dest.display().to_string();
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|source| Error::io(&dest_display, source))?;
        let mut hasher = checksum.map(|checksum| Hasher::new(checksum.algorithm));
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|source| Error::Http {
                url: url.to_string(),
                source,
            })?;
            if let Some(hasher) = hasher.as_mut() {
                hasher.update(&chunk);
            }
            file.write_all(&chunk)
                .await
                .map_err(|source| Error::io(&dest_display, source))?;
        }
        file.flush()
            .await
            .map_err(|source| Error::io(&dest_display, source))?;
        drop(file);

        if let (Some(hasher), Some(expected)) = (hasher, checksum) {
            let actual = hasher.finish();
            if actual != expected.digest {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(Error::ChecksumMismatch {
                    url: url.to_string(),
                    expected: expected.describe(),
                    actual: format!("{}-{}", expected.algorithm.name(), hex::encode(actual)),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use httpmock::prelude::*;

    use super::*;

    #[test]
    fn parses_sri() {
        let digest = Sha512::digest(b"hello");
        let sri = format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(digest)
        );
        let checksum = Checksum::from_sri(&sri).unwrap().unwrap();
        assert_eq!(checksum.algorithm, HashAlgorithm::Sha512);
        assert_eq!(checksum.digest, digest.to_vec());
        assert!(Checksum::from_sri("sha1-AAAA").unwrap().is_none());
    }

    #[test]
    fn parses_corepack_metadata() {
        let digest = Sha224::digest(b"tarball");
        let checksum = Checksum::from_build_metadata(&format!("sha224.{}", hex::encode(digest)))
            .unwrap()
            .unwrap();
        assert_eq!(checksum.algorithm, HashAlgorithm::Sha224);
        assert!(
            Checksum::from_build_metadata("sha1.abcd")
                .unwrap()
                .is_none()
        );
        assert!(Checksum::from_build_metadata("sha256.zz").is_err());
    }

    #[test]
    fn finds_entry_in_shasums() {
        let listing = format!(
            "{}  node-v22.1.0-linux-x64.tar.gz\n{}  node-v22.1.0-win-x64.zip\n",
            format_args!("{:0>64}", "1"),
            format_args!("{:0>64}", "2")
        );
        let listing = listing.as_str();
        let checksum = Checksum::from_shasums(listing, "node-v22.1.0-win-x64.zip")
            .unwrap()
            .unwrap();
        assert_eq!(checksum.digest[31], 2);
        assert!(Checksum::from_shasums(listing, "missing").is_none());
    }

    #[tokio::test]
    async fn download_verifies_checksum() {
        let server = MockServer::start_async().await;
        let body = b"archive bytes".to_vec();
        server
            .mock_async(|when, then| {
                when.method(GET).path("/file.tar.gz");
                then.status(200).body(body.clone());
            })
            .await;
        let downloader = Downloader::with_client(reqwest::Client::new());
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("file.tar.gz");

        let good = Checksum::sha256_hex(&hex::encode(Sha256::digest(&body))).unwrap();
        downloader
            .download(&server.url("/file.tar.gz"), &dest, Some(&good))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);

        let bad = Checksum::sha256_hex(&"00".repeat(32)).unwrap();
        let err = downloader
            .download(&server.url("/file.tar.gz"), &dest, Some(&bad))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
        assert!(!dest.exists(), "failed download must be removed");
    }

    #[tokio::test]
    async fn download_reports_http_errors() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/missing");
                then.status(404);
            })
            .await;
        let downloader = Downloader::with_client(reqwest::Client::new());
        let err = downloader
            .get_text(&server.url("/missing"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::HttpStatus { status: 404, .. }));
    }
}
