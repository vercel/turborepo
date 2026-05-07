use std::ffi::{OsStr, OsString};

use tower_lsp::{LspService, Server};
use turborepo_lsp::Backend;

fn main() {
    if is_daemon_command() {
        run_daemon_command();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    runtime.block_on(run_lsp());
}

async fn run_lsp() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
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
