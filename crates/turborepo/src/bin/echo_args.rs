// Native test peer. Synthetic reports are not capabilities of the turbo CLI.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--fixture-sleep"] {
        std::thread::sleep(std::time::Duration::from_secs(60));
        return;
    }
    if args == ["--__internal-managed-run-capabilities"] {
        let fixture = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|p| p.join("capability-peer.txt")))
            .and_then(|path| std::fs::read_to_string(path).ok());
        if let Some(fixture) = fixture {
            match fixture.as_str() {
                "hang" => {
                    if let Some(path) = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|p| p.join("capability-peer.pid")))
                    {
                        let _ = std::fs::write(path, std::process::id().to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
                "held-pipes" | "escaped-pipes" | "background" => {
                    if let Ok(binary) = std::env::current_exe() {
                        let mut command = std::process::Command::new(&binary);
                        command
                            .arg("--fixture-sleep")
                            .stdin(std::process::Stdio::null());
                        if fixture == "background" {
                            command
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null());
                        }
                        #[cfg(unix)]
                        if fixture == "escaped-pipes" {
                            use std::os::unix::process::CommandExt;
                            command.process_group(0);
                        }
                        if let Ok(child) = command.spawn()
                            && let Some(parent) = binary.parent()
                        {
                            let _ = std::fs::write(
                                parent.join("capability-descendant.pid"),
                                child.id().to_string(),
                            );
                        }
                    }
                    if fixture == "background" {
                        print!(
                            "{{\"abiVersion\":1,\"cliVersion\":\"synthetic-peer\",\"\
                             managedRunSchemas\":[0]}}"
                        );
                    }
                }
                "stderr" => eprintln!("unexpected peer warning"),
                "nonzero" => std::process::exit(7),
                "oversize" => print!("{}", "x".repeat(8192)),
                "environment" => {
                    let clean = std::env::var("TURBO_BINARY_PATH").is_err()
                        && std::env::var("TURBO_DOWNLOAD_LOCAL_ENABLED").as_deref() == Ok("0")
                        && std::env::var("DO_NOT_TRACK").as_deref() == Ok("1")
                        && std::env::var("TURBO_NO_UPDATE_NOTIFIER").as_deref() == Ok("1");
                    if clean {
                        print!(
                            "{{\"abiVersion\":1,\"cliVersion\":\"synthetic-peer\",\"\
                             managedRunSchemas\":[0]}}"
                        );
                    } else {
                        std::process::exit(9);
                    }
                }
                _ => print!("{fixture}"),
            }
            return;
        }
    }
    println!("{}", args.join(" "));
}
