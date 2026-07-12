#![doc = include_str!("../README.md")]
#![deny(clippy::all)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod auto;
pub mod certs;
/// Safe removal of files owned by Portless.
pub mod clean;
pub mod cli;
pub mod config;
/// Managed hosts-file entries and hostname resolution checks.
pub mod hosts;
pub mod mdns;
pub mod ngrok;
pub mod pages;
pub mod process;
pub mod proxy;
/// Upstream-compatible, disk-backed route registry.
pub mod routes;
pub mod service;
pub mod tailscale;
pub mod turbo;
pub mod workspace;

pub use proxy::{ProxyError, ProxyOptions, ProxyRoute, ProxyServer, ProxyShutdown, TlsConfig};
pub use routes::{Route, RouteConflict, RouteMetadata, RouteStore};
