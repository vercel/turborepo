use futures::{Stream, stream};

use crate::signals::Signal;

#[derive(Debug, thiserror::Error)]
#[error("Failed to register signal handler: {0}")]
pub struct Error(#[from] std::io::Error);

#[cfg(windows)]
/// A listener for Windows Console Ctrl-C events
pub fn get_signal() -> Result<impl Stream<Item = Option<Signal>>, Error> {
    use tokio::io::AsyncReadExt;

    let debug_ctrl_c = std::env::var("TURBO_DEBUG_WINDOWS_CTRL_C").as_deref() == Ok("1");
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    Ok(stream::once(async move {
        let wrapper_ctrl_c_port = std::env::var("TURBO_WINDOWS_CTRL_C_PORT")
            .ok()
            .and_then(|port| port.parse::<u16>().ok());

        if let Some(port) = wrapper_ctrl_c_port {
            if debug_ctrl_c {
                eprintln!("[turbo rust ctrl-c] connecting to wrapper on port {port}");
            }
            let wrapper_ctrl_c = async move {
                loop {
                    if let Ok(mut stream) =
                        tokio::net::TcpStream::connect(("127.0.0.1", port)).await
                    {
                        if debug_ctrl_c {
                            eprintln!("[turbo rust ctrl-c] connected to wrapper");
                        }
                        let mut byte = [0];
                        if stream.read_exact(&mut byte).await.is_ok() && byte[0] == 0x03 {
                            if debug_ctrl_c {
                                eprintln!("[turbo rust ctrl-c] received wrapper ctrl-c");
                            }
                            return Some(Signal::CtrlC);
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            };

            tokio::select! {
                signal = ctrl_c.recv() => {
                    if debug_ctrl_c {
                        eprintln!("[turbo rust ctrl-c] received console ctrl-c");
                    }
                    signal.map(|_| Signal::CtrlC)
                },
                signal = wrapper_ctrl_c => signal,
            }
        } else {
            if debug_ctrl_c {
                eprintln!("[turbo rust ctrl-c] waiting for console ctrl-c");
            }
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
