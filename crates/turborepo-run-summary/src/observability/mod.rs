//! Observability abstraction layer for Turborepo runs.
//!
//! This module provides a generic interface for recording metrics and telemetry
//! from completed Turborepo runs. The public API consists of:
//!
//! - [`Handle`]: An opaque handle to an observability backend
//! - [`try_init`]: Factory function to initialize an observability backend
//!
//! Currently, only OpenTelemetry is supported as a backend. Additional backends
//! can be added by:
//!
//! 1. Creating a new submodule (e.g., `observability::prometheus`) that
//!    implements the internal `RunObserver` trait
//! 2. Adding a `try_init_*` function that returns `Option<Handle>`
//! 3. Updating `try_init` to dispatch to the appropriate backend based on
//!    config
//!
//! The abstraction ensures that callers only depend on the generic `Handle`
//! type and are not coupled to any specific observability implementation.

use std::sync::Arc;

use turborepo_config::ExperimentalObservabilityOptions;

use crate::RunSummary;

#[cfg(feature = "otel")]
mod otel;

#[cfg(not(feature = "otel"))]
mod otel_disabled;

#[cfg(not(feature = "otel"))]
use otel_disabled as otel;

/// Trait for observing completed Turborepo runs, allowing different
/// observability backends like OpenTelemetry to be plugged in.
pub(crate) trait RunObserver: Send + Sync {
    /// Record metrics for a completed run.
    fn record(&self, summary: &RunSummary<'_>);
    /// Shutdown the observer, flushing any pending metrics.
    /// This is called when the handle is dropped or explicitly shut down.
    fn shutdown(&self);
}

/// A generic handle to an observability backend.
///
/// This is the only type that callers need to know about.
/// The concrete backend implementation is hidden behind the `RunObserver`
/// trait.
#[derive(Clone)]
pub struct Handle {
    pub(crate) inner: Arc<dyn RunObserver>,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle").finish_non_exhaustive()
    }
}

impl Handle {
    /// Record metrics for a completed run.
    pub fn record(&self, summary: &RunSummary<'_>) {
        self.inner.record(summary);
    }

    /// Shutdown the observer, flushing any pending metrics.
    pub fn shutdown(&self) {
        // We only have an Arc<dyn RunObserver>, so we delegate shutdown to the trait
        // and let the concrete implementation handle any shared references.
        self.inner.shutdown();
    }

    /// Initialize an observability handle from configuration options.
    ///
    /// Returns `None` if observability is disabled or misconfigured.
    ///
    /// Currently, this only supports OpenTelemetry backends configured via
    /// `ExperimentalObservabilityOptions` (from
    /// `experimentalObservability.otel` in turbo.json or via environment
    /// variables/CLI flags). In the future, this may dispatch to different
    /// backends based on the configuration provided.
    pub fn try_init(
        options: &ExperimentalObservabilityOptions,
        token: Option<&str>,
    ) -> Option<Self> {
        if let Some(otel_options) = options.otel.as_ref() {
            otel::try_init_otel(otel_options, token)
        } else {
            None
        }
    }
}
