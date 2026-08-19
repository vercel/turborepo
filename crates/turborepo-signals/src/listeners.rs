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

    Ok(stream::unfold(
        (wrapper_ctrl_c, ctrl_c),
        |(mut wrapper_ctrl_c, mut ctrl_c)| async move {
            let signal = if let Some(receiver) = wrapper_ctrl_c.as_mut() {
                receiver.recv().await.flatten()
            } else if let Some(ctrl_c) = ctrl_c.as_mut() {
                ctrl_c.recv().await.map(|_| Signal::CtrlC)
            } else {
                None
            };
            Some((signal, (wrapper_ctrl_c, ctrl_c)))
        },
    ))
}

#[cfg(windows)]
fn wrapper_ctrl_c_fd_from_env(value: Option<std::ffi::OsString>) -> Option<i32> {
    value.and_then(|fd| fd.to_str()?.parse::<i32>().ok())
}

#[cfg(windows)]
fn wrapper_ctrl_c_receiver(
    fd: i32,
) -> Result<tokio::sync::mpsc::UnboundedReceiver<Option<Signal>>, std::io::Error> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("turbo-windows-ctrl-c-pipe".to_string())
        .spawn(move || {
            let mut byte = [0];
            loop {
                match unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) } {
                    1 if byte[0] == 0x03 => {
                        if tx.send(Some(Signal::CtrlC)).is_err() {
                            return;
                        }
                    }
                    1 => {}
                    _ => {
                        tx.send(None).ok();
                        return;
                    }
                }
            }
        })?;
    Ok(rx)
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
    let sigint = unix::signal(unix::SignalKind::interrupt())?;
    let sigterm = unix::signal(unix::SignalKind::terminate())?;
    let sighup = unix::signal(unix::SignalKind::hangup())?;

    Ok(stream::unfold(
        (sigint, sigterm, sighup),
        |(mut sigint, mut sigterm, mut sighup)| async move {
            let signal = tokio::select! {
                res = sigint.recv() => {
                    res.map(|_| Signal::Interrupt)
                }
                res = sighup.recv() => {
                    res.map(|_| Signal::Interrupt)
                }
                res = sigterm.recv() => {
                    res.map(|_| Signal::Terminate)
                }
            };
            Some((signal, (sigint, sigterm, sighup)))
        },
    ))
}
