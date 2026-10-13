//! TURBO-6249: a Go workspace without host Go.
use serde_json::json;
use sha2::{Digest, Sha256};

use super::*;

const TURBO_JSON: &str = r#"{"futureFlags":{"experimentalGoWorkspaces":true},
    "agentGuidance":false,"noUpdateNotifier":true,"tasks":{"build":{"cache":false},"format":{"cache":false}}}"#;

/// Answers turbo's Go discovery probes for one `./app` module, then reports
/// how each task invoked it and whether GOROOT is this locked tree.
const GO: &str = r#"#!/bin/sh
echo "go $*" >> "$HOME/managed-calls"
root=$(pwd -P)
case "$1 $2" in
"work edit") echo '{"Go":"1.22","Use":[{"DiskPath":"./app"}]}' ;;
"mod edit") echo '{"Module":{"Path":"example.com/app"}}' ;;
"mod graph") ;;
"list -mod=readonly") printf '{"Path":"example.com/app","Main":true,"Dir":"%s/app"}\n' "$root" ;;
"env -json")
  shift 2
  sep='{'
  for name in "$@"; do
    case "$name" in GOOS) value=OS ;; GOCACHE) value="$HOME/go-cache" ;; GOMODCACHE) value="$HOME/go-mod" ;; *) value= ;; esac
    printf '%s"%s":"%s"' "$sep" "$name" "$value"
    sep=,
  done
  echo '}' ;;
"version ") echo "go version go1.22.12 OS/ARCH" ;;
"fmt "*) exec gofmt -l . ;;
build*)
  [ "$GOROOT/bin/go" -ef "$0" ] && goroot=locked || goroot=other
  echo "managed-go $* GOTOOLCHAIN=$GOTOOLCHAIN goroot=$goroot secret=${JOURNEY_SECRET:-filtered}" ;;
*) exit 1 ;;
esac
"#;
const GOFMT: &str = "#!/bin/sh\necho \"managed-gofmt $* secret=${JOURNEY_SECRET:-filtered}\"\n";

/// The Go release index and this host's archive for 1.22.12.
fn routes() -> Vec<(String, Vec<u8>)> {
    let (os, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("darwin", "arm64"),
        ("macos", _) => ("darwin", "amd64"),
        (_, "aarch64") => ("linux", "arm64"),
        _ => ("linux", "amd64"),
    };
    let go = GO
        .replace("OS/ARCH", &format!("{os}/{arch}"))
        .replace("=OS ", &format!("={os} "));
    let archive = tar_gz(
        "go",
        &[
            ("bin/go", &go),
            ("bin/gofmt", GOFMT),
            ("src/go.mod", "module std\n"),
        ],
    );
    let mut files = Vec::new();
    let mut routes = Vec::new();
    for (target_os, target_arch) in [
        ("darwin", "amd64"),
        ("darwin", "arm64"),
        ("linux", "amd64"),
        ("linux", "arm64"),
    ] {
        let name = format!("go1.22.12.{target_os}-{target_arch}.tar.gz");
        let sha256 = if (target_os, target_arch) == (os, arch) {
            routes.push((format!("/go/{name}"), archive.clone()));
            format!("{:x}", Sha256::digest(&archive))
        } else {
            "0".repeat(64)
        };
        files.push(
            json!({"filename": name, "os": target_os, "arch": target_arch,
            "sha256": sha256, "version": "go1.22.12", "size": 1, "kind": "archive"}),
        );
    }
    let index = json!([{"version": "go1.22.12", "stable": true, "files": files}]);
    routes.push((
        turborepo_setup::go_resolution::INDEX_PATH.into(),
        index.to_string().into_bytes(),
    ));
    routes
}

#[test]
fn go_setup_then_plain_run_uses_the_locked_toolchain_without_host_go() {
    let journey = Journey::new(
        &[
            ("turbo.json", TURBO_JSON),
            (".gitignore", "/.turbo/\n"),
            ("go.work", "go 1.22\n\nuse ./app\n"),
            ("app/go.mod", "module example.com/app\n\ngo 1.22\n"),
            ("app/main.go", "package main\n\nfunc main() {}\n"),
        ],
        routes(),
    );
    // Setup from a nested directory creates the lock; the repeat is frozen.
    journey.setup("app", LOCAL);
    journey.setup("", &["--frozen", "--tools-only"]);
    let bin = journey.bin();
    assert!(bin.join("go").exists() && bin.join("gofmt").exists());
    journey.calls();

    journey.runs_managed(
        "app",
        &["build", "format"],
        &[
            "managed-go build ./... GOTOOLCHAIN=local goroot=locked",
            "managed-gofmt -l .",
        ],
    );
    // turbo's own discovery and identity probes ran the managed go, too.
    let calls = journey.calls();
    for probe in ["go work edit -json", "go env -json", "go version"] {
        assert!(calls.contains(probe), "{probe}: {calls}");
    }

    // A missing export is an installation repair; a changed declaration is a
    // lock update. Neither falls back to a host go nor downloads.
    fs::remove_file(bin.join("go")).unwrap();
    journey.fails("build", "run turbo setup to repair the installation");
    journey.setup("", &["--frozen", "--tools-only"]);
    let work = journey.root().join("go.work");
    fs::write(&work, "go 1.22\ntoolchain go1.23.4\n\nuse ./app\n").unwrap();
    journey.fails("build", "run turbo setup to update turbo.lock");
    fs::write(&work, "go 1.22\n\nuse ./app\n").unwrap();
    let (code, text) = journey.turbo("", &["run", "build", "--ui=stream"]);
    assert_eq!(code, 0, "{text}");
}
