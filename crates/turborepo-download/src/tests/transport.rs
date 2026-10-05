//! Loopback-only TLS/proxy fixtures. Environment tests run in child processes:
//! no process-global env mutation or dependency on the developer's proxy/CA.
use std::{io, net::SocketAddr, sync::Mutex};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use tokio::{
    io::{AsyncRead, AsyncWrite, copy_bidirectional},
    net::TcpStream,
    process::Command,
};
use tokio_rustls::TlsAcceptor;

use super::*;

const CA: &[u8] = include_bytes!("fixtures/ca.pem");
const SERVER: &[u8] = include_bytes!("fixtures/server.pem");
const EXPIRED: &[u8] = include_bytes!("fixtures/expired.pem");
const KEY: &[u8] = include_bytes!("fixtures/server-key.pem");
const PROXY_AUTH: &str = "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: \
                          12\r\nConnection: close\r\n\r\nprivate-body";
const CA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tests/fixtures/ca.pem");
const INVALID_DER: &[u8] = include_bytes!("fixtures/invalid-der.pem");
const INVALID_DER_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/tests/fixtures/invalid-der.pem"
);

trait Socket: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Socket for T {}

struct Local {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<io::Result<()>>,
}

impl Local {
    async fn listen(
        certificate: Option<&[u8]>,
        tunnel: Option<SocketAddr>,
        response: &str,
    ) -> Result<Self, Box<dyn StdError>> {
        let acceptor = if let Some(certificate) = certificate {
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(certificate)?],
                PrivateKeyDer::from_pem_slice(KEY)?,
            )?;
            Some(TlsAcceptor::from(Arc::new(config)))
        } else {
            None
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let response = response.to_owned();
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await?;
                let mut stream: Box<dyn Socket> = if let Some(acceptor) = &acceptor {
                    match acceptor.accept(stream).await {
                        Ok(stream) => Box::new(stream),
                        Err(_) => continue, // Untrusted/expired/hostname-negative cases.
                    }
                } else {
                    Box::new(stream)
                };
                let request = read_headers(&mut stream).await?;
                recorded
                    .lock()
                    .map_err(|_| io::Error::other("poisoned fixture"))?
                    .push(request.clone());
                if let Some(upstream) = tunnel {
                    // Never resolve or connect to the request's authority: this
                    // proxy can only tunnel to its preselected loopback fixture.
                    if !request.starts_with(&format!(
                        "CONNECT localhost:{} HTTP/1.1\r\n",
                        upstream.port()
                    )) {
                        return Err(io::Error::other("unexpected CONNECT target"));
                    }
                    let mut upstream = TcpStream::connect(upstream).await?;
                    stream
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await?;
                    let _ = copy_bidirectional(&mut stream, &mut upstream).await;
                } else {
                    let _ = stream.write_all(response.as_bytes()).await;
                }
                let _ = stream.shutdown().await;
            }
        });
        Ok(Self {
            addr,
            requests,
            task,
        })
    }

    fn http_origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn https_origin(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
    }

    fn requests(&self) -> Result<Vec<String>, Box<dyn StdError>> {
        Ok(self
            .requests
            .lock()
            .map_err(|_| "poisoned fixture")?
            .clone())
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_headers(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buffer).await?;
        if n == 0 || bytes.len() + n > 8192 {
            return Err(io::Error::other("incomplete/oversized fixture headers"));
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

fn assert_private(error: &Error) {
    let diagnostic = format!("{error} {error:?}");
    for private in [
        "http://",
        "https://",
        "localhost",
        "127.0.0.1",
        "private-query",
        "private-body",
        "private-proxy-user",
        "private-proxy-password",
        "private-ca-path",
        "private-certificate-host",
        "Turborepo download test CA",
    ] {
        assert!(
            !diagnostic.contains(private),
            "diagnostic leaked private data"
        );
    }
    assert!(StdError::source(error).is_none());
}

// Child-only entry point. The ordinary test run performs no work here.
#[tokio::test]
async fn environment_child() -> TestResult {
    let Ok(url) = std::env::var("TURBO_DOWNLOAD_TEST_URL") else {
        return Ok(());
    };
    let parsed = Url::parse(&url)?;
    if !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err("environment fixture must be loopback".into());
    }
    let origin = parsed.origin().ascii_serialization();
    let approved = if parsed.scheme() == "http" {
        ApprovedOrigin::loopback_http_for_tests(&origin)?
    } else {
        ApprovedOrigin::https(&origin)?
    };
    if std::env::var_os("TURBO_DOWNLOAD_TEST_BAD_NATIVE_ROOTS").is_some() {
        // Prove this fixture triggers reqwest's native-root build failure even
        // with webpki and an explicit valid custom CA. Injected trust must not
        // be silently replaced by a fresh default builder on failure.
        let builder = Client::builder()
            .use_rustls_tls()
            .tls_built_in_native_certs(true)
            .tls_built_in_webpki_certs(true)
            .add_root_certificate(reqwest::Certificate::from_pem(CA)?);
        let error = DownloadClient::with_http_builder(builder, [approved.clone()])
            .err()
            .ok_or("expected native-root client build failure")?;
        assert_eq!(error, Error::ClientBuild);
        assert_private(&error);
        let denied = DownloadClient::new([])?;
        assert_eq!(
            denied.read_metadata(&url, limits(3)?).await,
            Err(Error::UnapprovedOrigin)
        );
    }
    let result = DownloadClient::new([approved])?
        .download_verified(&url, ExpectedSha256::from_hex(ABC_SHA256)?, limits(3)?)
        .await;
    let expected = std::env::var("TURBO_DOWNLOAD_TEST_EXPECTED")?;
    match result {
        Ok(artifact) => {
            assert_eq!(expected, "success");
            assert_eq!(artifact.as_bytes(), b"abc");
        }
        Err(error) => {
            assert_eq!(format!("{error:?}"), expected);
            assert_private(&error);
        }
    }
    Ok(())
}

async fn environment(url: &str, vars: &[(&str, &str)], expected: &str) -> TestResult {
    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "--exact",
        "tests::transport::environment_child",
        "--nocapture",
    ]);
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "REQUEST_METHOD",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        command.env_remove(name);
    }
    command.env_remove("TURBO_DOWNLOAD_TEST_BAD_NATIVE_ROOTS");
    command.env("TURBO_DOWNLOAD_TEST_URL", url);
    command.env("TURBO_DOWNLOAD_TEST_EXPECTED", expected);
    command.envs(vars.iter().copied());
    let output = command.output().await?;
    assert!(
        output.status.success(),
        "child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test]
async fn pem_valid_der_invalid_default_fallback_keeps_verification() -> TestResult {
    let der = CertificateDer::from_pem_slice(INVALID_DER)?;
    assert_eq!(der.as_ref(), &[0x30]); // Valid PEM, truncated DER SEQUENCE.
    assert!(rustls::RootCertStore::empty().add(der).is_err());
    let http = Local::listen(None, None, OK).await?;
    let https = Local::listen(Some(SERVER), None, OK).await?;
    let proxy = Local::listen(None, None, OK).await?;
    let proxy_url = proxy.http_origin();
    let vars = [
        ("SSL_CERT_FILE", INVALID_DER_PATH),
        ("SSL_CERT_DIR", ""),
        ("HTTP_PROXY", proxy_url.as_str()),
        ("NO_PROXY", "localhost"),
        ("TURBO_DOWNLOAD_TEST_BAD_NATIVE_ROOTS", "1"),
    ];
    environment(
        &format!("{}/artifact?token=private-query", http.http_origin()),
        &vars,
        "success",
    )
    .await?;
    // The fallback still rejects a CA absent from the bundled trust store;
    // neither invalid-certificate nor hostname verification may be bypassed.
    environment(
        &format!("{}/artifact?token=private-query", https.https_origin()),
        &vars,
        "CertificateFailed",
    )
    .await?;
    assert!(http.requests()?.is_empty());
    assert_eq!(proxy.requests()?.len(), 1);
    assert!(https.requests()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn http_proxy_environment_and_lowercase_variant() -> TestResult {
    let origin = Local::listen(None, None, OK).await?;
    let proxy = Local::listen(None, None, OK).await?;
    let url = format!("{}/artifact?token=private-query", origin.http_origin());
    for name in ["HTTP_PROXY", "http_proxy"] {
        environment(
            &url,
            &[(name, &proxy.http_origin()), ("NO_PROXY", "")],
            "success",
        )
        .await?;
    }
    assert!(origin.requests()?.is_empty());
    let requests = proxy.requests()?;
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert!(request.starts_with(&format!("GET {url} HTTP/1.1\r\n")));
    }
    Ok(())
}

#[tokio::test]
async fn https_proxy_connect_and_custom_ca_verify_artifact() -> TestResult {
    let origin = Local::listen(Some(SERVER), None, OK).await?;
    let proxy = Local::listen(None, Some(origin.addr), "").await?;
    environment(
        &format!("{}/artifact?token=private-query", origin.https_origin()),
        &[
            ("HTTPS_PROXY", &proxy.http_origin()),
            ("SSL_CERT_FILE", CA_PATH),
            ("NO_PROXY", ""),
        ],
        "success",
    )
    .await?;
    assert_eq!(proxy.requests()?.len(), 1);
    assert_eq!(origin.requests()?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn no_proxy_bypasses_http_and_https_proxies() -> TestResult {
    let http = Local::listen(None, None, OK).await?;
    let https = Local::listen(Some(SERVER), None, OK).await?;
    let proxy = Local::listen(None, None, PROXY_AUTH).await?;
    for origin in [http.http_origin(), https.https_origin()] {
        environment(
            &format!("{origin}/artifact"),
            &[
                ("HTTP_PROXY", &proxy.http_origin()),
                ("HTTPS_PROXY", &proxy.http_origin()),
                ("NO_PROXY", "127.0.0.1,localhost"),
                ("SSL_CERT_FILE", CA_PATH),
            ],
            "success",
        )
        .await?;
    }
    assert!(proxy.requests()?.is_empty());
    assert_eq!(http.requests()?.len(), 1);
    assert_eq!(https.requests()?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn ca_environment_file_and_directory_and_untrusted_failure() -> TestResult {
    let origin = Local::listen(Some(SERVER), None, OK).await?;
    let url = format!("{}/artifact?token=private-query", origin.https_origin());
    environment(&url, &[], "CertificateFailed").await?;
    environment(&url, &[("SSL_CERT_FILE", CA_PATH)], "success").await?;
    // File and directory loading are separately exercised, not accidentally
    // satisfied by the file setting or a machine's native trust store.
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))?;
    std::fs::write(dir.path().join("root.pem"), CA)?;
    let path = dir.path().to_str().ok_or("non-UTF8 fixture path")?;
    environment(&url, &[("SSL_CERT_DIR", path)], "success").await?;
    std::fs::write(
        dir.path().join("root.pem"),
        b"not a certificate: private-ca-path",
    )?;
    environment(&url, &[("SSL_CERT_DIR", path)], "CertificateFailed").await?;
    let missing = dir.path().join("private-ca-path");
    environment(
        &url,
        &[(
            "SSL_CERT_FILE",
            missing.to_str().ok_or("non-UTF8 fixture path")?,
        )],
        "CertificateFailed",
    )
    .await?;
    assert_eq!(origin.requests()?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn proxy_failures_are_redacted_and_not_attributed_to_origin() -> TestResult {
    let http = Local::listen(None, None, OK).await?;
    let https = Local::listen(Some(SERVER), None, OK).await?;
    let proxy = Local::listen(None, None, PROXY_AUTH).await?;
    let proxy_url = format!(
        "http://private-proxy-user:private-proxy-password@{}",
        proxy.addr
    );
    for (origin, name, expected) in [
        (
            http.http_origin(),
            "HTTP_PROXY",
            "ProxyAuthenticationRequired",
        ),
        (https.https_origin(), "HTTPS_PROXY", "ConnectionFailed"),
    ] {
        environment(
            &format!("{origin}/artifact?token=private-query"),
            &[(name, &proxy_url), ("NO_PROXY", "")],
            expected,
        )
        .await?;
    }
    assert_eq!(proxy.requests()?.len(), 2);
    assert!(http.requests()?.is_empty());
    assert!(https.requests()?.is_empty());
    Ok(())
}

fn custom_root_builder() -> Result<ClientBuilder, Error> {
    Ok(Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .tls_built_in_native_certs(false)
        .tls_built_in_webpki_certs(false)
        .add_root_certificate(reqwest::Certificate::from_pem(CA).map_err(|_| Error::ClientBuild)?))
}

#[tokio::test]
async fn injected_custom_root_keeps_digest_size_and_redirect_checks() -> TestResult {
    let origin = Local::listen(Some(SERVER), None, OK).await?;
    let url = format!("{}/artifact", origin.https_origin());
    let client = DownloadClient::with_http_builder(
        custom_root_builder()?,
        [ApprovedOrigin::https(&origin.https_origin())?],
    )?;
    assert_eq!(
        client
            .download_verified(&url, ExpectedSha256::from_hex(ABC_SHA256)?, limits(3)?)
            .await?
            .as_bytes(),
        b"abc"
    );
    assert!(matches!(
        client
            .download_verified(&url, ExpectedSha256::from_hex(&"0".repeat(64))?, limits(3)?)
            .await,
        Err(Error::DigestMismatch)
    ));
    assert!(matches!(
        client
            .download_verified(&url, ExpectedSha256::from_hex(ABC_SHA256)?, limits(2)?)
            .await,
        Err(Error::TooLarge)
    ));
    let response = format!(
        "HTTP/1.1 302 Redirect\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: \
         close\r\n\r\n"
    );
    let redirect = Local::listen(Some(SERVER), None, &response).await?;
    let client = DownloadClient::with_http_builder(
        custom_root_builder()?.redirect(Policy::limited(10)),
        [
            ApprovedOrigin::https(&origin.https_origin())?,
            ApprovedOrigin::https(&redirect.https_origin())?,
        ],
    )?;
    assert_eq!(
        client
            .read_metadata(&redirect.https_origin(), limits(3)?)
            .await,
        Err(Error::RedirectRejected)
    );
    assert_eq!(origin.requests()?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn injected_builder_cannot_disable_certificate_or_hostname_verification() -> TestResult {
    for certificate in [SERVER, EXPIRED] {
        let origin = Local::listen(Some(certificate), None, OK).await?;
        let cases = [
            (
                Client::builder()
                    .use_rustls_tls()
                    .no_proxy()
                    .tls_built_in_native_certs(false)
                    .tls_built_in_webpki_certs(false),
                origin.https_origin(),
            ),
            (
                custom_root_builder()?.resolve("private-certificate-host.invalid", origin.addr),
                format!(
                    "https://private-certificate-host.invalid:{}",
                    origin.addr.port()
                ),
            ),
        ];
        for (builder, url) in cases {
            let client = DownloadClient::with_http_builder(
                builder
                    .danger_accept_invalid_certs(true)
                    .danger_accept_invalid_hostnames(true),
                [ApprovedOrigin::https(&url)?],
            )?;
            let error = client
                .read_metadata(&url, limits(3)?)
                .await
                .err()
                .ok_or("expected certificate failure")?;
            assert_eq!(error, Error::CertificateFailed);
            assert_private(&error);
        }
        // Explicitly trust the issuer: the expired leaf must still be rejected.
        if certificate == EXPIRED {
            let client = DownloadClient::with_http_builder(
                custom_root_builder()?,
                [ApprovedOrigin::https(&origin.https_origin())?],
            )?;
            assert_eq!(
                client
                    .read_metadata(&origin.https_origin(), limits(3)?)
                    .await,
                Err(Error::CertificateFailed)
            );
        }
        assert!(origin.requests()?.is_empty());
    }
    Ok(())
}
