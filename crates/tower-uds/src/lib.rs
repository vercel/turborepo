//! tower-uds
//!
//! A unix domain socket server for the `tower` ecosystem.

#![feature(impl_trait_in_assoc_type)]

use std::{future::Future, path::Path};

use tower::Service;

pub struct UDSConnector<'a> {
    path: &'a Path,
}

impl<'a> UDSConnector<'a> {
    pub fn new(path: &'a Path) -> Self {
        Self { path }
    }
}

impl<'a, T> Service<T> for UDSConnector<'a> {
    #[cfg(not(target_os = "windows"))]
    type Response = tokio::net::UnixStream;

    // tokio does not support UDS on windows, so we need to use async-io
    // with a tokio compat layer instead
    #[cfg(target_os = "windows")]
    type Response = tokio_util::compat::Compat<async_io::Async<uds_windows::UnixStream>>;

    type Error = std::io::Error;

    type Future = impl Future<Output = Result<Self::Response, Self::Error>> + 'a;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Ok(()).into()
    }

    fn call(&mut self, _req: T) -> Self::Future {
        // we need to make sure our ref is immutable so that the closure has a lifetime
        // of 'a, not the anonymous lifetime of the call method
        let path = self.path;

        #[cfg(not(target_os = "windows"))]
        {
            async move { tokio::net::UnixStream::connect(path).await }
        }
        #[cfg(target_os = "windows")]
        {
            async move {
                use tokio_util::compat::FuturesAsyncReadCompatExt;
                let stream = uds_windows::UnixStream::connect(path)?;
                Ok(FuturesAsyncReadCompatExt::compat(async_io::Async::new(
                    stream,
                )?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        task::{Context, Poll},
    };

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    #[cfg(not(target_os = "windows"))]
    async fn test_successful_connection() {
        // Create a temporary socket path
        let socket_path = std::env::temp_dir().join("test_uds_success.sock");

        // Clean up any existing socket
        let _ = std::fs::remove_file(&socket_path);

        // Create a UDS server
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("Failed to bind socket");

        // Spawn server task
        let server_handle = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buffer).await {
                    // Echo back the data
                    let _ = stream.write_all(&buffer[..n]).await;
                }
            }
        });

        // Create connector and test connection
        let mut connector = UDSConnector::new(&socket_path);
        let mut stream = connector.call(()).await.expect("Failed to connect");

        // Test that we can actually communicate through the connection
        let test_data = b"Hello, UDS!";
        stream.write_all(test_data).await.expect("Failed to write");

        let mut buffer = [0u8; 1024];
        let n = stream.read(&mut buffer).await.expect("Failed to read");
        assert_eq!(&buffer[..n], test_data);

        // Clean up
        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    #[cfg(target_os = "windows")]
    async fn test_successful_connection_windows() {
        // Windows UDS implementation test
        use tokio_util::compat::TokioAsyncReadCompatExt;

        // Create a temporary socket path
        let socket_path = std::env::temp_dir().join("test_uds_success.sock");

        // Clean up any existing socket
        let _ = std::fs::remove_file(&socket_path);

        // Create a UDS server using the Windows implementation
        let listener =
            uds_windows::UnixListener::bind(&socket_path).expect("Failed to bind socket");
        let async_listener =
            async_io::Async::new(listener).expect("Failed to create async listener");

        // Spawn server task
        let server_handle = tokio::spawn(async move {
            if let Ok((stream, _)) = async_listener.accept().await {
                let mut stream = stream.compat();
                let mut buffer = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buffer).await {
                    // Echo back the data
                    let _ = stream.write_all(&buffer[..n]).await;
                }
            }
        });

        // Create connector and test connection
        let mut connector = UDSConnector::new(&socket_path);
        let mut stream = connector.call(()).await.expect("Failed to connect");

        // Test that we can actually communicate through the connection
        let test_data = b"Hello, UDS!";
        stream.write_all(test_data).await.expect("Failed to write");

        let mut buffer = [0u8; 1024];
        let n = stream.read(&mut buffer).await.expect("Failed to read");
        assert_eq!(&buffer[..n], test_data);

        // Clean up
        server_handle.abort();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_connection_failure() {
        // Try to connect to a non-existent socket
        let socket_path = std::env::temp_dir().join("nonexistent_socket.sock");

        // Make sure the socket doesn't exist
        let _ = std::fs::remove_file(&socket_path);

        let mut connector = UDSConnector::new(&socket_path);
        let result = connector.call(()).await;

        // Should fail with an IO error
        assert!(result.is_err());
        let error = result.unwrap_err();

        // On Unix, this should be NotFound, but on Windows it might be different
        #[cfg(not(target_os = "windows"))]
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);

        #[cfg(target_os = "windows")]
        {
            // On Windows, the error might be different, but should still be a connection
            // error
            let is_connection_error = matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            );
            assert!(
                is_connection_error,
                "Expected connection error, got: {:?}",
                error
            );
        }
    }

    #[tokio::test]
    async fn test_poll_ready_always_ready() {
        let socket_path = Path::new("/tmp/test.sock");
        let mut connector = UDSConnector::new(socket_path);

        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(&waker);

        // UDS connector should always be ready since it doesn't maintain persistent
        // connections
        let result = <UDSConnector as Service<()>>::poll_ready(&mut connector, &mut cx);
        assert!(matches!(result, Poll::Ready(Ok(()))));
    }
}
