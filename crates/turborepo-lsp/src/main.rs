use std::ffi::{OsStr, OsString};

fn main() {
    if is_daemon_command() {
        run_daemon_command();
    }

    turborepo_lsp::run_lsp_server();
}

fn is_daemon_command() -> bool {
    has_daemon_command(std::env::args_os())
}

fn has_daemon_command(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter()
        .skip(1)
        .any(|arg| arg == OsStr::new("daemon"))
}

fn run_daemon_command() -> ! {
    // The Turborepo language server no longer embeds the `turbo daemon`
    // runtime, so it no longer depends on the application crate. The `daemon`
    // subcommand is kept as an informational no-op so that any lingering
    // invocation (e.g. the VS Code extension's daemon start/stop/status
    // commands) exits cleanly instead of failing.
    eprintln!("The turbo daemon is not available from the Turborepo language server.");
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::has_daemon_command;

    #[test]
    fn detects_daemon_command() {
        assert!(has_daemon_command([
            OsString::from("turborepo-lsp"),
            OsString::from("--skip-infer"),
            OsString::from("daemon"),
        ]));
    }

    #[test]
    fn ignores_lsp_mode() {
        assert!(!has_daemon_command([OsString::from("turborepo-lsp")]));
    }
}
