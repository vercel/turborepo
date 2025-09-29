use futures::{Stream, stream};

use crate::signals::Signal;

#[derive(Debug, thiserror::Error)]
#[error("Failed to register signal handler: {0}")]
pub struct Error(#[from] std::io::Error);

#[cfg(windows)]
/// A listener for Windows Console Ctrl-C events
pub fn get_signal() -> Result<impl Stream<Item = Option<Signal>>, Error> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    Ok(stream::once(async move {
        ctrl_c.recv().await.map(|_| Signal::CtrlC)
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
