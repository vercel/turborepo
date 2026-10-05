//! Bounded, in-memory downloads, without extraction, installation, or CLI
//! wiring.
//!
//! Callers explicitly approve origins and choose byte/time limits per
//! operation. Metadata is **not verified**: a digest fetched alongside an
//! artifact from an untrusted source does not establish authenticity. Artifact
//! digests must come from a caller's pinned or otherwise trusted source.
//! Platform selection belongs to consumers (using `turborepo-platform`), not
//! this transport library.
//!
//! The default client uses reqwest's system proxy discovery (`HTTPS_PROXY`,
//! `HTTP_PROXY`, `NO_PROXY`, including reqwest's lowercase variants) and rustls
//! with native + bundled webpki roots, like Remote Cache. The native-root
//! loader honors `SSL_CERT_FILE` (PEM bundle) and `SSL_CERT_DIR` (certificate
//! directories). If default construction fails with native roots, it retries
//! with verified webpki roots only. Environment trust never disables
//! verification. Set environment configuration before constructing the client;
//! reuse it for subsequent downloads. Trusted callers can inject a builder to
//! add roots or override proxy discovery without changing the download safety
//! limits.
//!
//! ```no_run
//! use std::time::Duration;
//! use turborepo_download::{ApprovedOrigin, DownloadClient, Error, ExpectedSha256, Limits};
//!
//! // The caller supplies a pinned digest, not one taken from untrusted metadata.
//! async fn artifact(pinned_sha256: &str) -> Result<Vec<u8>, Error> {
//!     let client = DownloadClient::new([
//!         ApprovedOrigin::https("https://downloads.example.test")?,
//!     ])?;
//!     let digest = ExpectedSha256::from_hex(pinned_sha256)?;
//!     let limits = Limits::new(16 * 1024 * 1024, Duration::from_secs(30))?;
//!     Ok(client.download_verified(
//!         "https://downloads.example.test/artifact", digest, limits,
//!     ).await?.into_bytes())
//! }
//! ```

use std::{error::Error as StdError, time::Duration};

use reqwest::{Client, ClientBuilder, header::ACCEPT_ENCODING, redirect::Policy};
use sha2::{Digest, Sha256};
use url::{Host, Origin, Url};

/// Returned errors deliberately contain no URLs, response bodies, or underlying
/// HTTP errors, including in `Debug` and the error source chain. This contract
/// does not cover dependency logging (reqwest debug logs can include URLs).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid absolute HTTP(S) URL")]
    InvalidUrl,
    #[error("URL credentials are forbidden")]
    UrlCredentials,
    #[error("approval requires an HTTPS origin or explicit loopback HTTP test origin")]
    InvalidOrigin,
    #[error("URL origin is not approved")]
    UnapprovedOrigin,
    #[error("nonzero byte limit and representable, nonzero timeout are required")]
    InvalidLimits,
    #[error("SHA-256 digest is required")]
    MissingDigest,
    #[error("SHA-256 digest must contain exactly 64 hexadecimal characters")]
    InvalidDigest,
    #[error("could not build download HTTP client; check TLS and proxy configuration")]
    ClientBuild,
    #[error("download certificate verification failed; check custom CA trust")]
    CertificateFailed,
    #[error("download proxy authentication required")]
    ProxyAuthenticationRequired,
    #[error("download connection failed; check network, proxy, and certificate configuration")]
    ConnectionFailed,
    #[error("download HTTP request failed")]
    RequestFailed,
    #[error("download time limit exceeded")]
    TimedOut,
    #[error("download byte limit exceeded")]
    TooLarge,
    #[error("download redirects are forbidden")]
    RedirectRejected,
    #[error("download returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("artifact SHA-256 does not match the trusted digest")]
    DigestMismatch,
}

/// Exact scheme/host/effective-port approval, never a URL prefix or wildcard.
#[derive(Clone)]
pub struct ApprovedOrigin(Origin);

impl ApprovedOrigin {
    /// Approves HTTPS only. Paths, queries, fragments, and credentials are not
    /// allowed in an origin; requests may have paths and queries.
    pub fn https(origin: &str) -> Result<Self, Error> {
        Self::parse(origin, false)
    }

    /// Explicit test-only opt-in for HTTP at a literal loopback IP address.
    /// DNS names (including `localhost`) are not accepted by this escape hatch.
    pub fn loopback_http_for_tests(origin: &str) -> Result<Self, Error> {
        Self::parse(origin, true)
    }

    fn parse(origin: &str, allow_loopback_http: bool) -> Result<Self, Error> {
        // Inspect raw spelling before URL parsing erases dot segments.
        let raw = origin.split_once("://").ok_or(Error::InvalidUrl)?.1;
        let suffix = raw.find(['/', '\\', '?', '#']).map_or("", |i| &raw[i..]);
        if !matches!(suffix, "" | "/") {
            return Err(Error::InvalidOrigin);
        }
        let url = parse_url(origin)?;
        let loopback = match url.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        let scheme_allowed = if allow_loopback_http {
            url.scheme() == "http" && loopback
        } else {
            url.scheme() == "https"
        };
        if !scheme_allowed || url.path() != "/" || url.query().is_some() || url.fragment().is_some()
        {
            return Err(Error::InvalidOrigin);
        }
        Ok(Self(url.origin()))
    }
}

/// Nonzero body-byte limit and a total request/body/verification time budget.
/// Payload length is bounded, not total allocator/protocol overhead. Bytes are
/// buffered in memory, never written to disk; timeouts are not CPU preemption.
#[derive(Clone, Copy)]
pub struct Limits {
    max_bytes: usize,
    timeout: Duration,
}

impl Limits {
    pub fn new(max_bytes: usize, timeout: Duration) -> Result<Self, Error> {
        if max_bytes == 0
            || timeout.is_zero()
            || std::time::Instant::now().checked_add(timeout).is_none()
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self { max_bytes, timeout })
    }
}

/// An expected SHA-256 value. Parsing validates format, **not trust**; the
/// caller must obtain the value from a pinned or otherwise trusted source.
#[derive(Clone, Copy)]
pub struct ExpectedSha256([u8; 32]);

impl ExpectedSha256 {
    pub fn from_hex(digest: &str) -> Result<Self, Error> {
        if digest.is_empty() {
            return Err(Error::MissingDigest);
        }
        if digest.len() != 64 {
            return Err(Error::InvalidDigest);
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(digest, &mut bytes).map_err(|_| Error::InvalidDigest)?;
        Ok(Self(bytes))
    }
}

/// Constructed only after successful verification; has no filesystem effects.
pub struct VerifiedArtifact(Vec<u8>);

impl VerifiedArtifact {
    /// Synchronously verifies local bytes against a mandatory, caller-supplied
    /// SHA-256 digest from a pinned or otherwise trusted source.
    ///
    /// The caller must bound input size before calling; this helper applies no
    /// byte or time limits. It performs no HTTP, filesystem, or cache I/O and
    /// has no CLI or installer behavior. A matching digest pins the bytes, not
    /// publisher trust or authenticity.
    ///
    /// Returns [`Error::DigestMismatch`] without bytes or digests on mismatch.
    pub fn verify_bytes(bytes: Vec<u8>, digest: ExpectedSha256) -> Result<Self, Error> {
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if actual != digest.0 {
            return Err(Error::DigestMismatch);
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Reusable HTTP transport. Intentionally no `Debug` that could expose URLs.
#[derive(Clone)]
pub struct DownloadClient {
    client: Client,
    origins: Vec<ApprovedOrigin>,
}

impl DownloadClient {
    pub fn new(origins: impl IntoIterator<Item = ApprovedOrigin>) -> Result<Self, Error> {
        // Match Remote Cache's native -> bundled trust fallback, without an
        // API-client dependency or overriding reqwest's proxy discovery.
        let origins: Vec<_> = origins.into_iter().collect();
        let build = |native| {
            Self::with_http_builder(
                Client::builder()
                    .use_rustls_tls()
                    .tls_built_in_native_certs(native)
                    .tls_built_in_webpki_certs(true),
                origins.iter().cloned(),
            )
        };
        match build(true) {
            // reqwest can reject PEM-valid but DER-invalid native roots even
            // with webpki enabled. Retry only default construction: keep verified
            // bundled trust, origin approvals, and all download safety policies.
            Err(Error::ClientBuild) => build(false),
            result => result,
        }
    }

    /// Inject trusted HTTP configuration, rather than an already-built client
    /// whose redirect policy cannot be checked. Redirects, retries, and
    /// automatic decoding are overridden here. Callers are responsible for
    /// the security of injected TLS/proxy configuration. For example,
    /// `Client::builder().add_root_certificate(cert)` adds custom trust and
    /// `.no_proxy()` disables environment proxies. An empty approval list
    /// denies all requests. Build failures are returned without substituting a
    /// default client, so explicit injected trust is never silently discarded.
    pub fn with_http_builder(
        builder: ClientBuilder,
        origins: impl IntoIterator<Item = ApprovedOrigin>,
    ) -> Result<Self, Error> {
        let client = builder
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .danger_accept_invalid_certs(false)
            .danger_accept_invalid_hostnames(false)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()
            .map_err(|_| Error::ClientBuild)?;
        Ok(Self {
            client,
            origins: origins.into_iter().collect(),
        })
    }

    /// Read bounded, unverified metadata. Consumers must parse and validate it;
    /// this operation does not turn metadata-provided digests into trusted
    /// ones.
    pub async fn read_metadata(&self, url: &str, limits: Limits) -> Result<Vec<u8>, Error> {
        self.fetch(url, limits, None).await
    }

    /// Buffer an artifact and release it only if its SHA-256 matches. A digest
    /// is mandatory in the API; there is no unverified artifact fallback.
    pub async fn download_verified(
        &self,
        url: &str,
        digest: ExpectedSha256,
        limits: Limits,
    ) -> Result<VerifiedArtifact, Error> {
        self.fetch(url, limits, Some(digest))
            .await
            .map(VerifiedArtifact)
    }

    async fn fetch(
        &self,
        input: &str,
        limits: Limits,
        expected: Option<ExpectedSha256>,
    ) -> Result<Vec<u8>, Error> {
        let url = parse_url(input)?;
        if !self.origins.iter().any(|origin| origin.0 == url.origin()) {
            return Err(Error::UnapprovedOrigin);
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(limits.timeout)
            .ok_or(Error::InvalidLimits)?;
        tokio::time::timeout_at(deadline, async {
            let mut response = self
                .client
                .get(url)
                .header(ACCEPT_ENCODING, "identity")
                .timeout(limits.timeout)
                .send()
                .await
                .map_err(http_error)?;
            let status = response.status();
            if status.is_redirection() {
                return Err(Error::RedirectRejected);
            }
            if status == reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED {
                return Err(Error::ProxyAuthenticationRequired);
            }
            if !status.is_success() {
                return Err(Error::HttpStatus(status.as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|n| n > limits.max_bytes as u64)
            {
                return Err(Error::TooLarge);
            }
            let mut bytes = Vec::new();
            let mut hasher = expected.map(|_| Sha256::new());
            while let Some(chunk) = response.chunk().await.map_err(http_error)? {
                if chunk.len() > limits.max_bytes - bytes.len() {
                    return Err(Error::TooLarge);
                }
                if let Some(hasher) = &mut hasher {
                    hasher.update(&chunk);
                }
                bytes.extend_from_slice(&chunk);
            }
            if let (Some(expected), Some(hasher)) = (expected, hasher) {
                let actual: [u8; 32] = hasher.finalize().into();
                if actual != expected.0 {
                    return Err(Error::DigestMismatch);
                }
            }
            // Async timeouts cannot preempt synchronous hashing/copying. Never
            // return bytes successfully if that work consumed the time budget.
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::TimedOut);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| Error::TimedOut)?
    }
}

fn parse_url(input: &str) -> Result<Url, Error> {
    let url = Url::parse(input).map_err(|_| Error::InvalidUrl)?;
    // Also reject empty userinfo (`https://@host`), which URL normalization
    // otherwise erases. Require an explicit absolute authority.
    let (_, rest) = input.split_once("://").ok_or(Error::InvalidUrl)?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or(Error::InvalidUrl)?;
    if authority.contains('@') || !url.username().is_empty() || url.password().is_some() {
        return Err(Error::UrlCredentials);
    }
    if authority.is_empty()
        || authority.contains('\\')
        || !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
    {
        return Err(Error::InvalidUrl);
    }
    Ok(url)
}

fn http_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        return Error::TimedOut;
    }
    // Classify typed errors only: no formatting/string matching of sources
    // containing URLs, proxy credentials, certificate names, or filesystem paths.
    let mut source: Option<&(dyn StdError + 'static)> = Some(&error);
    while let Some(cause) = source {
        if matches!(
            cause.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented)
        ) {
            return Error::CertificateFailed;
        }
        // io::Error::source may skip its immediate inner error.
        source = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.get_ref().map(|inner| inner as &dyn StdError)
        } else {
            cause.source()
        };
    }
    if error.is_connect() {
        // reqwest cannot reliably distinguish a proxy CONNECT failure from an
        // origin connection failure. Give a neutral hint, not false attribution.
        Error::ConnectionFailed
    } else {
        Error::RequestFailed
    }
}

#[cfg(test)]
mod tests;
