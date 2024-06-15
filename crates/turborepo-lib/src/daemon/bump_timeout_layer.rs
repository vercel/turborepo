//! timeout_middleware
//!
//! This is middleware for tonic that integrates with bump_timeout to
//! continually reset the timeout when a request is received.

use std::sync::Arc;

use tonic::server::NamedService;
use tower::{Layer, Service};

use super::bump_timeout::BumpTimeout;

/// A layer that resets a <BumpTimeout> when a request is received.
pub struct BumpTimeoutLayer(Arc<BumpTimeout>);

impl BumpTimeoutLayer {
    #[allow(dead_code)]
    pub fn new(timeout: Arc<BumpTimeout>) -> Self {
        Self(timeout)
    }
}

impl<S> Layer<S> for BumpTimeoutLayer {
    type Service = BumpTimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BumpTimeoutService {
            inner,
            timeout: self.0.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BumpTimeoutService<S> {
    inner: S,
    timeout: Arc<BumpTimeout>,
}

impl<S, Request> Service<Request> for BumpTimeoutService<S>
where
    S: Service<Request>,
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

    fn call(&mut self, req: Request) -> Self::Future {
        self.timeout.reset();
        self.inner.call(req)
    }
}

impl<T: NamedService> NamedService for BumpTimeoutService<T> {
    const NAME: &'static str = T::NAME;
}
