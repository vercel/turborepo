// Inspired by https://github.com/pvolok/mprocs/blob/master/src/clipboard.rs
use std::process::Stdio;

use base64::Engine;
use which::which;

pub fn copy_to_clipboard(s: &str) {
    match copy_impl(s, &PROVIDER) {
        Ok(()) => (),
        Err(err) => tracing::debug!("Unable to copy: {}", err.to_string()),
    }
}

#[allow(dead_code)]
enum Provider {
    OSC52,
    Exec(&'static str, Vec<&'static str>),
    #[cfg(windows)]
    Win,
    NoOp,
}

#[cfg(windows)]
fn detect_copy_provider() -> Provider {
    Provider::Win
}

#[cfg(target_os = "macos")]
fn detect_copy_provider() -> Provider {
    detect_macos_copy_provider(
        has_value(std::env::var_os("SSH_TTY").as_deref())
            || has_value(std::env::var_os("SSH_CONNECTION").as_deref()),
        has_value(std::env::var_os("TMUX").as_deref()),
        check_prog,
    )
}

#[cfg(any(target_os = "macos", test))]
fn has_value(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

#[cfg(any(target_os = "macos", test))]
fn detect_macos_copy_provider(
    is_ssh_session: bool,
    is_tmux_session: bool,
    mut find_provider: impl FnMut(&'static str, &[&'static str]) -> Option<Provider>,
) -> Provider {
    if is_ssh_session {
        if is_tmux_session
            && let Some(provider) = find_provider("tmux", &["load-buffer", "-w", "-"])
        {
            return provider;
        }
        return Provider::OSC52;
    }

    find_provider("pbcopy", &[]).unwrap_or(Provider::OSC52)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_copy_provider() -> Provider {
    // Wayland
    if std::env::var("WAYLAND_DISPLAY").is_ok()
        && let Some(provider) = check_prog("wl-copy", &["--type", "text/plain"])
    {
        return provider;
    }
    // X11
    if std::env::var("DISPLAY").is_ok() {
        if let Some(provider) = check_prog("xclip", &["-i", "-selection", "clipboard"]) {
            return provider;
        }
        if let Some(provider) = check_prog("xsel", &["-i", "-b"]) {
            return provider;
        }
    }
    // Termux
    if let Some(provider) = check_prog("termux-clipboard-set", &[]) {
        return provider;
    }
    // Tmux
    if std::env::var("TMUX").is_ok()
        && let Some(provider) = check_prog("tmux", &["load-buffer", "-"])
    {
        return provider;
    }

    Provider::OSC52
}

#[allow(dead_code)]
fn check_prog(cmd: &'static str, args: &[&'static str]) -> Option<Provider> {
    if which(cmd).is_ok() {
        Some(Provider::Exec(cmd, args.to_vec()))
    } else {
        None
    }
}

fn copy_impl(s: &str, provider: &Provider) -> std::io::Result<()> {
    match provider {
        Provider::OSC52 => {
            let mut stdout = std::io::stdout().lock();
            use std::io::Write;
            write!(
                &mut stdout,
                "\x1b]52;;{}\x07",
                base64::engine::general_purpose::STANDARD.encode(s)
            )?;
        }

        Provider::Exec(prog, args) => {
            let mut child = std::process::Command::new(prog)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            // Do not exit early if we fail to write to the clipboard, make sure we attempt
            // to wait on the clipboard to exit to avoid a zombie process.
            let write_result = match child.stdin.as_mut() {
                Some(stdin) => std::io::Write::write_all(stdin, s.as_bytes()),
                None => Err(std::io::Error::other(
                    "clipboard provider stdin was unavailable",
                )),
            };
            let wait_result = child.wait();
            write_result?;
            wait_result?;
        }

        #[cfg(windows)]
        Provider::Win => clipboard_win::set_clipboard_string(s)
            .map_err(|e| std::io::Error::other(e.to_string()))?,

        Provider::NoOp => (),
    };

    Ok(())
}

static PROVIDER: std::sync::LazyLock<Provider> = std::sync::LazyLock::new(detect_copy_provider);

#[cfg(test)]
mod tests {
    use super::{Provider, copy_impl};

    #[test]
    fn macos_ssh_session_uses_tmux_clipboard_when_available() {
        let mut probes = Vec::new();
        let provider = super::detect_macos_copy_provider(true, true, |cmd, args| {
            probes.push((cmd, args.to_vec()));
            Some(Provider::Exec(cmd, args.to_vec()))
        });

        assert_eq!(probes, [("tmux", vec!["load-buffer", "-w", "-"])]);
        assert!(matches!(
            provider,
            Provider::Exec("tmux", args) if args == ["load-buffer", "-w", "-"]
        ));
    }

    #[test]
    fn macos_ssh_session_never_uses_pbcopy() {
        let mut probes = Vec::new();
        let provider = super::detect_macos_copy_provider(true, false, |cmd, args| {
            probes.push((cmd, args.to_vec()));
            Some(Provider::Exec(cmd, args.to_vec()))
        });

        assert!(probes.is_empty());
        assert!(matches!(provider, Provider::OSC52));
    }

    #[test]
    fn macos_ssh_session_without_tmux_executable_falls_back_to_osc52() {
        let mut probes = Vec::new();
        let provider = super::detect_macos_copy_provider(true, true, |cmd, args| {
            probes.push((cmd, args.to_vec()));
            None
        });

        assert_eq!(probes, [("tmux", vec!["load-buffer", "-w", "-"])]);
        assert!(matches!(provider, Provider::OSC52));
    }

    #[test]
    fn local_macos_session_uses_pbcopy() {
        let mut probes = Vec::new();
        let provider = super::detect_macos_copy_provider(false, true, |cmd, args| {
            probes.push((cmd, args.to_vec()));
            Some(Provider::Exec(cmd, args.to_vec()))
        });

        assert_eq!(probes, [("pbcopy", Vec::new())]);
        assert!(matches!(
            provider,
            Provider::Exec("pbcopy", args) if args.is_empty()
        ));
    }

    #[test]
    fn local_macos_session_without_pbcopy_falls_back_to_osc52() {
        assert!(matches!(
            super::detect_macos_copy_provider(false, false, |_, _| None),
            Provider::OSC52
        ));
    }

    #[test]
    fn empty_environment_values_are_not_present() {
        assert!(!super::has_value(None));
        assert!(!super::has_value(Some(std::ffi::OsStr::new(""))));
        assert!(super::has_value(Some(std::ffi::OsStr::new("value"))));
    }

    #[cfg(windows)]
    const MISSING_PROVIDER: &str = r"C:\definitely\not\turbo-missing-clipboard-provider.exe";
    #[cfg(not(windows))]
    const MISSING_PROVIDER: &str = "/definitely/not/turbo-missing-clipboard-provider";

    #[test]
    fn exec_provider_spawn_failure_returns_error() {
        let result = copy_impl("content", &Provider::Exec(MISSING_PROVIDER, Vec::new()));

        assert!(result.is_err());
    }
}
