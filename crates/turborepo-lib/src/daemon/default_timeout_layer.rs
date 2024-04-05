//! default timeout layer
//!
//! This module provides some basic middleware that aims to
//! improve the flexibility of the daemon server by doing
//! two things:
//!
//! a) remove the server-wide timeout of 100ms in favour of
//!    a less aggressive 30s. the way tonic works is the
//!    lowest timeout (server vs request-specific) is always
//!    used meaning clients' timeout requests were ignored
//!    if set to >100ms
//! b) add a middleware to reinstate the timeout, if the
//!    client does not specify it, defaulting to 100ms for
//!    'non-blocking' calls (requests in the hot path for
//!    a run of turbo), and falling back to the server
//!    limit for blocking ones (useful in cases like the
//!    LSP)
//!
//! With this in place, it means that clients can specify
//! a timeout that it wants (as long as it is less than 30s),
//! and the server has sane defaults

use std::time::Duration;

use tonic::{codegen::http::Request, server::NamedService, transport::Body};
use tower::{Layer, Service};

#[derive(Clone, Debug)]
pub struct DefaultTimeoutService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for DefaultTimeoutService<S>
where
    S: Service<Request<Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        if !req.uri().path().ends_with("Blocking") {
            req.headers_mut()
                .entry("grpc-timeout")
                .or_insert_with(move || {
                    let dur = Duration::from_millis(100);
                    tonic::codegen::http::HeaderValue::from_str(&format!("{}u", dur.as_micros()))
                        .expect("numbers are always valid ascii")
                });
        };

        self.inner.call(req)
    }
}

/// Provides a middleware that sets a default timeout for
/// non-blocking calls. See the module documentation for
/// more information.
#[derive(Clone, Debug)]
pub struct DefaultTimeoutLayer;

impl<S> Layer<S> for DefaultTimeoutLayer {
    type Service = DefaultTimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DefaultTimeoutService { inner }
    }
}

impl<T: NamedService> NamedService for DefaultTimeoutService<T> {
    const NAME: &'static str = T::NAME;
}

#[cfg(test)]
mod test {
    use std::{
        str::FromStr,
        sync::{Arc, Mutex},
    };

    use axum::http::HeaderValue;
    use test_case::test_case;

    use super::*;

    #[test_case("/ExampleBlocking", None, None ; "no default for blocking calls")]
    #[test_case("/Example", None, Some("100000u") ; "default for non-blocking calls")]
    #[test_case("/Example", Some("200u"), Some("200u") ; "respect client preference")]
    #[tokio::test]
    async fn overrides_timeout_for_non_blocking(
        path: &str,
        timeout: Option<&str>,
        expected: Option<&str>,
    ) {
        #[derive(Clone, Debug)]
        struct MockService(Arc<Mutex<Option<String>>>);

        impl Service<Request<Body>> for MockService {
            type Response = ();
            type Error = ();
            type Future = impl std::future::Future<Output = Result<(), ()>>;

            fn poll_ready(
                &mut self,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn call(&mut self, req: Request<Body>) -> Self::Future {
                // get the content of the header
                let header = self.0.clone();
                async move {
                    let mut header = header.lock().unwrap();
                    *header = req
                        .headers()
                        .get("grpc-timeout")
                        .map(|h| h.to_str().unwrap().to_string());
                    Ok(())
                }
            }
        }

        let inner = MockService(Arc::new(Mutex::new(None)));
        let mut svc = DefaultTimeoutLayer.layer(inner.clone());
        let mut req = Request::new(Body::empty());
        let uri = req.uri_mut();
        *uri = tonic::codegen::http::Uri::from_str(path).unwrap();
        if let Some(timeout) = timeout {
            req.headers_mut()
                .insert("grpc-timeout", HeaderValue::from_str(timeout).unwrap());
        }

        svc.call(req).await.unwrap();

        let header = inner.0.lock().unwrap();

        assert_eq!(header.as_deref(), expected);
    }
}
