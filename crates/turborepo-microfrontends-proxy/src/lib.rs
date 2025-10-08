#![deny(clippy::all)]

mod error;
mod proxy;
mod router;

pub use error::{ErrorPage, ProxyError};
pub use proxy::ProxyServer;
pub use router::{RouteMatch, Router};
