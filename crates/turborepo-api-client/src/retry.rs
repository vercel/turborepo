use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use reqwest::{RequestBuilder, Response, StatusCode, header::RETRY_AFTER};
use tokio::{
    sync::Mutex,
    time::{Instant, sleep, timeout_at},
};

use crate::Error;

const MIN_SLEEP_TIME_SECS: u64 = 2;
const MAX_SLEEP_TIME_SECS: u64 = 10;
const RETRY_MAX: u32 = 2;
// Allow a full capped Retry-After (10s) after the first attempt while still
// bounding the total time spent waiting for the coordinator and retry backoff.
const MAX_RETRY_ELAPSED: Duration = Duration::from_secs(20);

/// Shared cooldown for artifact requests from one APIClient/team/token.
/// In-flight requests may complete after another request publishes a 429
/// cooldown.
#[derive(Clone, Default)]
pub(crate) struct RateLimit(Arc<Mutex<Option<Instant>>>);

fn retry_after(response: &Response, now: SystemTime) -> Option<Duration> {
    let value = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    let duration = if let Ok(seconds) = value.parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
        date.signed_duration_since(chrono::DateTime::<chrono::Utc>::from(now))
            .to_std()
            .ok()?
    };
    Some(duration.min(Duration::from_secs(MAX_SLEEP_TIME_SECS)))
}

/// Artifact-only retry path. Requests check the shared cooldown before each
/// send; an already-in-flight request may still complete after a 429. A 429
/// publishes its cooldown even for non-replayable bodies. 5xx and transport
/// errors use ordinary backoff and never extend the shared cooldown.
/// The elapsed budget bounds waiting and retries, not an in-flight HTTP send;
/// that is subject to the request's timeout, if one was configured.
pub(crate) async fn make_rate_limited_request(
    request_builder: RequestBuilder,
    strategy: RetryStrategy,
    rate_limit: RateLimit,
) -> Result<Retry, Error> {
    let deadline = Instant::now() + MAX_RETRY_ELAPSED;
    let mut request_builder = Some(request_builder);
    let mut last_response = None;
    let mut last_error = None;

    for retry_count in 0..RETRY_MAX {
        // Check immediately before sending. The lock is released before HTTP I/O:
        // requests already in flight when a 429 arrives cannot be recalled.
        loop {
            let gate = match timeout_at(deadline, rate_limit.0.lock()).await {
                Ok(gate) => gate,
                Err(_) => return exhausted(last_response, last_error, retry_count),
            };
            let until = *gate;
            if let Some(until) = until
                && until > Instant::now()
            {
                drop(gate);
                if until >= deadline {
                    return exhausted(last_response, last_error, retry_count);
                }
                sleep(until.saturating_duration_since(Instant::now())).await;
                continue;
            }
            break;
        }

        let builder = request_builder.as_ref().and_then(RequestBuilder::try_clone);
        let can_retry = builder.is_some();
        let response = match builder {
            Some(builder) => builder.send().await,
            None => {
                let Some(builder) = request_builder.take() else {
                    return exhausted(last_response, last_error, retry_count);
                };
                builder.send().await
            }
        };
        if let Ok(ref response) = response
            && response.status() == StatusCode::TOO_MANY_REQUESTS
        {
            let backoff = Duration::from_secs(
                2_u64
                    .pow(retry_count)
                    .clamp(MIN_SLEEP_TIME_SECS, MAX_SLEEP_TIME_SECS),
            );
            let delay = retry_after(response, SystemTime::now()).unwrap_or(backoff);
            let until = Instant::now() + delay;
            let mut gate = rate_limit.0.lock().await;
            // Concurrent 429s can arrive out of order; never shorten a
            // cooldown already published by another request.
            *gate = Some(gate.map_or(until, |previous| previous.max(until)));
        }
        match response {
            Ok(response) => {
                let status = response.status();
                if !can_retry
                    || retry_count + 1 == RETRY_MAX
                    || !RetryStrategy::should_retry_status(status)
                {
                    return Ok(Retry::Retried(response, retry_count));
                }
                if status == StatusCode::TOO_MANY_REQUESTS {
                    last_response = Some(response);
                    continue;
                }
                last_response = Some(response);
                last_error = None;
            }
            Err(err) => {
                if !can_retry || !strategy.should_retry(&err) {
                    return Err(err.into());
                }
                last_error = Some(err);
                last_response = None;
            }
        }
        let backoff = Duration::from_secs(
            2_u64
                .pow(retry_count)
                .clamp(MIN_SLEEP_TIME_SECS, MAX_SLEEP_TIME_SECS),
        );
        if Instant::now() + backoff >= deadline {
            return exhausted(last_response, last_error, retry_count + 1);
        }
        sleep(backoff).await;
    }
    exhausted(last_response, last_error, RETRY_MAX)
}

fn exhausted(
    response: Option<Response>,
    error: Option<reqwest::Error>,
    count: u32,
) -> Result<Retry, Error> {
    if let Some(response) = response {
        Ok(Retry::Retried(response, count))
    } else if let Some(error) = error {
        Err(Error::TooManyFailures(Box::new(error)))
    } else {
        Err(Error::RateLimitWaitExceeded)
    }
}

#[derive(Debug)]
pub enum Retry {
    Once(Response),
    #[allow(dead_code)]
    Retried(Response, u32),
}

impl Retry {
    pub fn into_response(self) -> Response {
        match self {
            Retry::Once(response) => response,
            Retry::Retried(response, _) => response,
        }
    }

    #[allow(dead_code)]
    pub fn retry_count(&self) -> Option<u32> {
        match self {
            Retry::Once(_) => None,
            Retry::Retried(_, count) => Some(*count),
        }
    }
}

/// Retries a request until `RETRY_MAX` is reached, the `should_retry_request`
/// function returns false, or the future succeeds. Uses an exponential backoff
/// with a base of 2 to delay between retries.
///
/// # Arguments
///
/// * `request_builder`: The request builder with everything, i.e. headers and
///   body already set. Requests with streaming bodies cannot be cloned, so they
///   can only be sent once and cannot be retried.
/// * `strategy`: The strategy to use for retrying requests.
///
/// returns: Result<Response, Error>
pub(crate) async fn make_retryable_request(
    request_builder: RequestBuilder,
    strategy: RetryStrategy,
) -> Result<Retry, Error> {
    let mut last_error = None;
    for retry_count in 0..RETRY_MAX {
        // A request builder can fail to clone for two reasons:
        // - the URL given was given as a string and isn't a valid URL this can be
        //   mitigated by constructing requests with pre-parsed URLs via Url::parse
        // - the request body is a stream, in this case we'll just send the one request
        //   we have
        let Some(builder) = request_builder.try_clone() else {
            return Ok(Retry::Once(request_builder.send().await?));
        };
        match builder.send().await {
            Ok(value) => {
                if retry_count + 1 == RETRY_MAX
                    || !RetryStrategy::should_retry_status(value.status())
                {
                    return Ok(Retry::Retried(value, retry_count));
                }
            }
            Err(err) => {
                if !strategy.should_retry(&err) {
                    return Err(err.into());
                }
                last_error = Some(err);
            }
        }

        let sleep_period = (2_u64)
            .pow(retry_count)
            .clamp(MIN_SLEEP_TIME_SECS, MAX_SLEEP_TIME_SECS);
        sleep(std::time::Duration::from_secs(sleep_period)).await;
    }

    match last_error {
        Some(error) => Err(Error::TooManyFailures(Box::new(error))),
        None => Err(Error::RetryExhaustedWithoutError),
    }
}

/// A retry strategy. Note that error statuses and TOO_MANY_REQUESTS are always
/// retried.
pub enum RetryStrategy {
    /// Retry in the case of connection issues, but ignore timeouts.
    Connection,
    /// Retry in the case of connection issues and timeouts.
    Timeout,
}

impl RetryStrategy {
    fn should_retry_status(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS
            || (status.is_server_error() && status != StatusCode::NOT_IMPLEMENTED)
    }

    fn should_retry(&self, error: &reqwest::Error) -> bool {
        if error.status().is_some_and(Self::should_retry_status) {
            return true;
        }

        match self {
            RetryStrategy::Connection => error.is_connect(),
            RetryStrategy::Timeout => error.is_timeout(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches,
        time::{Duration, SystemTime},
    };

    use reqwest::StatusCode;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    use crate::{
        Error,
        retry::{
            RETRY_MAX, RateLimit, RetryStrategy, make_rate_limited_request, make_retryable_request,
            retry_after,
        },
    };

    #[tokio::test]
    async fn parses_and_bounds_retry_after() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784111777);
        for (header, expected) in [
            ("0", Some(Duration::ZERO)),
            ("4", Some(Duration::from_secs(4))),
            ("1000000", Some(Duration::from_secs(10))),
            (
                "Sun, 06 Nov 1994 08:49:42 GMT",
                Some(Duration::from_secs(5)),
            ),
            ("Sun, 06 Nov 1994 08:49:32 GMT", None),
            ("invalid", None),
        ] {
            let mock = httpmock::MockServer::start_async().await;
            mock.mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.status(429).header("Retry-After", header);
            })
            .await;
            let response = reqwest::get(mock.url("/")).await.unwrap();
            assert_eq!(retry_after(&response, now), expected, "{header}");
        }
    }

    // A tiny deterministic HTTP server: publish a 429 before starting the
    // second request, which must wait for Retry-After before its first send.
    #[tokio::test(start_paused = true)]
    async fn waits_for_known_cooldown_before_initial_send() {
        // Keep the runtime runnable so its paused clock advances only when
        // explicitly requested, even while the HTTP driver is waiting on I/O.
        let keep_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let keep_running_task = keep_running.clone();
        let keepalive = tokio::spawn(async move {
            while keep_running_task.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (first_received_tx, first_received_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (requests_tx, mut requests_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let mut first_received_tx = Some(first_received_tx);
            let mut release_rx = Some(release_rx);
            let mut count = 0;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                count += 1;
                let number = count;
                let tx = requests_tx.clone();
                let release = if number == 1 { release_rx.take() } else { None };
                let signal = if number == 1 {
                    first_received_tx.take()
                } else {
                    None
                };
                tokio::spawn(async move {
                    let mut buffer = [0; 4096];
                    let _ = stream.read(&mut buffer).await.unwrap();
                    tx.send(number).unwrap();
                    if let Some(signal) = signal {
                        let _ = signal.send(());
                    }
                    if let Some(release) = release {
                        let _ = release.await;
                    }
                    let reply = if number == 1 {
                        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3\r\nContent-Length: \
                         0\r\nConnection: close\r\n\r\n"
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    };
                    stream.write_all(reply.as_bytes()).await.unwrap();
                });
            }
        });

        let http = reqwest::Client::new();
        let rate_limit = RateLimit::default();
        let first = tokio::spawn(make_rate_limited_request(
            http.get(&url),
            RetryStrategy::Timeout,
            rate_limit.clone(),
        ));
        first_received_rx.await.unwrap();
        assert_eq!(requests_rx.recv().await, Some(1));
        release_tx.send(()).unwrap();
        // Wait until the first 429 has published its cooldown.
        for _ in 0..1000 {
            if rate_limit.0.lock().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(rate_limit.0.lock().await.is_some());
        let second = tokio::spawn(make_rate_limited_request(
            http.get(&url),
            RetryStrategy::Timeout,
            rate_limit.clone(),
        ));
        // An unrelated client/key can send during the cooldown.
        let independent = tokio::spawn(make_rate_limited_request(
            http.get(&url),
            RetryStrategy::Timeout,
            RateLimit::default(),
        ));
        assert_eq!(requests_rx.recv().await, Some(2));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(requests_rx.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..1000 {
            if first.is_finished() && second.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(first.is_finished() && second.is_finished());
        assert_eq!(
            independent.await.unwrap().unwrap().into_response().status(),
            StatusCode::OK
        );
        assert_eq!(
            first.await.unwrap().unwrap().into_response().status(),
            StatusCode::OK
        );
        assert_eq!(
            second.await.unwrap().unwrap().into_response().status(),
            StatusCode::OK
        );
        assert_eq!(requests_rx.recv().await, Some(3));
        assert_eq!(requests_rx.recv().await, Some(4));
        server.abort();
        keep_running.store(false, std::sync::atomic::Ordering::Relaxed);
        keepalive.await.unwrap();
    }

    #[tokio::test]
    async fn successful_requests_share_a_key_without_serializing_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (first_received_tx, first_received_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = first_stream.read(&mut buffer).await.unwrap();
            first_received_tx.send(()).unwrap();
            let (mut second_stream, _) = listener.accept().await.unwrap();
            let _ = second_stream.read(&mut buffer).await.unwrap();
            second_stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            release_rx.await.unwrap();
            first_stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let http = reqwest::Client::new();
        let limit = RateLimit::default();
        let first = tokio::spawn(make_rate_limited_request(
            http.get(&url),
            RetryStrategy::Timeout,
            limit.clone(),
        ));
        first_received_rx.await.unwrap();
        let second = tokio::spawn(make_rate_limited_request(
            http.get(&url),
            RetryStrategy::Timeout,
            limit.clone(),
        ));
        // The second must finish while the first still waits for headers.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), second)
                .await
                .expect("second request was serialized")
                .unwrap()
                .unwrap()
                .into_response()
                .status(),
            StatusCode::OK
        );
        assert!(!first.is_finished());
        assert!(limit.0.lock().await.is_none());
        release_tx.send(()).unwrap();
        assert_eq!(
            first.await.unwrap().unwrap().into_response().status(),
            StatusCode::OK
        );
        server.await.unwrap();
    }

    #[test]
    fn scopes_artifact_cooldowns_to_client_team_and_token() {
        let http = reqwest::Client::new();
        let client = crate::APIClient::new_with_client(
            http.clone(),
            "http://localhost",
            None,
            None,
            "test",
            false,
        );
        let other_client =
            crate::APIClient::new_with_client(http, "http://localhost", None, None, "test", false);
        let token = turborepo_types::SecretString::new("one".to_string());
        let other_token = turborepo_types::SecretString::new("two".to_string());
        let limit = client.artifact_rate_limit(&token, Some("team"), None);
        assert!(std::sync::Arc::ptr_eq(
            &limit.0,
            &client
                .clone()
                .artifact_rate_limit(&token, Some("team"), None)
                .0
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &limit.0,
            &client
                .artifact_rate_limit(&other_token, Some("team"), None)
                .0
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &limit.0,
            &client
                .artifact_rate_limit(&token, Some("other-team"), None)
                .0
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &limit.0,
            &other_client
                .artifact_rate_limit(&token, Some("team"), None)
                .0
        ));
    }

    #[tokio::test]
    async fn artifact_preflight_uses_the_same_coordinator() {
        use crate::CacheClient;

        let mock = httpmock::MockServer::start_async().await;
        let options = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::OPTIONS)
                    .path("/v8/artifacts/hash");
                then.status(429).header("Retry-After", "0");
            })
            .await;
        let get = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/v8/artifacts/hash");
                then.status(200);
            })
            .await;
        let client = crate::APIClient::new_with_client(
            reqwest::Client::new(),
            mock.base_url(),
            None,
            None,
            "test",
            true,
        );
        let token = turborepo_types::SecretString::new("token".to_string());
        let response = client
            .fetch_artifact("hash", &token, Some("team_one"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        options.assert_calls_async(2).await;
        get.assert_calls_async(1).await;
        assert!(
            client
                .artifact_rate_limit(&token, Some("team_one"), None)
                .0
                .lock()
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn bounded_wait_does_not_send_a_request() {
        let mock = httpmock::MockServer::start_async().await;
        let request = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.status(200);
            })
            .await;
        let rate_limit = RateLimit::default();
        *rate_limit.0.lock().await = Some(tokio::time::Instant::now() + Duration::from_secs(100));
        let result = make_rate_limited_request(
            reqwest::Client::new().get(mock.url("/")),
            RetryStrategy::Timeout,
            rate_limit,
        )
        .await;
        assert_matches!(result, Err(Error::RateLimitWaitExceeded));
        request.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn server_errors_do_not_publish_cooldowns() {
        let mock = httpmock::MockServer::start_async().await;
        let request = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.status(503).header("Retry-After", "10");
            })
            .await;
        let rate_limit = RateLimit::default();
        let result = make_rate_limited_request(
            reqwest::Client::new().get(mock.url("/")),
            RetryStrategy::Timeout,
            rate_limit.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            result.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(rate_limit.0.lock().await.is_none());
        request.assert_calls_async(2).await;
    }

    #[tokio::test]
    async fn non_replayable_429_still_publishes_cooldown() {
        let mock = httpmock::MockServer::start_async().await;
        let request = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST);
                then.status(429).header("Retry-After", "4");
            })
            .await;
        let body = reqwest::Body::wrap_stream(tokio_stream::once(Ok::<_, std::io::Error>(
            bytes::Bytes::from_static(b"upload"),
        )));
        let rate_limit = RateLimit::default();
        let response = make_rate_limited_request(
            reqwest::Client::new().post(mock.url("/")).body(body),
            RetryStrategy::Connection,
            rate_limit.clone(),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(rate_limit.0.lock().await.unwrap() > tokio::time::Instant::now());
        request.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn handles_too_many_failures() {
        let mock = httpmock::MockServer::start_async().await;
        mock.mock_async(|when, then| {
            when.method(httpmock::Method::GET);
            then.delay(Duration::from_secs(100));
        })
        .await;

        let request_builder = reqwest::Client::new()
            .get(mock.url("/"))
            .timeout(Duration::from_millis(500));
        let result = make_retryable_request(request_builder, RetryStrategy::Timeout).await;

        // Only assert the return type — the mock's call count can be fewer than
        // expected when the timeout fires before httpmock registers the request
        // (e.g. on loaded CI machines).
        assert_matches!(result, Err(Error::TooManyFailures(_)));
    }

    #[tokio::test]
    async fn handles_connection_timeout() {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(10))
            .build()
            .unwrap();

        let request_builder = client.get("http://localhost:1").send().await; // bad port
        let should_retry = RetryStrategy::Connection.should_retry(&request_builder.unwrap_err());

        assert_matches!(should_retry, true);
    }

    #[tokio::test]
    async fn handles_connection_timeout_retries() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(20))
            .connect_timeout(Duration::from_millis(10))
            .build()
            .unwrap();

        let mock = httpmock::MockServer::start_async().await;
        let req = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.delay(Duration::from_secs(100));
            })
            .await;

        let request_builder = client.get(mock.url("/")); // bad port
        let result = make_retryable_request(request_builder, RetryStrategy::Connection).await;

        // we should make at most one request and give up if it times out after
        // connecting
        assert_matches!(result, Err(_));
        req.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn retries_retryable_http_statuses() {
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            let mock = httpmock::MockServer::start_async().await;
            let req = mock
                .mock_async(|when, then| {
                    when.method(httpmock::Method::GET);
                    then.status(status.as_u16()).body("retryable error");
                })
                .await;

            let request_builder = reqwest::Client::new().get(mock.url("/"));
            let result = make_retryable_request(request_builder, RetryStrategy::Timeout)
                .await
                .unwrap();

            req.assert_calls_async(RETRY_MAX as usize).await;
            let response = result.into_response();
            assert_eq!(response.status(), status);
            assert_eq!(response.text().await.unwrap(), "retryable error");
        }
    }

    #[tokio::test]
    async fn does_not_retry_non_retryable_http_statuses() {
        let mock = httpmock::MockServer::start_async().await;
        let req = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.status(StatusCode::FORBIDDEN.as_u16())
                    .body("forbidden");
            })
            .await;

        let request_builder = reqwest::Client::new().get(mock.url("/"));
        let result = make_retryable_request(request_builder, RetryStrategy::Timeout)
            .await
            .unwrap();

        req.assert_calls_async(1).await;
        let response = result.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.text().await.unwrap(), "forbidden");
    }
}
