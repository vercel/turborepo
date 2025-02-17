use std::future::Future;

use crate::signals::Signal;

#[cfg(windows)]
/// A listener for Windows Console Ctrl-C events
pub fn get_signal() -> Result<impl Future<Output = Option<Signal>>, std::io::Error> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c().map_err(run::Error::SignalHandler)?;
    Ok(async move { ctrl_c.recv().await.map(|_| Signal::CtrlC) })
}

#[cfg(not(windows))]
/// A listener for commong Unix signals that require special handling
///
/// Currently listens for SIGINT and SIGTERM
pub fn get_signal() -> Result<impl Future<Output = Option<Signal>>, std::io::Error> {
    use tokio::signal::unix;
    let mut sigint = unix::signal(unix::SignalKind::interrupt())?;
    let mut sigterm = unix::signal(unix::SignalKind::terminate())?;

    Ok(async move {
        tokio::select! {
            res = sigint.recv() => {
                res.map(|_| Signal::Interrupt)
            }
            res = sigterm.recv() => {
                res.map(|_| Signal::Terminate)
            }
        }
    })
}
