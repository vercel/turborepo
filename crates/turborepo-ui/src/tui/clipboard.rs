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
    if let Some(provider) = check_prog("pbcopy", &[]) {
        return provider;
    }
    Provider::OSC52
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_copy_provider() -> Provider {
    // Wayland
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        if let Some(provider) = check_prog("wl-copy", &["--type", "text/plain"]) {
            return provider;
        }
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
    if std::env::var("TMUX").is_ok() {
        if let Some(provider) = check_prog("tmux", &["load-buffer", "-"]) {
            return provider;
        }
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
                .spawn()
                .unwrap();
            std::io::Write::write_all(&mut child.stdin.as_ref().unwrap(), s.as_bytes())?;
            child.wait()?;
        }

        #[cfg(windows)]
        Provider::Win => clipboard_win::set_clipboard_string(s)
            .map_err(|e| std::io::Error::other(e.to_string()))?,

        Provider::NoOp => (),
    };

    Ok(())
}

lazy_static::lazy_static! {
  static ref PROVIDER: Provider = detect_copy_provider();
}
