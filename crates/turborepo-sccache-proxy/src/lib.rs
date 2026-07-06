//! A local HTTP proxy that exposes the Turborepo Remote Cache as an
//! sccache-compatible storage backend.
//!
//! sccache's `webdav` storage backend only needs plain `GET`/`PUT` (plus
//! `HEAD` for stats) against `{endpoint}/{key}` with bearer-token
//! authentication — no PROPFIND or other WebDAV verbs are on its read/write
//! path. This proxy accepts those requests on a loopback listener and
//! translates them into Remote Cache artifact API calls, so per-compilation
//! -unit rustc results ride the same authenticated cache plumbing as task
//! artifacts. Cache objects are fetched lazily at rustc invocation time:
//! nothing needs to be restored into the environment before a task starts.
//!
//! # Endpoint stability
//!
//! The sccache client daemonizes a background server on first use, and that
//! server captures its storage configuration (endpoint and token) at
//! startup, then outlives the `turbo run` that spawned it. If the endpoint
//! changed between runs, the persistent sccache server would keep talking to
//! a dead port. Two things keep the endpoint stable across runs:
//!
//! * The listen port is derived deterministically from the repository root
//!   ([`derive_port`]), so every run of the same repository binds the same
//!   port.
//! * The bearer token is persisted per repository ([`load_or_create_token`]),
//!   so a long-lived sccache server keeps authenticating successfully.
//!
//! Trailing writes from the sccache server after `turbo` shuts the proxy
//! down fail softly: sccache treats storage errors as cache misses and
//! logs them without failing compilation.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::head,
};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tracing::{debug, warn};
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::{APIAuth, APIClient, CacheClient};
use turborepo_types::SecretString;

/// Ports are derived into this range. It sits inside the IANA "registered"
/// range, away from the ephemeral range OSes assign outbound connections
/// from, so a derived port is unlikely to be transiently occupied.
const PORT_RANGE_START: u16 = 42000;
const PORT_RANGE_LEN: u16 = 3000;

/// Version prefix folded into artifact ids so the key scheme can change
/// without colliding with objects written by earlier versions.
const KEY_SCHEME: &str = "sccache-proxy-v1:";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to bind sccache proxy to 127.0.0.1:{port}: {source}")]
    Bind {
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to read or create the sccache proxy token: {0}")]
    Token(#[source] std::io::Error),
    #[error("sccache proxy server error: {0}")]
    Serve(#[source] std::io::Error),
}

/// Derive the proxy's stable listen port for a repository. Deterministic on
/// the repository root path so consecutive runs (and the persistent sccache
/// server they share) agree on the endpoint.
pub fn derive_port(repo_root: &AbsoluteSystemPath) -> u16 {
    let digest = Sha256::digest(repo_root.as_str().as_bytes());
    // Two bytes of the digest are ample for a 3000-slot range.
    let n = u16::from_be_bytes([digest[0], digest[1]]);
    PORT_RANGE_START + (n % PORT_RANGE_LEN)
}

/// Load the per-repository bearer token, creating it on first use.
///
/// The token authenticates loopback requests to the proxy so arbitrary
/// local processes cannot read or write the team's remote cache through it.
/// It must be stable across runs (see the module docs), so it is persisted
/// at `token_path` with owner-only permissions rather than generated per
/// run.
pub fn load_or_create_token(token_path: &AbsoluteSystemPath) -> Result<String, Error> {
    match token_path.read_existing_to_string().map_err(Error::Token)? {
        Some(existing) if !existing.trim().is_empty() => Ok(existing.trim().to_owned()),
        _ => {
            let token = generate_token();
            token_path.ensure_dir().map_err(Error::Token)?;
            token_path
                .create_with_contents_secret(token.as_bytes())
                .map_err(Error::Token)?;
            Ok(token)
        }
    }
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Map an sccache storage key (an arbitrary path) to a Remote Cache artifact
/// id. sccache keys are opaque to us; hashing keeps the id well-formed
/// regardless of the key's shape and namespaces proxy objects away from task
/// artifacts.
fn artifact_id_for_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_SCHEME.as_bytes());
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

struct ProxyState {
    client: APIClient,
    auth: APIAuth,
    /// Expected `Authorization` header value: `Bearer {token}`.
    expected_authorization: String,
}

impl ProxyState {
    fn token(&self) -> &SecretString {
        &self.auth.token
    }

    fn team_id(&self) -> Option<&str> {
        self.auth.team_id.as_deref()
    }

    fn team_slug(&self) -> Option<&str> {
        self.auth.team_slug.as_deref()
    }
}

/// The proxy server, bound but not yet serving. Construct with
/// [`SccacheProxyServer::bind`], hand the endpoint to sccache via
/// `SCCACHE_WEBDAV_ENDPOINT`, and drive it with
/// [`SccacheProxyServer::run`].
pub struct SccacheProxyServer {
    listener: TcpListener,
    router: Router,
    port: u16,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl SccacheProxyServer {
    /// Bind the proxy on `127.0.0.1:port`. `token` is the bearer token
    /// requests must present (see [`load_or_create_token`]).
    pub async fn bind(
        port: u16,
        client: APIClient,
        auth: APIAuth,
        token: &str,
    ) -> Result<Self, Error> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|source| Error::Bind { port, source })?;
        let port = listener
            .local_addr()
            .map_err(|source| Error::Bind { port, source })?
            .port();

        let state = Arc::new(ProxyState {
            client,
            auth,
            expected_authorization: format!("Bearer {token}"),
        });
        let router = Router::new()
            .route("/{*key}", head(handle_head).get(handle_get).put(handle_put))
            .with_state(state);

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        Ok(Self {
            listener,
            router,
            port,
            shutdown_tx,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The endpoint value for `SCCACHE_WEBDAV_ENDPOINT`.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A handle that shuts the server down gracefully when signalled.
    pub fn shutdown_handle(&self) -> tokio::sync::broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Serve until the shutdown handle is signalled.
    pub async fn run(self) -> Result<(), Error> {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .map_err(Error::Serve)
    }
}

fn authorized(state: &ProxyState, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.expected_authorization)
}

async fn handle_get(
    State(state): State<Arc<ProxyState>>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = artifact_id_for_key(&key);
    match state
        .client
        .fetch_artifact(&id, state.token(), state.team_id(), state.team_slug())
        .await
    {
        Ok(Some(response)) => {
            debug!("sccache proxy hit for key {key}");
            Body::from_stream(response.bytes_stream()).into_response()
        }
        Ok(None) => {
            debug!("sccache proxy miss for key {key}");
            StatusCode::NOT_FOUND.into_response()
        }
        Err(err) => {
            warn!("sccache proxy fetch failed for key {key}: {err}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

async fn handle_head(
    State(state): State<Arc<ProxyState>>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = artifact_id_for_key(&key);
    match state
        .client
        .artifact_exists(&id, state.token(), state.team_id(), state.team_slug())
        .await
    {
        Ok(Some(_)) => StatusCode::OK.into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            warn!("sccache proxy stat failed for key {key}: {err}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

async fn handle_put(
    State(state): State<Arc<ProxyState>>,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: bytes::Bytes,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = artifact_id_for_key(&key);
    let len = body.len();
    let stream = futures::stream::once(async move { Ok(body) });
    match state
        .client
        .put_artifact(
            &id,
            stream,
            len,
            0,
            None,
            state.token(),
            state.team_id(),
            state.team_slug(),
            None,
            None,
        )
        .await
    {
        Ok(()) => {
            debug!("sccache proxy stored key {key} ({len} bytes)");
            StatusCode::OK.into_response()
        }
        Err(err) => {
            warn!("sccache proxy store failed for key {key}: {err}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_auth() -> APIAuth {
        APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("remote-token".to_string()),
            team_slug: None,
        }
    }

    async fn start_backend() -> (u16, tokio::task::JoinHandle<()>) {
        let port = port_scanner::request_open_port().expect("open port");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = turborepo_vercel_api_mock::start_test_server(port, Some(ready_tx)).await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
            .await
            .expect("mock server timed out")
            .expect("mock server failed to start");
        (port, handle)
    }

    fn api_client(backend_port: u16) -> APIClient {
        APIClient::new(
            format!("http://localhost:{backend_port}"),
            Some(std::time::Duration::from_secs(10)),
            None,
            "2.0.0",
            true,
        )
        .expect("api client")
    }

    async fn start_proxy(backend_port: u16, bearer: &str) -> (SccacheProxyServer, u16) {
        // Port 0: tests must not collide on the derived range.
        let server = SccacheProxyServer::bind(0, api_client(backend_port), test_auth(), bearer)
            .await
            .expect("bind proxy");
        let port = server.port();
        (server, port)
    }

    #[test]
    fn key_mapping_is_stable_and_key_dependent() {
        let a = artifact_id_for_key("a/b/0123abc");
        let b = artifact_id_for_key("a/b/0123abd");
        assert_eq!(a, artifact_id_for_key("a/b/0123abc"));
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn derived_port_is_stable_and_in_range() {
        let root = if cfg!(windows) {
            AbsoluteSystemPath::new("C:\\some\\repo").expect("path")
        } else {
            AbsoluteSystemPath::new("/some/repo").expect("path")
        };
        let port = derive_port(root);
        assert_eq!(port, derive_port(root));
        assert!((PORT_RANGE_START..PORT_RANGE_START + PORT_RANGE_LEN).contains(&port));
    }

    #[test]
    fn token_persists_across_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = turbopath::AbsoluteSystemPathBuf::try_from(dir.path())
            .expect("abs path")
            .join_components(&[".turbo", "sccache-proxy-token"]);
        let first = load_or_create_token(&path).expect("create token");
        let second = load_or_create_token(&path).expect("reload token");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[tokio::test]
    async fn round_trip_through_proxy() {
        let (backend_port, backend) = start_backend().await;
        let bearer = "local-proxy-token";
        let (server, port) = start_proxy(backend_port, bearer).await;
        let shutdown = server.shutdown_handle();
        let proxy = tokio::spawn(server.run());

        let http = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}/6/2/sccache-key");

        // Miss before write.
        let resp = http
            .get(&base)
            .bearer_auth(bearer)
            .send()
            .await
            .expect("get");
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

        // Write.
        let resp = http
            .put(&base)
            .bearer_auth(bearer)
            .body("compiled object bytes")
            .send()
            .await
            .expect("put");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        // Stat and read back.
        let resp = http
            .head(&base)
            .bearer_auth(bearer)
            .send()
            .await
            .expect("head");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let resp = http
            .get(&base)
            .bearer_auth(bearer)
            .send()
            .await
            .expect("get");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.bytes().await.expect("body"), "compiled object bytes");

        // A different key is still a miss.
        let resp = http
            .get(format!("http://127.0.0.1:{port}/6/2/other-key"))
            .bearer_auth(bearer)
            .send()
            .await
            .expect("get");
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

        let _ = shutdown.send(());
        let _ = proxy.await;
        backend.abort();
    }

    #[tokio::test]
    async fn rejects_missing_or_wrong_bearer_token() {
        let (backend_port, backend) = start_backend().await;
        let (server, port) = start_proxy(backend_port, "correct-token").await;
        let shutdown = server.shutdown_handle();
        let proxy = tokio::spawn(server.run());

        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/some/key");

        for request in [
            http.get(&url),
            http.get(&url).bearer_auth("wrong-token"),
            http.put(&url).body("data"),
            http.head(&url),
        ] {
            let resp = request.send().await.expect("request");
            assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
        }

        let _ = shutdown.send(());
        let _ = proxy.await;
        backend.abort();
    }
}
