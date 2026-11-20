//! Informational CLI exits never require a healthy managed installation.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{fs, io, net::TcpListener, time::Duration};

#[test]
fn help_and_version_succeed_without_setup_with_the_future_flag_on_or_off() {
    for enabled in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("package.json"),
            r#"{"name":"fixture","private":true,"packageManager":"pnpm@10.0.0","scripts":{"build":"exit 97"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("turbo.json"),
            serde_json::json!({
                "futureFlags": {"experimentalSetup": enabled},
                "noUpdateNotifier": true,
                "agentGuidance": false,
                "tasks": {"build": {}}
            })
            .to_string(),
        )
        .unwrap();
        let invalid_lock = b"invalid lock; informational exits must not inspect it";
        fs::write(root.join("turbo.lock"), invalid_lock).unwrap();
        let mut original: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        original.sort();
        for args in [
            vec!["--help"],
            vec!["run", "--help"],
            vec!["run", "build", "-h"],
            vec!["--version"],
            vec!["run", "build", "--version"],
        ] {
            let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
            monitor.set_nonblocking(true).unwrap();
            let proxy = format!("http://{}", monitor.local_addr().unwrap());
            let output = assert_cmd::Command::new(env!("CARGO_BIN_EXE_turbo"))
                .timeout(Duration::from_secs(15))
                .env_clear()
                .current_dir(root)
                .args(&args)
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", root.join("home"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_DATA_HOME", root.join("data"))
                .env("DO_NOT_TRACK", "1")
                .env("TURBO_NO_UPDATE_NOTIFIER", "1")
                .env("HTTP_PROXY", &proxy)
                .env("HTTPS_PROXY", &proxy)
                .env("ALL_PROXY", &proxy)
                .env("NO_PROXY", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "enabled={enabled} {args:?}: {output:?}"
            );
            assert!(!output.stdout.is_empty());
            assert!(!String::from_utf8_lossy(&output.stderr).contains("managed"));
            assert_eq!(fs::read(root.join("turbo.lock")).unwrap(), invalid_lock);
            let mut after: Vec<_> = fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            after.sort();
            assert_eq!(after, original, "informational exit created local state");
            assert!(matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
        }
    }
}
