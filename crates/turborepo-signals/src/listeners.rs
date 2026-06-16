use futures::{Stream, stream};

use crate::signals::Signal;

#[derive(Debug, thiserror::Error)]
#[error("Failed to register signal handler: {0}")]
pub struct Error(#[from] std::io::Error);

#[cfg(windows)]
/// A listener for Windows Console Ctrl-C events
pub fn get_signal() -> Result<impl Stream<Item = Option<Signal>>, Error> {
    use tokio::io::AsyncReadExt;

    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    Ok(stream::once(async move {
        let wrapper_ctrl_c_port = std::env::var("__TURBO_WINDOWS_CTRL_C_PORT")
            .ok()
            .and_then(|port| port.parse::<u16>().ok());

        if let Some(port) = wrapper_ctrl_c_port {
            let wrapper_ctrl_c = async move {
                loop {
                    if let Ok(mut stream) =
                        tokio::net::TcpStream::connect(("127.0.0.1", port)).await
                    {
                        let mut byte = [0];
                        if stream.read_exact(&mut byte).await.is_ok() && byte[0] == 0x03 {
                            return Some(Signal::CtrlC);
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            };

            tokio::select! {
                signal = ctrl_c.recv() => signal.map(|_| Signal::CtrlC),
                signal = wrapper_ctrl_c => signal,
            }
        } else {
            ctrl_c.recv().await.map(|_| Signal::CtrlC)
        }
    }))
}

#[cfg(not(windows))]
/// A listener for commong Unix signals that require special handling
///
/// Currently listens for SIGINT and SIGTERM
pub fn get_signal() -> Result<impl Stream<Item = Option<Signal>>, Error> {
    use tokio::signal::unix;
    let mut sigint = unix::signal(unix::SignalKind::interrupt())?;
    let mut sigterm = unix::signal(unix::SignalKind::terminate())?;
    let mut sighup = unix::signal(unix::SignalKind::hangup())?;

    Ok(stream::once(async move {
        tokio::select! {
            res = sigint.recv() => {
                res.map(|_| Signal::Interrupt)
            }
            res = sighup.recv() => {
                res.map(|_| Signal::Interrupt)
            }
            res = sigterm.recv() => {
                res.map(|_| Signal::Terminate)
            }
        }
    }))
}
