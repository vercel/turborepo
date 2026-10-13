//! TURBO-6239: a uv workspace without host uv or Python.
use serde_json::json;
use sha2::{Digest, Sha256};

use super::*;

const TURBO_JSON: &str = r#"{"futureFlags":{"experimentalPythonWorkspaces":true},
    "agentGuidance":false,"noUpdateNotifier":true,"tasks":{"build":{"cache":false}}}"#;
const PYPROJECT: &str = "[tool.turbo]\nname = \"acme\"\n\n[tool.uv]\nrequired-version = \
                         \">=0.13\"\n\n[tool.uv.workspace]\nmembers = [\"packages/*\"]\n";
const APP: &str = "[project]\nname = \"app\"\nversion = \"0.1.0\"\nrequires-python = \
                   \">=3.12\"\ndependencies = []\n\n[build-system]\nrequires = \
                   [\"uv_build>=0.8,<2\"]\nbuild-backend = \"uv_build\"\n";
const RELEASES: &str = "/github/python-build-standalone/releases/download/20260211";

/// Lists the fixture CPython for setup, answers turbo's uv identity probes
/// with the managed interpreter, and reports builds.
const UV: &str = r#"#!/bin/sh
case "$1 $2" in
"python list") cat <<'EOF'
LISTING
EOF
exit 0 ;;
esac
echo "uv $*" >> "$HOME/managed-calls"
case "$1 $2" in
"--version ") echo "uv 0.13.0" ;;
"python find") command -v python3 ;;
build*) echo "managed-uv $* python=$UV_PYTHON downloads=$UV_PYTHON_DOWNLOADS secret=${JOURNEY_SECRET:-filtered}" ;;
*) exit 1 ;;
esac
"#;
const PYTHON: &str =
    "#!/bin/sh\necho \"python $*\" >> \"$HOME/managed-calls\"\necho \"Python 3.12.12\"\n";

/// uv 0.13.0 and CPython 3.12.12 releases for this host.
fn routes() -> Vec<(String, Vec<u8>)> {
    let (os, arch, libc, target) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macos", "aarch64", "none", "aarch64-apple-darwin"),
        ("macos", _) => ("macos", "x86_64", "none", "x86_64-apple-darwin"),
        (_, "aarch64") => ("linux", "aarch64", "gnu", "aarch64-unknown-linux-gnu"),
        _ => ("linux", "x86_64", "gnu", "x86_64-unknown-linux-gnu"),
    };
    let filename = "cpython-3.12.12+20260211-host-install_only_stripped.tar.gz".replace('+', "%2B");
    let python = tar_gz(
        "python",
        &[("bin/python3.12", PYTHON), ("bin/python3", PYTHON)],
    );
    let entry = |(os, arch, libc): (&str, &str, &str), filename: &str| {
        json!({"key":format!("cpython-3.12.12-{os}-{arch}-{libc}"),
            "version":"3.12.12","os":os,"arch":arch,"libc":libc,"implementation":"cpython",
            "variant":"default","url":format!("https://releases.astral.sh{RELEASES}/{filename}")})
    };
    let mut listing = vec![entry((os, arch, libc), &filename)];
    let mut sums = format!(
        "{:x}  {}\n",
        Sha256::digest(&python),
        filename.replace("%2B", "+")
    );
    // Like real `--all-arches` output (#14691), list both Linux
    // architectures; only the host build is ever downloaded.
    for linux in [("linux", "x86_64", "gnu"), ("linux", "aarch64", "gnu")] {
        if linux != (os, arch, libc) {
            let peer = format!("cpython-3.12.12%2B20260211-{}-install_only.tar.gz", linux.1);
            sums += &format!("{:064x}  {}\n", 1, peer.replace("%2B", "+"));
            listing.push(entry(linux, &peer));
        }
    }
    let listing = serde_json::Value::Array(listing);
    let uv = tar_gz(
        &format!("uv-{target}"),
        &[
            ("uv", &UV.replace("LISTING", &listing.to_string())),
            ("uvx", "#!/bin/sh\nexit 1\n"),
        ],
    );
    let release = "/github/uv/releases/download/0.13.0";
    let uv_sums = format!("{:x} *uv-{target}.tar.gz\n", Sha256::digest(&uv));
    vec![
        (format!("{RELEASES}/{filename}"), python),
        (format!("{RELEASES}/SHA256SUMS"), sums.into_bytes()),
        (format!("{release}/uv-{target}.tar.gz"), uv),
        (format!("{release}/sha256.sum"), uv_sums.into_bytes()),
        (
            "/github/versions/main/v1/uv.ndjson".into(),
            format!("{}\n", json!({"version": "0.13.0", "artifacts": []})).into_bytes(),
        ),
    ]
}

#[test]
fn uv_setup_then_plain_run_uses_the_locked_uv_and_python_without_host_python() {
    let journey = Journey::new(
        &[
            ("turbo.json", TURBO_JSON),
            (".gitignore", "/.turbo/\n"),
            ("pyproject.toml", PYPROJECT),
            (".python-version", "3.12\n"),
            ("packages/app/pyproject.toml", APP),
            ("packages/app/src/app/__init__.py", ""),
        ],
        routes(),
    );
    journey.setup("packages/app", LOCAL);
    journey.setup("", &["--frozen", "--tools-only"]);
    let bin = journey.bin();
    assert!(bin.join("uv").exists() && bin.join("python3").exists());
    journey.calls();

    journey.runs_managed(
        "packages/app",
        &["build"],
        &[
            "managed-uv build --package=app",
            "python=cpython-3.12.12-",
            "downloads=never",
        ],
    );
    // turbo trusted exactly the managed uv inside the repository for its
    // identity probes, and they selected the managed interpreter.
    let calls = journey.calls();
    for probe in ["uv --version", "uv python find", "python --version"] {
        assert!(calls.contains(probe), "{probe}: {calls}");
    }
    journey.discovers_managed("uv workspace metadata");

    fs::remove_file(bin.join("uv")).unwrap();
    journey.fails("build", "run turbo setup to repair the installation");
    journey.discovery_fails("run turbo setup to repair the installation", &[]);
    journey.setup("", &["--frozen", "--tools-only"]);
    let version = journey.root().join(".python-version");
    fs::write(&version, "3.13\n").unwrap();
    journey.fails("build", "run turbo setup to update turbo.lock");
    fs::write(&version, "3.12\n").unwrap();
    let (code, text) = journey.turbo("", &["run", "build", "--ui=stream"]);
    assert_eq!(code, 0, "{text}");
}
