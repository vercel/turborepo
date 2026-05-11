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
    std::panic::set_hook(Box::new(turborepo_lib::panic_handler));

    let exit_code = turborepo_lib::main(None).unwrap_or_else(|err| {
        eprintln!("{err:?}");
        1
    });

    std::process::exit(exit_code)
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
