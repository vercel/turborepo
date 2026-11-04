#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

#[test]
fn completion_requests_work_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    for shell in ["bash", "zsh", "fish", "powershell", "nu", "elvish"] {
        for (line, candidate) in [
            ("turbo da", "daemon"),
            ("turbo daemon st", "status"),
            ("turbo run --dry", "--dry-run"),
            ("turbo --cwd \"folder with spaces\" da", "daemon"),
        ] {
            let output = common::run_turbo(
                dir.path(),
                &["__complete_word__", "--shell", shell, "--line", line],
            );
            let combined = common::combined_output(&output);
            assert!(output.status.success(), "{shell}: {line}: {combined}");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains(candidate),
                "{shell}: {line}: missing {candidate}: {combined}"
            );
            assert!(output.stderr.is_empty(), "{shell}: {line}: {combined}");
        }
    }
}

#[test]
fn completion_requests_accept_the_shim_prefix() {
    let dir = tempfile::tempdir().unwrap();
    for shell in ["bash", "zsh", "fish", "powershell", "nu", "elvish"] {
        let request = ["__complete_word__", "--shell", shell, "--line", "turbo da"];
        let direct = common::run_turbo(dir.path(), &request);
        let forwarded: Vec<_> = std::iter::once("--skip-infer").chain(request).collect();
        let forwarded = common::run_turbo(dir.path(), &forwarded);
        assert!(
            forwarded.status.success(),
            "{shell}: {}",
            common::combined_output(&forwarded)
        );
        assert_eq!(forwarded.stdout, direct.stdout, "{shell}");
        assert!(forwarded.stderr.is_empty(), "{shell}");
    }
}

#[test]
fn elvish_word_requests_preserve_cursor_words() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[(&[&str], &str)] = &[
        (&["turbo", "da"], "daemon"),
        (&["turbo", ""], "daemon"),
        (&["turbo", "--cwd", "folder with spaces", "da"], "daemon"),
        (&["turbo", "run", "--"], "--filter"),
        (&["turbo", "da", "--"], "--filter"),
    ];
    for prefix in [None, Some("--skip-infer")] {
        for (words, candidate) in cases {
            let args: Vec<_> = prefix
                .into_iter()
                .chain(["__complete_word__", "--shell", "elvish", "--words"])
                .chain(words.iter().copied())
                .collect();
            let output = common::run_turbo(dir.path(), &args);
            let combined = common::combined_output(&output);
            assert!(output.status.success(), "{args:?}: {combined}");
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.split('\t').next() == Some(*candidate)),
                "{args:?}: missing {candidate}: {combined}"
            );
            assert!(output.stderr.is_empty(), "{args:?}: {combined}");
        }
    }
}

#[test]
fn completion_marker_in_task_arguments_does_not_become_a_request() {
    let dir = tempfile::tempdir().unwrap();
    for prefix in [None, Some("--skip-infer")] {
        let args: Vec<_> = prefix
            .into_iter()
            .chain([
                "run",
                "build",
                "--",
                "__complete_word__",
                "--shell",
                "bash",
                "--line",
                "turbo da",
            ])
            .collect();
        let output = common::run_turbo(dir.path(), &args);
        assert!(!output.status.success());
        assert!(
            output.stdout.is_empty(),
            "forwarded arguments produced completions: {}",
            common::combined_output(&output)
        );
    }
}

#[cfg(unix)]
#[test]
fn generated_bash_script_completes_commands() {
    let dir = tempfile::tempdir().unwrap();
    let output = common::run_turbo(dir.path(), &["completion", "bash"]);
    assert!(
        output.status.success(),
        "{}",
        common::combined_output(&output)
    );
    let script = dir.path().join("completion.bash");
    std::fs::write(&script, output.stdout).unwrap();
    let binary = assert_cmd::cargo::cargo_bin("turbo");
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .unwrap();
    let output = std::process::Command::new("bash")
        .args([
            "--noprofile",
            "--norc",
            "-c",
            r#"
source "$1"
COMP_LINE="turbo da"; COMP_POINT=8; COMP_WORDS=(turbo da); COMP_CWORD=1
_usage_complete_turbo || exit 1
[[ " ${COMPREPLY[*]} " == *" daemon "* ]]
"#,
            "bash",
        ])
        .arg(script)
        .env("PATH", path)
        .env("DO_NOT_TRACK", "1")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        common::combined_output(&output)
    );
}
