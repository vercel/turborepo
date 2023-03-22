use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tonic::transport::server::Connected;

///
pub async fn get_channel(
    path: PathBuf,
) -> Result<tonic::transport::Channel, tonic::transport::Error> {
    let arc = Arc::new(path);
    tonic::transport::Endpoint::try_from("http://[::]:50051")?
        .connect_with_connector(tower::service_fn(move |_| {
            // we clone the reference counter here and move it into the async closure
            let arc = arc.clone();
            #[cfg(unix)]
            {
                async move { tokio::net::UnixStream::connect::<&Path>(arc.as_path()).await }
            }

            #[cfg(windows)]
            {
                async move { uds_windows::UnixStream::connect(arc.as_path()) }
            }
        }))
        .await
}

#[derive(thiserror::Error, Debug)]
pub enum SocketOpenError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(windows)]
    #[error("windows socket error: {0}")]
    UdsWindows(#[from] uds_windows::Error),
    #[error("pidlock error")]
    LockError(#[from] pidlock::PidlockError),
}

/// Gets a stream of incoming connections from a Unix socket.
/// On windows, this will use the `uds_windows` crate, and
/// poll the result in another thread.
pub async fn open_socket(
    path: turborepo_paths::AbsoluteNormalizedPathBuf,
) -> Result<
    (
        pidlock::Pidlock,
        impl Stream<Item = Result<impl Connected + AsyncWrite + AsyncRead, std::io::Error>>,
    ),
    SocketOpenError,
> {
    let pid_path = path.join(turborepo_paths::ForwardRelativePath::new("turbod.pid").unwrap());
    let sock_path = path.join(turborepo_paths::ForwardRelativePath::new("turbod.sock").unwrap());
    let mut lock = pidlock::Pidlock::new(pid_path.to_path_buf());

    // this will fail if the pid is already owned
    lock.acquire()?;
    std::fs::remove_file(&sock_path).ok();

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
        let listener = uds_windows::UnixListener::bind(path);
        futures::stream::unfold(listener, |listener| async move {
            match listener.accept().await {
                Ok((stream, _)) => Some((Ok(stream), listener)),
                Err(err) => Some((Err(err), listener)),
            }
        })
    }
}

/// An adaptor over uds_windows that implements AsyncRead and AsyncWrite.
#[cfg(windows)]
struct UdsWindowsStream(uds_windows::UnixStream);

#[cfg(windows)]
impl AsyncRead for UdsWindowsStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.poll_read(cx, buf)
    }
}

#[cfg(windows)]
impl AsyncWrite for UdsWindowsStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.0.poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.0.poll_shutdown(cx)
    }
}
