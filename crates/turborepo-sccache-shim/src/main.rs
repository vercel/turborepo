//! Experimental spike: a localhost HTTP server that speaks the subset of
//! WebDAV that sccache's `SCCACHE_WEBDAV_ENDPOINT` backend (opendal) uses,
//! translating it to the Turborepo Remote Cache artifacts API.
//!
//! The point being proven: sccache can use Turborepo Remote Cache as its
//! compile-cache storage **without any sccache modifications** — turbo runs
//! this shim locally and points `RUSTC_WRAPPER=sccache` at it, so every
//! rustc invocation is remote-cached with the user's existing turbo
//! credentials.
//!
//! Protocol notes (discovered empirically against sccache 0.16 / opendal):
//! * Keys arrive as `{prefix}/{a}/{b}/{c}/{hash}` — sccache shards keys by
//!   their first three characters. The final path segment is the full key, so
//!   it alone becomes the (flattened) artifact hash.
//! * A `.sccache_check` key is written and read at server startup to probe
//!   read/write access; it flows through the same artifact translation.
//! * Reads are `GET`, existence probes are `HEAD`, writes are `PUT`. Anything
//!   else (`MKCOL`, `PROPFIND`, `OPTIONS`) is acknowledged as a no-op — the
//!   artifact namespace is flat, so "directories" always exist.
//!
//! Every request is logged to stderr as `VERB /path -> status (bytes)` so
//! the same binary doubles as the protocol-discovery tool.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::Response,
};
use turborepo_api_client::{APIClient, CacheClient};
use turborepo_types::SecretString;

struct Shim {
    client: APIClient,
    token: SecretString,
    team_id: Option<String>,
}

/// The artifact hash for a WebDAV path: the final path segment, prefixed to
/// keep sccache artifacts in their own namespace beside turbo's task
/// artifacts. Rejects anything that isn't a plain key so the shim never
/// forwards a malformed path into a URL.
fn artifact_hash(path: &str) -> Option<String> {
    let key = path.rsplit('/').next()?;
    let valid = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    valid.then(|| format!("sccache-{key}"))
}

async fn handle(State(shim): State<Arc<Shim>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let request_length = request
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("0")
        .to_string();

    let (status, body) = respond(&shim, request).await;
    eprintln!(
        "{method} {path} -> {status} (in {request_length} out {} bytes)",
        body.len()
    );

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
}

/// Minimal WebDAV `PROPFIND` multistatus body. opendal stats paths with
/// PROPFIND before reading/writing; directories must report
/// `<D:collection/>` so its write path believes the parent exists.
///
/// opendal 0.55 (sccache's pin) requires `getlastmodified` to be present —
/// its `Prop.getlastmodified` is a non-optional `String` — so a static
/// HTTP-date is always emitted; sccache never consults it.
fn multistatus(href: &str, collection: bool, content_length: Option<u64>) -> Vec<u8> {
    let resourcetype = if collection {
        "<D:resourcetype><D:collection/></D:resourcetype>".to_string()
    } else {
        let length = content_length.unwrap_or(0);
        format!("<D:resourcetype/><D:getcontentlength>{length}</D:getcontentlength>")
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>{href}</D:href>
    <D:propstat>
      <D:prop>
        <D:getlastmodified>Thu, 01 Jan 1970 00:00:00 GMT</D:getlastmodified>
        {resourcetype}
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#
    )
    .into_bytes()
}

async fn respond(shim: &Shim, request: Request) -> (StatusCode, Vec<u8>) {
    let path = request.uri().path().to_string();

    // Directory-shaped paths: the artifact namespace is flat, so every
    // "directory" exists. PROPFIND reports a collection; MKCOL succeeds.
    let is_dir = path.ends_with('/');
    if is_dir || artifact_hash(&path).is_none() {
        return match *request.method() {
            Method::GET | Method::HEAD => (StatusCode::NOT_FOUND, Vec::new()),
            _ if request.method().as_str() == "PROPFIND" => {
                (StatusCode::MULTI_STATUS, multistatus(&path, true, None))
            }
            _ => (StatusCode::CREATED, Vec::new()),
        };
    }

    let Some(hash) = artifact_hash(&path) else {
        return (StatusCode::BAD_REQUEST, Vec::new());
    };

    // PROPFIND on a file path: stat it upstream.
    if request.method().as_str() == "PROPFIND" {
        return match shim
            .client
            .artifact_exists(&hash, &shim.token, shim.team_id.as_deref(), None)
            .await
        {
            Ok(Some(response)) => {
                let length = response
                    .headers()
                    .get("Content-Length")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok());
                (StatusCode::MULTI_STATUS, multistatus(&path, false, length))
            }
            Ok(None) => (StatusCode::NOT_FOUND, Vec::new()),
            Err(error) => {
                eprintln!("stat error for {hash}: {error}");
                (StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
            }
        };
    }

    match *request.method() {
        Method::GET => match shim
            .client
            .fetch_artifact(&hash, &shim.token, shim.team_id.as_deref(), None)
            .await
        {
            Ok(Some(response)) => match response.bytes().await {
                Ok(bytes) => (StatusCode::OK, bytes.to_vec()),
                Err(error) => {
                    eprintln!("fetch body error for {hash}: {error}");
                    (StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
                }
            },
            Ok(None) => (StatusCode::NOT_FOUND, Vec::new()),
            Err(error) => {
                eprintln!("fetch error for {hash}: {error}");
                (StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
            }
        },
        Method::HEAD => match shim
            .client
            .artifact_exists(&hash, &shim.token, shim.team_id.as_deref(), None)
            .await
        {
            Ok(Some(_)) => (StatusCode::OK, Vec::new()),
            Ok(None) => (StatusCode::NOT_FOUND, Vec::new()),
            Err(error) => {
                eprintln!("exists error for {hash}: {error}");
                (StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
            }
        },
        Method::PUT => {
            let bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("body read error for {hash}: {error}");
                    return (StatusCode::BAD_REQUEST, Vec::new());
                }
            };
            let length = bytes.len();
            let stream = tokio_stream::once(Ok(bytes));
            match shim
                .client
                .put_artifact(
                    &hash,
                    stream,
                    length,
                    0,
                    None,
                    &shim.token,
                    shim.team_id.as_deref(),
                    None,
                    None,
                    None,
                )
                .await
            {
                Ok(()) => (StatusCode::CREATED, Vec::new()),
                Err(error) => {
                    eprintln!("put error for {hash}: {error}");
                    (StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
                }
            }
        }
        // MKCOL and friends: the artifact namespace is flat; directories
        // always "exist".
        _ => (StatusCode::CREATED, Vec::new()),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let port: u16 = std::env::var("SHIM_PORT")
        .context("SHIM_PORT is required")?
        .parse()
        .context("SHIM_PORT must be a port number")?;
    let upstream = std::env::var("SHIM_UPSTREAM").context("SHIM_UPSTREAM is required")?;
    let token = SecretString::new(std::env::var("SHIM_TOKEN").context("SHIM_TOKEN is required")?);
    let team_id = std::env::var("SHIM_TEAM_ID").ok();

    let client = APIClient::new(
        &upstream,
        Some(Duration::from_secs(30)),
        None,
        "sccache-shim",
        false,
    )
    .context("failed to construct API client")?;

    let shim = Arc::new(Shim {
        client,
        token,
        team_id,
    });

    let app = Router::new().fallback(handle).with_state(shim);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("failed to bind 127.0.0.1:{port}"))?;
    eprintln!("sccache shim listening on 127.0.0.1:{port} -> {upstream}");
    axum::serve(listener, app).await.context("server error")
}
