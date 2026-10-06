use std::{
    error::Error as StdError,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

use super::*;

mod sha512;
mod transport;

type TestResult = Result<(), Box<dyn StdError>>;
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";

// Real loopback HTTP only. A pause between prefix/suffix can delay headers or
// the body; the task and listening socket are dropped at the end of each test.
struct Fixture {
    origin: String,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<std::io::Result<()>>,
}

impl Fixture {
    async fn new(prefix: &str, suffix: &str, pause: Duration) -> std::io::Result<Self> {
        Self::parts(&[(prefix, Duration::ZERO), (suffix, pause)]).await
    }

    async fn parts(parts: &[(&str, Duration)]) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let parts: Vec<_> = parts
            .iter()
            .map(|&(part, pause)| (part.to_owned(), pause))
            .collect();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).await?;
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                count.fetch_add(1, Ordering::SeqCst);
                for (part, pause) in &parts {
                    tokio::time::sleep(*pause).await;
                    if stream.write_all(part.as_bytes()).await.is_err() {
                        break;
                    }
                }
                let _ = stream.shutdown().await;
            }
        });
        Ok(Self { origin, hits, task })
    }

    fn client(&self) -> Result<DownloadClient, Error> {
        DownloadClient::with_http_builder(
            http_fixture_builder(),
            [ApprovedOrigin::loopback_http_for_tests(&self.origin)?],
        )
    }

    fn url(&self) -> String {
        format!("{}/artifact?token=private-query", self.origin)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// HTTP-only fixtures must not load ambient/system CAs; no_proxy only affects
// proxy discovery, not native certificate loading.
fn http_fixture_builder() -> ClientBuilder {
    Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .tls_built_in_native_certs(false)
}

fn limits(bytes: usize) -> Result<Limits, Error> {
    Limits::new(bytes, Duration::from_secs(2))
}

#[test]
fn verify_bytes_accepts_known_abc_digest() -> TestResult {
    let artifact =
        VerifiedArtifact::verify_bytes(b"abc".to_vec(), ExpectedSha256::from_hex(ABC_SHA256)?)?;
    assert_eq!(artifact.as_bytes(), b"abc");
    assert_eq!(artifact.into_bytes(), b"abc");
    Ok(())
}

#[test]
fn verify_bytes_rejects_altered_bytes() -> TestResult {
    let result =
        VerifiedArtifact::verify_bytes(b"abd".to_vec(), ExpectedSha256::from_hex(ABC_SHA256)?);
    assert!(matches!(result, Err(Error::DigestMismatch)));
    Ok(())
}

#[test]
fn verify_bytes_rejects_wrong_digest() -> TestResult {
    let result =
        VerifiedArtifact::verify_bytes(b"abc".to_vec(), ExpectedSha256::from_hex(&"0".repeat(64))?);
    assert!(matches!(result, Err(Error::DigestMismatch)));
    Ok(())
}

#[test]
fn verify_bytes_accepts_known_empty_digest() -> TestResult {
    let digest = ExpectedSha256::from_hex(
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    )?;
    let artifact = VerifiedArtifact::verify_bytes(Vec::new(), digest)?;
    assert!(artifact.as_bytes().is_empty());
    assert!(artifact.into_bytes().is_empty());
    Ok(())
}

#[test]
fn verify_bytes_digest_mismatch_is_redacted() -> TestResult {
    let error = VerifiedArtifact::verify_bytes(
        b"private-artifact-bytes".to_vec(),
        ExpectedSha256::from_hex(ABC_SHA256)?,
    )
    .err()
    .ok_or("expected a digest mismatch")?;
    assert_eq!(error, Error::DigestMismatch);
    assert_eq!(
        error.to_string(),
        "artifact SHA-256 does not match the trusted digest"
    );
    assert_eq!(format!("{error:?}"), "DigestMismatch");
    assert!(StdError::source(&error).is_none());
    Ok(())
}

#[tokio::test]
async fn reads_metadata_and_verifies_pinned_artifact() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    let client = fixture.client()?;
    assert_eq!(
        client.read_metadata(&fixture.url(), limits(3)?).await?,
        b"abc"
    );
    let artifact = client
        .download_verified(
            &fixture.url(),
            ExpectedSha256::from_hex(&ABC_SHA256.to_uppercase())?,
            limits(3)?,
        )
        .await?;
    assert_eq!(artifact.as_bytes(), b"abc");
    assert_eq!(artifact.into_bytes(), b"abc");
    assert_eq!(fixture.hits(), 2);
    Ok(())
}

#[tokio::test]
async fn missing_and_malformed_digest_cannot_start_download() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    assert!(matches!(
        ExpectedSha256::from_hex(""),
        Err(Error::MissingDigest)
    ));
    for digest in ["abc", &"g".repeat(64), &"0".repeat(63), &"0".repeat(65)] {
        assert!(matches!(
            ExpectedSha256::from_hex(digest),
            Err(Error::InvalidDigest)
        ));
    }
    assert_eq!(fixture.hits(), 0);
    Ok(())
}

#[tokio::test]
async fn wrong_digest_never_returns_artifact() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    let result = fixture
        .client()?
        .download_verified(
            &fixture.url(),
            ExpectedSha256::from_hex(&"0".repeat(64))?,
            limits(3)?,
        )
        .await;
    assert!(matches!(result, Err(Error::DigestMismatch)));
    assert_eq!(fixture.hits(), 1);
    Ok(())
}

#[tokio::test]
async fn total_timeout_covers_headers_and_body() -> TestResult {
    let header = "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n";
    for prefix in ["", header] {
        let suffix = if prefix.is_empty() { OK } else { "abc" };
        let fixture = Fixture::new(prefix, suffix, Duration::from_secs(1)).await?;
        let result = fixture
            .client()?
            .download_verified(
                &fixture.url(),
                ExpectedSha256::from_hex(ABC_SHA256)?,
                Limits::new(3, Duration::from_millis(50))?,
            )
            .await;
        assert!(matches!(result, Err(Error::TimedOut)));
        assert_eq!(fixture.hits(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn bounds_declared_chunked_and_close_delimited_bodies() -> TestResult {
    let responses = [
        OK,
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: \
         close\r\n\r\n1\r\na\r\n2\r\nbc\r\n0\r\n\r\n",
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabc",
    ];
    for response in responses {
        let fixture = Fixture::new(response, "", Duration::ZERO).await?;
        let client = fixture.client()?;
        assert_eq!(
            client.read_metadata(&fixture.url(), limits(2)?).await,
            Err(Error::TooLarge)
        );
        assert!(matches!(
            client
                .download_verified(
                    &fixture.url(),
                    ExpectedSha256::from_hex(ABC_SHA256)?,
                    limits(2)?
                )
                .await,
            Err(Error::TooLarge)
        ));
        assert_eq!(
            client.read_metadata(&fixture.url(), limits(3)?).await?,
            b"abc"
        );
    }
    Ok(())
}

#[tokio::test]
async fn chunked_writes_require_completion_and_keep_total_deadline() -> TestResult {
    let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    // The unterminated case contains all of `abc`, matching the pinned digest.
    // The slow case advances every 200ms, but must still exhaust its 300ms budget.
    for (end, gap_ms, budget_ms, failure) in [
        ("0\r\n\r\n", 10, 2000, None),
        ("", 0, 2000, Some(Error::RequestFailed)),
        ("0\r\n\r\n", 200, 300, Some(Error::TimedOut)),
    ] {
        let gap = Duration::from_millis(gap_ms);
        let fixture = Fixture::parts(&[
            (header, Duration::ZERO),
            ("1\r\na\r\n", Duration::ZERO),
            ("1\r\nb\r\n", gap),
            ("1\r\nc\r\n", gap),
            (end, Duration::ZERO),
        ])
        .await?;
        let result = fixture
            .client()?
            .download_verified(
                &fixture.url(),
                ExpectedSha256::from_hex(ABC_SHA256)?,
                Limits::new(3, Duration::from_millis(budget_ms))?,
            )
            .await
            .map(VerifiedArtifact::into_bytes);
        assert_eq!(result, failure.map_or(Ok(b"abc".to_vec()), Err));
        assert_eq!(fixture.hits(), 1);
    }
    Ok(())
}

#[tokio::test]
async fn oversized_content_length_is_rejected_without_waiting_for_body() -> TestResult {
    let fixture = Fixture::new(
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
        "abcd",
        Duration::from_secs(1),
    )
    .await?;
    assert_eq!(
        fixture
            .client()?
            .read_metadata(&fixture.url(), Limits::new(3, Duration::from_millis(100))?,)
            .await,
        Err(Error::TooLarge)
    );
    assert_eq!(fixture.hits(), 1);
    Ok(())
}

#[tokio::test]
async fn redirect_policy_is_forced_off_even_for_approved_targets() -> TestResult {
    let target = Fixture::new(OK, "", Duration::ZERO).await?;
    for status in [301, 302, 303, 307, 308] {
        let response = format!(
            "HTTP/1.1 {status} Redirect\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
            target.url()
        );
        let source = Fixture::new(&response, "", Duration::ZERO).await?;
        let client = DownloadClient::with_http_builder(
            http_fixture_builder().redirect(Policy::limited(10)),
            [
                ApprovedOrigin::loopback_http_for_tests(&source.origin)?,
                ApprovedOrigin::loopback_http_for_tests(&target.origin)?,
            ],
        )?;
        assert_eq!(
            client.read_metadata(&source.url(), limits(3)?).await,
            Err(Error::RedirectRejected)
        );
        assert_eq!(source.hits(), 1);
    }
    assert_eq!(target.hits(), 0);
    Ok(())
}

#[tokio::test]
async fn origin_approval_is_exact_and_empty_list_denies_all() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    let other = Fixture::new(OK, "", Duration::ZERO).await?;
    let client = fixture.client()?;
    let port = Url::parse(&fixture.origin)?
        .port()
        .ok_or("fixture port missing")?;
    for url in [
        other.url(),
        format!("http://localhost:{port}/artifact"),
        fixture.url().replacen("http://", "https://", 1),
        format!("http://127.0.0.1.example.invalid:{port}/artifact"),
    ] {
        assert_eq!(
            client.read_metadata(&url, limits(3)?).await,
            Err(Error::UnapprovedOrigin)
        );
    }
    let none = DownloadClient::new([])?;
    assert_eq!(
        none.read_metadata(&fixture.url(), limits(3)?).await,
        Err(Error::UnapprovedOrigin)
    );
    assert_eq!(fixture.hits(), 0);
    assert_eq!(other.hits(), 0);
    Ok(())
}

#[test]
fn approvals_are_https_by_default_and_http_escape_is_loopback_only() -> TestResult {
    for origin in [
        "http://127.0.0.1:1234",
        "https://example.test/path",
        "https://example.test/fixtures/..",
        "https://example.test/%2e/",
        "https://example.test?secret=x",
        "https://example.test#secret",
        "https://example.test:443/path",
    ] {
        assert!(matches!(
            ApprovedOrigin::https(origin),
            Err(Error::InvalidOrigin)
        ));
    }
    for origin in [
        "http://localhost:1234",
        "http://192.0.2.1",
        "http://127.0.0.1/fixtures/..",
        "https://127.0.0.1",
    ] {
        assert!(matches!(
            ApprovedOrigin::loopback_http_for_tests(origin),
            Err(Error::InvalidOrigin)
        ));
    }
    assert!(ApprovedOrigin::https("https://example.test/").is_ok());
    assert!(ApprovedOrigin::loopback_http_for_tests("http://[::1]:1234").is_ok());
    assert!(ApprovedOrigin::loopback_http_for_tests("http://127.0.0.2:1234").is_ok());
    let canonical = ApprovedOrigin::https("https://EXAMPLE.test:443")?;
    assert_eq!(
        canonical.0,
        Url::parse("https://example.test/metadata")?.origin()
    );
    assert_ne!(
        canonical.0,
        Url::parse("https://example.test:444/metadata")?.origin()
    );
    Ok(())
}

#[tokio::test]
async fn credentials_and_invalid_urls_are_rejected_before_http() -> TestResult {
    let fixture = Fixture::new(OK, "", Duration::ZERO).await?;
    let client = fixture.client()?;
    for userinfo in ["user:private-password", "user", "", ":private-password"] {
        let url = fixture.url().replacen("://", &format!("://{userinfo}@"), 1);
        assert_eq!(
            client.read_metadata(&url, limits(3)?).await,
            Err(Error::UrlCredentials)
        );
        let origin = fixture
            .origin
            .replacen("://", &format!("://{userinfo}@"), 1);
        assert!(matches!(
            ApprovedOrigin::loopback_http_for_tests(&origin),
            Err(Error::UrlCredentials)
        ));
    }
    for url in [
        "not a URL?private-query",
        "/relative",
        "file:///private-path",
        "http:////@127.0.0.1/artifact",
        "http://\\\\127.0.0.1/artifact",
    ] {
        assert_eq!(
            client.read_metadata(url, limits(3)?).await,
            Err(Error::InvalidUrl)
        );
    }
    assert_eq!(fixture.hits(), 0);
    Ok(())
}

#[tokio::test]
async fn diagnostics_never_include_http_urls_bodies_or_sources() -> TestResult {
    for (response, expected) in [
        (
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 12\r\n\r\nprivate-body",
            Error::HttpStatus(403),
        ),
        ("", Error::RequestFailed),
        ("malformed private-body\r\n\r\n", Error::RequestFailed),
    ] {
        let fixture = Fixture::new(response, "", Duration::ZERO).await?;
        let result = fixture
            .client()?
            .read_metadata(&fixture.url(), limits(20)?)
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => return Err("expected an HTTP failure".into()),
        };
        assert_eq!(error, expected);
        let diagnostic = format!("{error} {error:?}");
        for secret in ["private-query", "private-body", "127.0.0.1", "http://"] {
            assert!(!diagnostic.contains(secret));
        }
        assert!(StdError::source(&error).is_none());
    }
    Ok(())
}

#[test]
fn limits_must_be_nonzero_and_timeout_representable() {
    assert!(matches!(
        Limits::new(0, Duration::from_secs(1)),
        Err(Error::InvalidLimits)
    ));
    assert!(matches!(
        Limits::new(1, Duration::ZERO),
        Err(Error::InvalidLimits)
    ));
    assert!(matches!(
        Limits::new(1, Duration::MAX),
        Err(Error::InvalidLimits)
    ));
}
