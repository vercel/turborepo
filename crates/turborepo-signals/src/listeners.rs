use futures::{Stream, stream};

use crate::signals::Signal;

#[derive(Debug, thiserror::Error)]
#[error("Failed to register signal handler: {0}")]
pub struct Error(#[from] std::io::Error);

#[cfg(windows)]
/// A listener for Windows Console Ctrl-C events
pub fn get_signal() -> Result<impl Stream<Item = Option<Signal>>, Error> {
    let wrapper_ctrl_c = wrapper_ctrl_c_fd_from_env(std::env::var_os("__TURBO_WINDOWS_CTRL_C_FD"))
        .map(wrapper_ctrl_c_receiver)
        .transpose()?;
    let ctrl_c = if wrapper_ctrl_c.is_none() {
        Some(tokio::signal::windows::ctrl_c()?)
    } else {
        None
    };

    Ok(stream::once(async move {
        if let Some(wrapper_ctrl_c) = wrapper_ctrl_c {
            wrapper_ctrl_c.await.ok().flatten()
        } else if let Some(mut ctrl_c) = ctrl_c {
            ctrl_c.recv().await.map(|_| Signal::CtrlC)
        } else {
            None
        }
    }))
}

#[cfg(windows)]
fn wrapper_ctrl_c_fd_from_env(value: Option<std::ffi::OsString>) -> Option<i32> {
    value.and_then(|fd| fd.to_str()?.parse::<i32>().ok())
}

#[cfg(windows)]
fn wrapper_ctrl_c_receiver(
    fd: i32,
) -> Result<tokio::sync::oneshot::Receiver<Option<Signal>>, std::io::Error> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("turbo-windows-ctrl-c-pipe".to_string())
        .spawn(move || {
            tx.send(read_wrapper_ctrl_c(fd)).ok();
        })?;
    Ok(rx)
}

#[cfg(windows)]
fn read_wrapper_ctrl_c(fd: i32) -> Option<Signal> {
    let mut byte = [0];
    loop {
        match unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) } {
            1 if byte[0] == 0x03 => return Some(Signal::CtrlC),
            1 => {}
            _ => return None,
        }
    }
}

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::wrapper_ctrl_c_fd_from_env;

    #[test]
    fn parses_wrapper_ctrl_c_fd() {
        assert_eq!(wrapper_ctrl_c_fd_from_env(Some("3".into())), Some(3));
        assert_eq!(wrapper_ctrl_c_fd_from_env(Some("not-a-fd".into())), None);
        assert_eq!(wrapper_ctrl_c_fd_from_env(None), None);
    }
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
