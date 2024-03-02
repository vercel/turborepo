use reqwest::{RequestBuilder, Response, StatusCode};
use tokio::time::sleep;

use crate::Error;

const MIN_SLEEP_TIME_SECS: u64 = 2;
const MAX_SLEEP_TIME_SECS: u64 = 10;
const RETRY_MAX: u32 = 2;

/// Retries a request until `RETRY_MAX` is reached, the `should_retry_request`
/// function returns false, or the future succeeds. Uses an exponential backoff
/// with a base of 2 to delay between retries.
///
/// # Arguments
///
/// * `request_builder`: The request builder with everything, i.e. headers and
///   body already set. NOTE: This must be cloneable, so no streams are allowed.
///
/// returns: Result<Response, Error>
pub(crate) async fn make_retryable_request(
    request_builder: RequestBuilder,
) -> Result<Response, Error> {
    let mut last_error = None;
    for retry_count in 0..RETRY_MAX {
        // A request builder can fail to clone for two reasons:
        // - the URL given was given as a string and isn't a valid URL this can be
        //   mitigated by constructing requests with pre-parsed URLs via Url::parse
        // - the request body is a stream, in this case we'll just send the one request
        //   we have
        let Some(builder) = request_builder.try_clone() else {
            return Ok(request_builder.send().await?);
        };
        match builder.send().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !should_retry_request(&err) {
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

fn should_retry_request(error: &reqwest::Error) -> bool {
    if let Some(status) = error.status() {
        if status == StatusCode::TOO_MANY_REQUESTS {
            return true;
        }

        if status.as_u16() >= 500 && status.as_u16() != 501 {
            return true;
        }
    }

    false
}
