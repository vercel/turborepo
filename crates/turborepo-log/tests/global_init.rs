//! Integration tests for the global logger lifecycle.
//!
//! Each `tests/*.rs` file is compiled as a separate binary, so the
//! `OnceLock`-based global logger can be tested without conflicting
//! with other test files.

use std::sync::Arc;

use turborepo_log::{Logger, Source, flush, init, log, sinks::collector::CollectorSink};

#[test]
fn full_lifecycle() {
    let collector = Arc::new(CollectorSink::new());

    // Handle created before init — the global logger isn't set yet,
    // so events emitted RIGHT NOW are dropped.
    let early_handle = log(Source::turbo("early"));
    early_handle.warn("before init").emit();
    assert_eq!(collector.events().len(), 0);

    // Builder created before init — holds LogResolver::Global, which
    // defers resolution to emit() time.
    let early_builder = early_handle.warn("built before init");

    // Free function builder created before init — also defers.
    let early_free = turborepo_log::warn(Source::turbo("free"), "free before init");

    // Initialize the global logger.
    assert!(init(Logger::new(vec![Box::new(collector.clone())])).is_ok());

    // Handle created after init works immediately.
    let handle = log(Source::turbo("test"));
    handle.warn("test warning").field("key", "value").emit();
    assert_eq!(collector.events().len(), 1);
    assert_eq!(collector.events()[0].message(), "test warning");

    // The early handle now works too — deferred resolution means it
    // finds the global logger on the next emit() call.
    early_handle.info("now works").emit();
    assert_eq!(collector.events().len(), 2);

    // Builders created before init also work — lazy resolution at
    // emit() time finds the now-initialized global logger.
    early_builder.emit();
    assert_eq!(collector.events().len(), 3);
    assert_eq!(collector.events()[2].message(), "built before init");

    early_free.emit();
    assert_eq!(collector.events().len(), 4);
    assert_eq!(collector.events()[3].message(), "free before init");

    // Double init fails with InitError.
    let result = init(Logger::new(vec![]));
    assert!(result.is_err());

    // Free functions work.
    turborepo_log::warn(Source::turbo("free"), "free warning").emit();
    assert_eq!(collector.events().len(), 5);

    // All levels work.
    turborepo_log::info(Source::turbo("free"), "info").emit();
    turborepo_log::error(Source::turbo("free"), "error").emit();
    assert_eq!(collector.events().len(), 7);

    // Flush doesn't panic.
    flush();
}
