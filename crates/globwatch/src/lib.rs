//! A wrapper around notify that allows for glob-based watching.
//!
//! ## What is flushing?
//!
//! On certain filesystems, file events are not guaranteed to be delivered in
//! the correct order, or on time. This can cause issues when trying to
//! determine if a file has changed, as we don't want to register a watcher
//! for a file if we are not 'up to date'. The flushing mechanism allows us to
//! watch for a full round trip through the filesystem to ensure the watcher is
//! up to date.

#![deny(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    clippy::unwrap_used,
    unused_must_use,
    unsafe_code
)]
#![feature(drain_filter)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc, Mutex},
};

use futures::{channel::oneshot, future::Either, Stream, StreamExt as _};
use itertools::Itertools;
use merge_streams::StreamExt as _;
pub use notify::{Error, Event, Watcher};
pub use stop_token::{stream::StreamExt, StopSource, StopToken, TimedOutError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{event, span, trace, warn, Id, Level, Span};

/// A wrapper around notify that allows for glob-based watching.
#[derive(Debug)]
pub struct GlobWatcher<T: Watcher> {
    watcher: Arc<Mutex<T>>,
    stream: UnboundedReceiver<Event>,
    flush_dir: PathBuf,

    config: UnboundedReceiver<WatcherCommand>,
}

impl GlobWatcher<notify::RecommendedWatcher> {
    /// Create a new watcher, using the given flush directory as a temporary
    /// storage when flushing file events. For more information on flushing,
    /// see the module-level documentation.
    #[tracing::instrument]
    pub fn new(flush_dir: PathBuf) -> Result<(Self, GlobSender), Error> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (tconf, rconf) = tokio::sync::mpsc::unbounded_channel();

        // even if this fails, we may still be able to continue
        std::fs::create_dir_all(&flush_dir).ok();

        let mut watcher = notify::recommended_watcher(move |event: Result<Event, Error>| {
            let span = span!(tracing::Level::TRACE, "watcher");
            let _ = span.enter();

            let result = event.map(|e| {
                trace!(parent: &span, "sending event: {:?}", e);
                let tx = tx.clone();
                futures::executor::block_on(async move { tx.send(e) })
            });

            match result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    warn!(parent: &span, "watch server closed: {:?}", e);
                }
                Err(e) => {
                    warn!(parent: &span, "error from notify: {:?}", e);
                }
            }
        })?;

        watcher.watch(flush_dir.as_path(), notify::RecursiveMode::Recursive)?;

        Ok((
            Self {
                watcher: Arc::new(Mutex::new(watcher)),
                flush_dir,
                stream: rx,
                config: rconf,
            },
            GlobSender(tconf),
        ))
    }
}

impl<T: Watcher> GlobWatcher<T> {
    /// Convert the watcher into a stream of events,
    /// handling config changes and flushing transparently.
    #[tracing::instrument(skip(self))]
    pub fn into_stream(
        self,
        token: stop_token::StopToken,
    ) -> impl Stream<Item = Result<Event, TimedOutError>> {
        let flush_id = Arc::new(AtomicU64::new(1));
        let flush_dir = Arc::new(self.flush_dir.clone());
        let flush = Arc::new(Mutex::new(HashMap::<u64, oneshot::Sender<()>>::new()));
        Box::pin(
            UnboundedReceiverStream::new(self.stream)
                .map(Either::Left)
                .merge(UnboundedReceiverStream::new(self.config).map(Either::Right))
                .filter_map(move |f| {
                    let span = span!(tracing::Level::TRACE, "stream_processor");
                    let _ = span.enter();
                    let watcher = self.watcher.clone();
                    let flush_id = flush_id.clone();
                    let flush_dir = flush_dir.clone();
                    let flush = flush.clone();
                    async move {
                        match f {
                            Either::Left(mut e) => {
                                for flush_id in e
                                    .paths
                                    .drain_filter(|p| p.starts_with(flush_dir.as_path()))
                                    .filter_map(|p| {
                                        get_flush_id(
                                            p.strip_prefix(flush_dir.as_path())
                                                .expect("confirmed above"),
                                        )
                                    })
                                {
                                    trace!("flushing {:?}", flush);
                                    if let Some(tx) =
                                        flush.lock().expect("no panic").remove(&flush_id)
                                    {
                                        // if this fails, it just means the requestor has gone away
                                        // and we can ignore it
                                        tx.send(()).ok();
                                    }
                                }

                                if e.paths.is_empty() {
                                    None
                                } else {
                                    event!(parent: &span, Level::TRACE, "yielding {:?}", e);
                                    Some(e)
                                }
                            }
                            Either::Right(WatcherCommand::Flush(tx)) => {
                                // create file in flush dir
                                let flush_id =
                                    flush_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                let flush_file = flush_dir.join(format!("{}", flush_id));
                                if let Err(e) = std::fs::File::create(flush_file) {
                                    warn!("failed to create flush file: {}", e);
                                } else {
                                    flush.lock().expect("no panic").insert(flush_id, tx);
                                }
                                None
                            }
                            Either::Right(WatcherCommand::Watcher(change)) => {
                                event!(parent: &span, Level::TRACE, "change {:?}", change);
                                Self::handle_config_change(
                                    &mut watcher.lock().expect("no panic"),
                                    change,
                                );
                                None
                            }
                        }
                    }
                })
                .timeout_at(token),
        )
    }

    #[tracing::instrument(skip(watcher))]
    fn handle_config_change(watcher: &mut T, config: WatcherChange) {
        match config {
            WatcherChange::Include(glob, _) => {
                for p in glob_to_paths(&glob) {
                    if let Err(e) = watcher.watch(&p, notify::RecursiveMode::Recursive) {
                        warn!("failed to watch {:?}: {}", p, e);
                    }
                }
            }
            WatcherChange::Exclude(glob, _) => {
                for p in glob_to_paths(&glob) {
                    // we don't care if this fails, it's just a best-effort
                    watcher.unwatch(&p).ok();
                }
            }
        }
    }
}

fn get_flush_id(relative_path: &Path) -> Option<u64> {
    relative_path
        .file_name()
        .and_then(|p| p.to_str())
        .and_then(|p| p.parse().ok())
}

/// A configuration change to the watcher.
#[derive(Debug)]
pub enum WatcherCommand {
    /// A change to the watcher configuration.
    Watcher(WatcherChange),
    /// A request to flush the watcher.
    Flush(oneshot::Sender<()>),
}

/// A change to the watcher configuration.
///
/// This is used to communicate changes to the watcher
/// from other threads. Can optionally contain the span
/// that the change was made in, for tracing purposes.
#[derive(Debug)]
pub enum WatcherChange {
    /// Register a glob to be included by the watcher.
    Include(String, Option<Id>),
    /// Register a glob to be excluded by the watcher.
    Exclude(String, Option<Id>),
}

/// A sender for watcher configuration changes.
#[derive(Debug, Clone)]
pub struct GlobSender(UnboundedSender<WatcherCommand>);

/// The server is no longer running.
#[derive(Debug, Copy, Clone)]
pub struct ConfigError;

impl GlobSender {
    /// Register a glob to be included by the watcher.
    #[tracing::instrument(skip(self))]
    pub async fn include(&self, glob: String) -> Result<(), ConfigError> {
        trace!("including {:?}", glob);
        self.0
            .send(WatcherCommand::Watcher(WatcherChange::Include(
                glob,
                Span::current().id(),
            )))
            .map_err(|_| ConfigError)
    }

    /// Register a glob to be excluded by the watcher.
    #[tracing::instrument(skip(self))]
    pub async fn exclude(&self, glob: String) -> Result<(), ConfigError> {
        trace!("excluding {:?}", glob);
        self.0
            .send(WatcherCommand::Watcher(WatcherChange::Exclude(
                glob,
                Span::current().id(),
            )))
            .map_err(|_| ConfigError)
    }

    /// Await a full filesystem flush from the watcher.
    pub async fn flush(&self) -> Result<(), ConfigError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(WatcherCommand::Flush(tx))
            .map_err(|_| ConfigError)?;
        rx.await.map_err(|_| ConfigError)
    }
}

/// Gets the minimum set of paths that can be watched for a given glob,
/// specified in minimatch glob syntax.
///
/// note: it is currently extremely conservative, handling only **, braces, and
/// ?. any other case watches the entire directory.
fn glob_to_paths(glob: &str) -> Vec<PathBuf> {
    let mut chunks = vec![];

    for chunk in glob.split('/') {
        if chunk.contains('*') || chunk.contains('[') || chunk.contains(']') {
            break;
        }

        if chunk.starts_with('{') && chunk.ends_with('}') {
            return chunk[1..chunk.len() - 1]
                .split(',')
                .map(|c| chunks.iter().chain(std::iter::once(&c)).collect())
                .collect();
        }

        // a question mark in the first character is invalid
        if chunk.starts_with('?') {
            break;
        }

        if chunk.contains('?') {
            let no_qmark = chunk.replace('?', "");

            if no_qmark.len() * 2 == chunk.len() {
                // each character has a question mark, so we
                // will end up watching an empty chunk, which
                // is just the parent directory
                break;
            }

            // we need the powerset of all the paths
            // with and without the optional characters
            return chunk
                .match_indices('?')
                .map(|(i, _)| i)
                .enumerate()
                .map(|(i, j)| j - i - 1) // subtract 1 for each index to account for the removal of the '?'
                .powerset() // get all the possible combinations of ignored and not
                .map(|indices| {
                    let mut new_chunk = no_qmark.clone();
                    // reverse the indices so we can remove them without
                    // having to recalculate the offsets
                    for i in indices.iter().rev() {
                        new_chunk.remove(*i);
                    }
                    new_chunk
                })
                .map(|chunk| {
                    chunks
                        .iter()
                        .chain(std::iter::once(&chunk.as_str()))
                        .collect()
                })
                .collect();
        }

        chunks.push(chunk);
    }

    vec![chunks.iter().collect()]
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use test_case::test_case;

    #[test_case("foo/**", vec!["foo"])]
    #[test_case("foo/{a,b}", vec!["foo/a", "foo/b"])]
    #[test_case("foo/*/bar", vec!["foo"])]
    #[test_case("foo/[a-d]/bar", vec!["foo"])]
    #[test_case("foo/a?/bar", vec!["foo"])]
    #[test_case("foo/ab?/bar", vec!["foo/a", "foo/ab"])]
    #[test_case("foo/ab?c?", vec!["foo/a", "foo/ab", "foo/abc", "foo/ac"])]
    // todo: this should be ["foo/a/a", "foo/a/ab", "foo/b/a", "foo/b/ab"]
    #[test_case("foo/{a,b}/ab?", vec!["foo/a", "foo/b"])]
    fn test_handles_doublestar(glob: &str, paths_exp: Vec<&str>) {
        let mut paths = super::glob_to_paths(glob);
        paths.sort();
        assert_eq!(
            paths,
            paths_exp.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }
}
