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

#[derive(PartialEq, Eq, Debug)]
enum GlobSymbol<'a> {
    Char(&'a [u8]),
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Star,
    DoubleStar,
    Question,
    Negation,
    PathSeperator,
}

/// Gets the minimum set of paths that can be watched for a given glob,
/// specified in minimatch glob syntax.
///
/// syntax:
/// ?		Matches any single character.
/// *		Matches zero or more characters, except for path separators
/// **		Matches zero or more characters, including path separators.
///         Must match a complete path segment.
/// [ab]	Matches one of the characters contained in the brackets.
///         Character ranges, e.g. [a-z] are also supported. Use [!ab] or [^ab]
///         to match any character except those contained in the brackets.
/// {a,b}	Matches one of the patterns contained in the braces. Any of the
///         wildcard characters can be used in the sub-patterns. Braces may
///         be nested up to 10 levels deep.
/// !		When at the start of the glob, this negates the result.
///         Multiple ! characters negate the glob multiple times.
/// \		A backslash character may be used to escape any special characters.
///
/// Of these, we only handle `{` and escaping.
///
/// note: it is currently extremely conservative, handling only `**`, braces,
/// and `?`. any other case watches the entire directory.
fn glob_to_paths(glob: &str) -> Vec<PathBuf> {
    // get all the symbols and chunk them by path seperator
    let chunks = glob_to_symbols(glob).group_by(|s| s != &GlobSymbol::PathSeperator);
    let chunks = chunks
        .into_iter()
        .filter_map(|(not_sep, chunk)| (not_sep).then(|| chunk));

    // multi cartisian product allows us to get all the possible combinations
    // of path components for each chunk. for example, if we have a glob
    // `{a,b}/1/{c,d}`, it will lazily yield the following sets of segments:
    //   ["a", "1", "c"]
    //   ["a", "1", "d"]
    //   ["b", "1", "c"]
    //   ["b", "1", "d"]

    chunks
        .map(symbols_to_combinations) // yield all the possible segments for each glob chunk
        .take_while(|c| c.is_some()) // if any segment has no possible paths, we can stop
        .filter_map(|chunk| chunk)
        .multi_cartesian_product() // get all the possible combinations of path segments
        .map(|chunks| chunks.into_iter().collect::<PathBuf>())
        .collect()
}

/// given a set of symbols, returns an iterator over the possible path segments
/// that can be generated from them
///
/// example: given the symbols "{a,b}b" it will yield ["ab"] and ["bb"]
fn symbols_to_combinations<'a, T: Iterator<Item = GlobSymbol<'a>>>(
    symbols: T,
) -> Option<impl Iterator<Item = String> + Clone> {
    let mut bytes = Vec::new();

    for symbol in symbols {
        match symbol {
            GlobSymbol::Char(c) => {
                bytes.extend_from_slice(c);
            }
            GlobSymbol::OpenBracket => return None, // todo handle brackets
            GlobSymbol::CloseBracket => return None,
            GlobSymbol::OpenBrace => return None, // todo handle braces
            GlobSymbol::CloseBrace => return None,
            GlobSymbol::Star => return None,
            GlobSymbol::DoubleStar => return None,
            GlobSymbol::Question => return None,
            GlobSymbol::Negation => return None,
            GlobSymbol::PathSeperator => return None,
        }
    }

    Some(std::iter::once(
        String::from_utf8(bytes).expect("char is always valid utf8"),
    ))
}

/// parses and escapes a glob, returning an iterator over the symbols
fn glob_to_symbols(glob: &str) -> impl Iterator<Item = GlobSymbol> {
    let glob_bytes = glob.as_bytes();
    let mut escaped = false;
    let mut cursor = unic_segment::GraphemeCursor::new(0, glob.len());

    std::iter::from_fn(move || loop {
        let start = cursor.cur_cursor();
        if start == glob.len() {
            return None;
        }
        let end = cursor.next_boundary(glob, 0).unwrap().unwrap();

        if escaped {
            escaped = false;
            return if end - start == 1 {
                Some(GlobSymbol::Char(match glob_bytes[start] {
                    b'a' => &[b'\x61'],
                    b'b' => &[b'\x08'],
                    b'n' => &[b'\n'],
                    b'r' => &[b'\r'],
                    b't' => &[b'\t'],
                    _ => &glob_bytes[start..end],
                }))
            } else {
                return Some(GlobSymbol::Char(&glob_bytes[start..end]));
            };
        }

        return if end - start == 1 {
            match glob_bytes[start] {
                b'\\' => {
                    escaped = true;
                    continue;
                }
                b'[' => Some(GlobSymbol::OpenBracket),
                b']' => Some(GlobSymbol::CloseBracket),
                b'{' => Some(GlobSymbol::OpenBrace),
                b'}' => Some(GlobSymbol::CloseBrace),
                b'*' => {
                    if glob_bytes.get(end) == Some(&b'*') {
                        cursor.set_cursor(end + 1);
                        Some(GlobSymbol::DoubleStar)
                    } else {
                        Some(GlobSymbol::Star)
                    }
                }
                b'?' => Some(GlobSymbol::Question),
                b'!' => Some(GlobSymbol::Negation),
                b'/' => Some(GlobSymbol::PathSeperator),
                _ => Some(GlobSymbol::Char(&glob_bytes[start..end])),
            }
        } else {
            Some(GlobSymbol::Char(&glob_bytes[start..end]))
        };
    })
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use test_case::test_case;

    use super::GlobSymbol::*;

    #[test_case("foo/**", vec!["foo"])]
    #[test_case("foo/{a,b}", vec!["foo/a", "foo/b"])]
    #[test_case("foo/*/bar", vec!["foo"])]
    #[test_case("foo/[a-d]/bar", vec!["foo"])]
    #[test_case("foo/a?/bar", vec!["foo"])]
    #[test_case("foo/ab?/bar", vec!["foo"] ; "question marks ")]
    #[test_case("foo/{a,b}/ab?", vec!["foo/a", "foo/b"])]
    fn test_handles_doublestar(glob: &str, paths_exp: Vec<&str>) {
        let mut paths = super::glob_to_paths(glob);
        paths.sort();
        assert_eq!(
            paths,
            paths_exp.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }

    #[test_case("🇳🇴/🇳🇴", vec![Char("🇳🇴".as_bytes()), PathSeperator, Char("🇳🇴".as_bytes())])]
    #[test_case("foo/**", vec![Char(b"f"), Char(b"o"), Char(b"o"), PathSeperator, DoubleStar])]
    #[test_case("foo/{a,b}", vec![Char(b"f"), Char(b"o"), Char(b"o"), PathSeperator, OpenBrace, Char(b"a"), Char(b","), Char(b"b"), CloseBrace])]
    #[test_case("\\f", vec![Char(b"f")])]
    #[test_case("\\\\f", vec![Char(b"\\"), Char(b"f")])]
    #[test_case("\\🇳🇴", vec![Char("🇳🇴".as_bytes())])]
    #[test_case("\\n", vec![Char(b"\n")])]
    fn test_glob_to_symbols(glob: &str, symbols_exp: Vec<super::GlobSymbol>) {
        let symbols = super::glob_to_symbols(glob).collect::<Vec<_>>();
        assert_eq!(symbols.as_slice(), symbols_exp.as_slice());
    }
}
