use reqwest::{RequestBuilder, Response, StatusCode};
use tokio::time::sleep;

use crate::Error;

const MIN_SLEEP_TIME_SECS: u64 = 2;
const MAX_SLEEP_TIME_SECS: u64 = 10;
const RETRY_MAX: u32 = 2;

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
///   body already set. NOTE: This must be cloneable, so no streams are allowed.
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
            Ok(value) => return Ok(Retry::Retried(value, retry_count)),
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

    Err(Error::TooManyFailures(Box::new(last_error.unwrap())))
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
    fn should_retry(&self, error: &reqwest::Error) -> bool {
        if let Some(status) = error.status() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }

            if status.as_u16() >= 500 && status.as_u16() != 501 {
                return true;
            }
        }

        match self {
            RetryStrategy::Connection => error.is_connect(),
            RetryStrategy::Timeout => error.is_timeout(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, time::Duration};

    use crate::{
        retry::{make_retryable_request, RetryStrategy},
        Error,
    };

    #[tokio::test]
    async fn handles_too_many_failures() {
        let mock = httpmock::MockServer::start_async().await;
        let req = mock
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET);
                then.delay(Duration::from_secs(100));
            })
            .await;

        let request_builder = reqwest::Client::new()
            .get(mock.url("/"))
            .timeout(Duration::from_millis(10));
        let result = make_retryable_request(request_builder, RetryStrategy::Timeout).await;

        req.assert_hits_async(2).await;
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
        req.assert_hits_async(1).await;
    }
}
