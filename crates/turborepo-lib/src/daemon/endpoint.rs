use std::sync::{atomic::AtomicBool, Arc};
#[cfg(windows)]
use std::{io::ErrorKind, sync::atomic::Ordering, time::Duration};

use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tonic::transport::server::Connected;
use tracing::{debug, trace};
use turbopath::AbsoluteSystemPathBuf;

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
    path: AbsoluteSystemPathBuf,
    #[allow(unused)] running: Arc<AtomicBool>,
) -> Result<
    (
        pidlock::Pidlock,
        impl Stream<Item = Result<impl Connected + AsyncWrite + AsyncRead, std::io::Error>>,
    ),
    SocketOpenError,
> {
    let pid_path = path.join_component("turbod.pid");
    let sock_path = path.join_component("turbod.sock");
    let mut lock = pidlock::Pidlock::new(pid_path.as_path().to_owned());

    trace!("acquiring pidlock");
    // this will fail if the pid is already owned
    lock.acquire()?;
    std::fs::remove_file(&sock_path).ok();

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
        path::Path,
        process::Command,
        sync::{atomic::AtomicBool, Arc},
    };

    use pidlock::PidlockError;
    use turbopath::AbsoluteSystemPathBuf;

    use super::listen_socket;
    use crate::daemon::endpoint::SocketOpenError;

    fn pid_path(tmp_path: &Path) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(tmp_path.join("turbod.pid")).unwrap()
    }

    #[tokio::test]
    async fn test_stale_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();
        let pid_path = pid_path(&tmp_path);
        // A pid that will never be running and is guaranteed not to be us
        pid_path.create_with_contents("100000").unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(pid_path, running).await;

        // Note: PidLock doesn't implement Debug, so we can't unwrap_err()
        if let Err(err) = result {
            assert_matches!(err, SocketOpenError::LockError(PidlockError::LockExists(_)));
        } else {
            panic!("expected an error")
        }
    }

    #[tokio::test]
    async fn test_existing_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_owned();
        let pid_path = pid_path(&tmp_path);

        #[cfg(windows)]
        let node_bin = "node.exe";
        #[cfg(not(windows))]
        let node_bin = "node";

        let mut child = Command::new(node_bin).spawn().unwrap();
        pid_path
            .create_with_contents(format!("{}", child.id()).as_ref())
            .unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(pid_path, running).await;

        // Note: PidLock doesn't implement Debug, so we can't unwrap_err()
        if let Err(err) = result {
            assert_matches!(err, SocketOpenError::LockError(PidlockError::LockExists(_)));
        } else {
            panic!("expected an error")
        }

        child.kill().unwrap();
    }
}
