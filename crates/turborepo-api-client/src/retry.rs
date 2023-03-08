use std::future::Future;

use tokio::time::sleep;

use crate::error::Error;

const MIN_SLEEP_TIME_SECS: u64 = 2;
const MAX_SLEEP_TIME_SECS: u64 = 10;

/// Retries a future until `max_retries` is reached, the `should_retry` function
/// returns false, or the future succeeds. Uses an exponential backoff with a
/// base of 2 to delay between retries.
///
/// # Arguments
///
/// * `max_retries`: Maximum number of retries
/// * `future_generator`: Function to call to generate the future for each retry
/// * `should_retry`: Determines if a retry should be attempted based on the
///   error
///
/// returns: Result<T, Error>
pub async fn retry_future<T, E: Into<Error>, F: Future<Output = Result<T, E>>>(
    max_retries: u32,
    future_generator: impl Fn() -> F,
    should_retry: impl Fn(&E) -> bool,
) -> Result<T, Error> {
    let mut last_error = None;
    for retry_count in 0..max_retries {
        let future = future_generator();
        match future.await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !should_retry(&err) {
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

    Err(Error::TooManyFailures(Box::new(last_error.unwrap().into())))
}
