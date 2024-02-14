use std::sync::{atomic::AtomicBool, Arc};
#[cfg(windows)]
use std::{io::ErrorKind, sync::atomic::Ordering, time::Duration};

use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tonic::transport::server::Connected;
use tracing::{debug, trace};
use turbopath::AbsoluteSystemPath;

#[derive(thiserror::Error, Debug)]
pub enum SocketOpenError {
    /// Returned when there is an IO error opening the socket,
    /// such as the path being too long, or the path being
    /// invalid.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("pidlock error: {0}")]
    LockError(#[from] pidlock::PidlockError),
}

#[cfg(windows)]
const WINDOWS_POLL_DURATION: Duration = Duration::from_millis(1);

/// Gets a stream of incoming connections from a Unix socket.
/// On windows, this will use the `uds_windows` crate, and
/// poll the result in another thread.
///
/// note: the running param is used by the windows
///       code path to shut down the non-blocking polling
#[tracing::instrument]
pub async fn listen_socket(
    pid_path: &AbsoluteSystemPath,
    sock_path: &AbsoluteSystemPath,
    #[allow(unused)] running: Arc<AtomicBool>,
) -> Result<
    (
        pidlock::Pidlock,
        impl Stream<Item = Result<impl Connected + AsyncWrite + AsyncRead, std::io::Error>>,
    ),
    SocketOpenError,
> {
    let mut lock = pidlock::Pidlock::new(pid_path.as_std_path().to_owned());

    trace!("acquiring pidlock");
    // this will fail if the pid is already owned
    // todo: make sure we fall back and handle this
    lock.acquire()?;
    sock_path.remove_file().ok();

    debug!("pidlock acquired at {}", pid_path);
    debug!("listening on socket at {}", sock_path);

    #[cfg(unix)]
    {
        Ok((
            lock,
            tokio_stream::wrappers::UnixListenerStream::new(tokio::net::UnixListener::bind(
                sock_path,
            )?),
        ))
    }

    #[cfg(windows)]
    {
        use tokio_util::compat::FuturesAsyncReadCompatExt;

        let listener = Arc::new(uds_windows::UnixListener::bind(sock_path)?);
        listener.set_nonblocking(true)?;

        let stream = futures::stream::unfold(listener, move |listener| {
            let task_running = running.clone();
            async move {
                // ensure the underlying thread is aborted on drop
                let task_listener = listener.clone();
                let task = tokio::task::spawn_blocking(move || loop {
                    break match task_listener.accept() {
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            std::thread::sleep(WINDOWS_POLL_DURATION);
                            if !task_running.load(Ordering::SeqCst) {
                                None
                            } else {
                                continue;
                            }
                        }
                        res => Some(res),
                    };
                });

                let result = task
                    .await
                    .expect("no panic")?
                    .map(|(stream, _)| stream)
                    .and_then(async_io::Async::new)
                    .map(FuturesAsyncReadCompatExt::compat)
                    .map(UdsWindowsStream);

                Some((result, listener))
            }
        });

        Ok((lock, stream))
    }
}

/// An adaptor over uds_windows that implements AsyncRead and AsyncWrite.
///
/// It utilizes structural pinning to forward async read and write
/// implementations onto the inner type.
#[cfg(windows)]
struct UdsWindowsStream<T>(T);

#[cfg(windows)]
impl<T> UdsWindowsStream<T> {
    /// Project the (pinned) uds windows stream to get the inner (pinned) type
    ///
    /// SAFETY
    ///
    /// structural pinning requires a few invariants to hold which can be seen
    /// here https://doc.rust-lang.org/std/pin/#pinning-is-structural-for-field
    ///
    /// in short:
    /// - we cannot implement Unpin for UdsWindowsStream
    /// - we cannot use repr packed
    /// - we cannot move in the drop impl (the default impl doesn't)
    /// - we must uphold the rust 'drop guarantee'
    /// - we cannot offer any api to move data out of the pinned value (such as
    ///   Option::take)
    fn project(self: std::pin::Pin<&mut Self>) -> std::pin::Pin<&mut T> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

#[cfg(windows)]
impl<T: AsyncRead> AsyncRead for UdsWindowsStream<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().poll_read(cx, buf)
    }
}

#[cfg(windows)]
impl<T: AsyncWrite> AsyncWrite for UdsWindowsStream<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        self.project().poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_shutdown(cx)
    }
}

#[cfg(windows)]
impl<T> Connected for UdsWindowsStream<T> {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches::assert_matches,
        process::Command,
        sync::{atomic::AtomicBool, Arc},
    };

    use pidlock::PidlockError;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::listen_socket;
    use crate::daemon::{endpoint::SocketOpenError, Paths};

    fn pid_path(daemon_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        daemon_root.join_component("turbod.pid")
    }

    #[tokio::test]
    async fn test_stale_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);
        paths.pid_file.ensure_dir().unwrap();
        // A pid that will never be running and is guaranteed not to be us
        paths.pid_file.create_with_contents("100000").unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(&paths.pid_file, &paths.sock_file, running).await;

        assert!(
            result.is_ok(),
            "expected to clear stale pid file and connect"
        );
    }

    #[tokio::test]
    async fn test_existing_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);

        #[cfg(windows)]
        let node_bin = "node.exe";
        #[cfg(not(windows))]
        let node_bin = "node";

        let mut child = Command::new(node_bin).spawn().unwrap();
        paths.pid_file.ensure_dir().unwrap();
        paths
            .pid_file
            .create_with_contents(format!("{}", child.id()))
            .unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(&paths.pid_file, &paths.sock_file, running).await;

        // Note: PidLock doesn't implement Debug, so we can't unwrap_err()

        // todo: update this test. we should delete the socket file first, remove the
        // pid file, and start a new daemon. the old one should just time
        // out, and this should not error.
        if let Err(err) = result {
            assert_matches!(err, SocketOpenError::LockError(PidlockError::AlreadyOwned));
        } else {
            panic!("expected an error")
        }

        child.kill().unwrap();
    }
}
