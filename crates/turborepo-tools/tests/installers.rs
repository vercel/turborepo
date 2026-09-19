#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! End-to-end installer tests against a mock of each upstream. Every
//! toolchain `turbo setup` supports is installed into a temporary
//! repository, its shims are executed, and the manifest is checked.
//!
//! Executables are shell scripts, so these tests are Unix only.

use std::{io::Write, process::Command};

use httpmock::prelude::*;
use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_tools::{
    InstallStatus, Installer, ToolsDir, declared, http::Downloader, install::SilentReporter,
    platform::Platform, sources::Sources,
};

/// `(path, contents, mode)` entries for a generated archive.
type Entry<'a> = (&'a str, &'a str, u32);

fn tar_gz(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (path, contents, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, path, contents.as_bytes())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn zip_archive(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, contents, mode) in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(*mode);
        writer.start_file(*path, options).unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write(root: &AbsoluteSystemPath, name: &str, contents: &str) {
    root.join_component(name)
        .create_with_contents(contents)
        .unwrap();
}

fn run(shim: &AbsoluteSystemPath, args: &[&str]) -> String {
    let output = Command::new(shim.as_std_path())
        .args(args)
        .env_remove("RUSTUP_HOME")
        .env_remove("UV_PYTHON_INSTALL_DIR")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} failed: {}",
        shim,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Harness {
    _tmp: tempfile::TempDir,
    repo: AbsoluteSystemPathBuf,
    server: MockServer,
    platform: Platform,
}

impl Harness {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        Self {
            _tmp: tmp,
            repo,
            server: MockServer::start(),
            platform: Platform::current().unwrap(),
        }
    }

    fn sources(&self) -> Sources {
        let base = self.server.base_url();
        Sources {
            node_dist: format!("{base}/node"),
            npm_registry: format!("{base}/registry"),
            bun_releases: format!("{base}/bun"),
            rustup_update_root: format!("{base}/rustup"),
            uv_releases: format!("{base}/uv"),
            pypi: format!("{base}/pypi"),
            go_dist: format!("{base}/go"),
        }
    }

    fn installer(&self) -> Installer<'_> {
        Installer::new(
            &self.repo,
            &SilentReporter,
            self.sources(),
            Downloader::with_client(reqwest::Client::new()),
        )
        .unwrap()
    }

    fn tools(&self) -> ToolsDir {
        ToolsDir::new(&self.repo)
    }

    fn bin(&self, name: &str) -> AbsoluteSystemPathBuf {
        self.tools().bin_dir().join_component(name)
    }

    fn serve(&self, path: &str, body: Vec<u8>) {
        let path = path.to_string();
        self.server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(200).body(body);
        });
    }

    /// Installs everything the repository declares and asserts the outcomes.
    async fn install_all(&self) -> Vec<turborepo_tools::InstallOutcome> {
        let declarations = declared::discover(&self.repo).unwrap();
        assert!(!declarations.is_empty(), "test repo must declare something");
        let mut installer = self.installer();

        let check = installer.check(&declarations).await.unwrap();
        assert!(
            check.iter().all(|o| o.status == InstallStatus::Missing),
            "{check:?}"
        );

        let outcomes = installer.install(&declarations, false).await.unwrap();
        assert!(
            outcomes
                .iter()
                .all(|o| o.status == InstallStatus::Installed),
            "{outcomes:?}"
        );

        // Idempotent: a second run touches nothing, --check is satisfied.
        let again = installer.install(&declarations, false).await.unwrap();
        assert!(
            again.iter().all(|o| o.status == InstallStatus::UpToDate),
            "{again:?}"
        );
        let check = installer.check(&declarations).await.unwrap();
        assert!(check.iter().all(|o| o.status.is_satisfied()), "{check:?}");
        outcomes
    }
}

#[tokio::test]
async fn node_from_range_with_declared_npm_override() {
    let h = Harness::new();
    write(&h.repo, ".nvmrc", "20\n");
    write(
        &h.repo,
        "package.json",
        r#"{"packageManager": "npm@10.5.0"}"#,
    );

    // nodejs.org index + release
    h.serve(
        "/node/index.json",
        br#"[{"version":"v22.1.0","lts":false},{"version":"v20.11.1","lts":"Iron"},{"version":"v20.11.0","lts":"Iron"}]"#.to_vec(),
    );
    let prefix = format!("node-v20.11.1-{}", h.platform.node_suffix());
    let node_archive = tar_gz(&[
        (
            &format!("{prefix}/bin/node"),
            "#!/bin/sh\necho v20.11.1\n",
            0o755,
        ),
        (
            &format!("{prefix}/lib/node_modules/npm/bin/npm-cli.js"),
            "bundled npm\n",
            0o644,
        ),
    ]);
    let file = format!("{prefix}.tar.gz");
    h.serve(
        "/node/v20.11.1/SHASUMS256.txt",
        format!("{}  {file}\n", sha256_hex(&node_archive)).into_bytes(),
    );
    h.serve(&format!("/node/v20.11.1/{file}"), node_archive);

    // npm registry: npm@10.5.0 overrides the bundled npm shim
    let npm_archive = tar_gz(&[
        (
            "package/package.json",
            r#"{"name":"npm","version":"10.5.0","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
            0o644,
        ),
        (
            "package/bin/npm-cli.js",
            "console.log('registry npm')\n",
            0o755,
        ),
        (
            "package/bin/npx-cli.js",
            "console.log('registry npx')\n",
            0o755,
        ),
    ]);
    let tarball = h.server.url("/registry/npm/-/npm-10.5.0.tgz");
    h.serve(
        "/registry/npm/10.5.0",
        format!(
            r#"{{"dist":{{"tarball":"{tarball}","integrity":"sha512-{}"}}}}"#,
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                sha2::Sha512::digest(&npm_archive)
            )
        )
        .into_bytes(),
    );
    h.serve("/registry/npm/-/npm-10.5.0.tgz", npm_archive);

    let outcomes = h.install_all().await;
    assert_eq!(outcomes[0].tool, "node");
    assert_eq!(outcomes[0].version, "20.11.1", "newest 20.x wins");
    assert_eq!(outcomes[1].tool, "npm");

    assert_eq!(run(&h.bin("node"), &[]), "v20.11.1");
    let npm_shim = h.bin("npm").read_to_string().unwrap();
    assert!(
        npm_shim.contains("npm/10.5.0/bin/npm-cli.js"),
        "declared npm must replace the bundled shim: {npm_shim}"
    );
    let manifest = h.tools().read_manifest().unwrap();
    assert_eq!(manifest.tools["node"].path, "node/20.11.1");
    assert_eq!(manifest.tools["npm"].bins, vec!["npm", "npx"]);
}

#[tokio::test]
async fn bun_from_github_zip() {
    let h = Harness::new();
    write(
        &h.repo,
        "package.json",
        r#"{"packageManager": "bun@1.1.0"}"#,
    );

    let dir = format!("bun-{}", h.platform.bun_suffix());
    let archive = zip_archive(&[(
        &format!("{dir}/bun"),
        "#!/bin/sh\necho \"bun $(basename \"$0\") $*\"\n",
        0o755,
    )]);
    let file = format!("{dir}.zip");
    h.serve(
        "/bun/bun-v1.1.0/SHASUMS256.txt",
        format!("{}  {file}\n", sha256_hex(&archive)).into_bytes(),
    );
    h.serve(&format!("/bun/bun-v1.1.0/{file}"), archive);

    h.install_all().await;
    assert_eq!(run(&h.bin("bun"), &["--version"]), "bun bun --version");
    // bunx is a second link to the same binary; bun keys off argv[0].
    assert_eq!(run(&h.bin("bunx"), &["x"]), "bun bunx x");
}

#[tokio::test]
async fn yarn_berry_comes_from_cli_dist() {
    let h = Harness::new();
    write(
        &h.repo,
        "package.json",
        r#"{"devEngines": {"packageManager": {"name": "yarn", "version": "^4.0.0"}}}"#,
    );

    let archive = tar_gz(&[
        (
            "package/package.json",
            r#"{"name":"@yarnpkg/cli-dist","version":"4.1.0","bin":{"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"}}"#,
            0o644,
        ),
        ("package/bin/yarn.js", "console.log('yarn 4')\n", 0o755),
    ]);
    let tarball = h
        .server
        .url("/registry/@yarnpkg/cli-dist/-/cli-dist-4.1.0.tgz");
    // Abbreviated packument for range resolution (scoped name is URL-encoded).
    h.server.mock(|when, then| {
        when.method(GET)
            .path_matches(regex::Regex::new(r"cli-dist(/4\.1\.0)?$").unwrap());
        then.status(200).body(
            format!(
                r#"{{"versions":{{"3.6.0":{{}},"4.0.2":{{}},"4.1.0":{{}}}},"dist":{{"tarball":"{tarball}"}}}}"#
            ),
        );
    });
    h.serve("/registry/@yarnpkg/cli-dist/-/cli-dist-4.1.0.tgz", archive);

    let outcomes = h.install_all().await;
    assert_eq!(outcomes[0].tool, "yarn");
    assert_eq!(outcomes[0].version, "4.1.0");
    let manifest = h.tools().read_manifest().unwrap();
    assert_eq!(manifest.tools["yarn"].bins, vec!["yarn", "yarnpkg"]);
    assert!(
        h.bin("yarn")
            .read_to_string()
            .unwrap()
            .contains("yarn/4.1.0/bin/yarn.js")
    );
}

#[tokio::test]
async fn go_language_version_resolves_to_newest_patch() {
    let h = Harness::new();
    write(&h.repo, "go.mod", "module example.com/m\n\ngo 1.22\n");

    h.server.mock(|when, then| {
        when.method(GET)
            .path("/go/")
            .query_param("mode", "json")
            .query_param("include", "all");
        then.status(200).body(
            br#"[{"version":"go1.23.0","stable":true},{"version":"go1.22.5","stable":true},{"version":"go1.22.6","stable":false},{"version":"go1.22.4","stable":true}]"#,
        );
    });
    let archive = tar_gz(&[
        ("go/bin/go", "#!/bin/sh\necho go1.22.5\n", 0o755),
        ("go/bin/gofmt", "#!/bin/sh\necho gofmt\n", 0o755),
        ("go/VERSION", "go1.22.5\n", 0o644),
    ]);
    let file = format!("go1.22.5.{}.tar.gz", h.platform.go_suffix());
    h.serve(
        &format!("/go/{file}.sha256"),
        sha256_hex(&archive).into_bytes(),
    );
    h.serve(&format!("/go/{file}"), archive);

    let outcomes = h.install_all().await;
    assert_eq!(outcomes[0].version, "1.22.5", "newest stable 1.22.x");
    assert_eq!(run(&h.bin("go"), &["version"]), "go1.22.5");
    assert_eq!(run(&h.bin("gofmt"), &[]), "gofmt");
    assert!(
        h.tools()
            .root()
            .join_components(&["go", "1.22.5", "VERSION"])
            .exists()
    );
}

#[tokio::test]
async fn uv_then_python_through_managed_uv() {
    let h = Harness::new();
    write(
        &h.repo,
        "pyproject.toml",
        "[project]\nname = \"x\"\n[tool.uv]\nrequired-version = \">=0.5,<0.6\"\n",
    );
    write(&h.repo, ".python-version", "3.12\n");

    h.serve(
        "/pypi/uv/json",
        br#"{"releases":{"0.4.9":[{"yanked":false}],"0.5.3":[{"yanked":false}],"0.5.9":[{"yanked":true}],"0.6.0":[]}}"#.to_vec(),
    );
    let triple = h.platform.rust_triple();
    // The fake uv records the install dir it was given so the test can prove
    // both the shim env and the installer env point into the repository.
    let fake_uv = "#!/bin/sh\nif [ \"$1\" = python ] && [ \"$2\" = install ]; then\nmkdir -p \
                   \"$UV_PYTHON_INSTALL_DIR/cpython-$3\" && echo \"installed $3 into \
                   $UV_PYTHON_INSTALL_DIR\"\nelse\necho \"uv 0.5.3 \
                   dir=$UV_PYTHON_INSTALL_DIR\"\nfi\n";
    let archive = tar_gz(&[
        (&format!("uv-{triple}/uv"), fake_uv, 0o755),
        (&format!("uv-{triple}/uvx"), "#!/bin/sh\necho uvx\n", 0o755),
    ]);
    let file = format!("uv-{triple}.tar.gz");
    h.serve(
        &format!("/uv/0.5.3/{file}.sha256"),
        format!("{} *{file}\n", sha256_hex(&archive)).into_bytes(),
    );
    h.serve(&format!("/uv/0.5.3/{file}"), archive);

    let outcomes = h.install_all().await;
    assert_eq!(outcomes[0].tool, "uv");
    assert_eq!(outcomes[0].version, "0.5.3", "yanked 0.5.9 is skipped");
    assert_eq!(outcomes[1].tool, "python");

    let python_dir = h.tools().root().join_component("python");
    assert!(
        python_dir.join_component("cpython-3.12").exists(),
        "python installed through the managed uv"
    );
    // The uv shim carries UV_PYTHON_INSTALL_DIR itself.
    assert_eq!(
        run(&h.bin("uv"), &["--version"]),
        format!("uv 0.5.3 dir={python_dir}")
    );
    let manifest = h.tools().read_manifest().unwrap();
    assert_eq!(
        manifest.tools["python"].env["UV_PYTHON_INSTALL_DIR"],
        "python"
    );
    let env = h.tools().activation_env(None).unwrap();
    assert!(env.iter().any(
        |(k, v)| k == "UV_PYTHON_INSTALL_DIR" && v == std::ffi::OsStr::new(python_dir.as_str())
    ));
}

#[tokio::test]
async fn rust_bootstraps_a_repository_scoped_rustup() {
    let h = Harness::new();
    write(
        &h.repo,
        "rust-toolchain.toml",
        "[toolchain]\nchannel = \"1.80.0\"\ncomponents = [\"clippy\"]\nprofile = \"minimal\"\n",
    );

    // A fake rustup-init that behaves like the real one for our purposes: it
    // writes a `rustup` proxy into CARGO_HOME/bin which in turn records
    // toolchain installs under RUSTUP_HOME and creates cargo/rustc proxies.
    let fake_rustup_init = r#"#!/bin/sh
set -e
mkdir -p "$CARGO_HOME/bin" "$RUSTUP_HOME"
echo "$*" > "$RUSTUP_HOME/init-args"
cat > "$CARGO_HOME/bin/rustup" <<'EOF'
#!/bin/sh
set -e
case "$1 $2" in
  "toolchain install")
    shift 2; channel="$1"; shift
    mkdir -p "$RUSTUP_HOME/toolchains/$channel"
    echo "$*" > "$RUSTUP_HOME/toolchains/$channel/args"
    for proxy in cargo rustc; do
      printf '#!/bin/sh\necho "%s from $RUSTUP_HOME"\n' "$proxy" > "$(dirname "$0")/$proxy"
      chmod +x "$(dirname "$0")/$proxy"
    done ;;
  "default "*) echo "$2" > "$RUSTUP_HOME/default" ;;
  *) echo "rustup $*" ;;
esac
EOF
chmod +x "$CARGO_HOME/bin/rustup"
"#;
    let triple = h.platform.rust_triple();
    h.serve(
        &format!("/rustup/dist/{triple}/rustup-init.sha256"),
        format!("{} *rustup-init\n", sha256_hex(fake_rustup_init.as_bytes())).into_bytes(),
    );
    h.serve(
        &format!("/rustup/dist/{triple}/rustup-init"),
        fake_rustup_init.as_bytes().to_vec(),
    );

    let outcomes = h.install_all().await;
    assert_eq!(outcomes[0].tool, "rust");
    assert_eq!(outcomes[0].version, "1.80.0");

    let rust = h.tools().root().join_component("rust");
    let rustup_home = rust.join_component("rustup");
    assert_eq!(
        rustup_home
            .join_component("init-args")
            .read_to_string()
            .unwrap()
            .trim(),
        "-y --no-modify-path --default-toolchain none --profile minimal"
    );
    assert_eq!(
        rustup_home
            .join_components(&["toolchains", "1.80.0", "args"])
            .read_to_string()
            .unwrap()
            .trim(),
        "--profile minimal --component clippy"
    );
    assert_eq!(
        rustup_home
            .join_component("default")
            .read_to_string()
            .unwrap()
            .trim(),
        "1.80.0"
    );
    assert!(
        !rust.join_component("rustup-init").exists(),
        "installer removed after use"
    );

    // Proxies are exposed and carry RUSTUP_HOME even from a bare shell.
    assert_eq!(
        run(&h.bin("cargo"), &["--version"]),
        format!("cargo from {rustup_home}")
    );
    assert_eq!(
        run(&h.bin("rustc"), &[]),
        format!("rustc from {rustup_home}")
    );
    let manifest = h.tools().read_manifest().unwrap();
    assert_eq!(manifest.tools["rust"].env["RUSTUP_HOME"], "rust/rustup");
    assert_eq!(
        manifest.tools["rust"].bins,
        vec!["cargo", "rustc", "rustup"]
    );
}

#[tokio::test]
async fn force_reinstalls_and_checksum_mismatch_fails() {
    let h = Harness::new();
    write(&h.repo, "go.work", "go 1.22\n\ntoolchain go1.22.5\n");
    let archive = tar_gz(&[("go/bin/go", "#!/bin/sh\necho go\n", 0o755)]);
    let file = format!("go1.22.5.{}.tar.gz", h.platform.go_suffix());
    // Wrong checksum first.
    let mut bad = h.server.mock(|when, then| {
        when.method(GET).path(format!("/go/{file}.sha256"));
        then.status(200).body("00".repeat(32));
    });
    h.serve(&format!("/go/{file}"), archive.clone());

    let declarations = declared::discover(&h.repo).unwrap();
    let mut installer = h.installer();
    let err = installer.install(&declarations, false).await.unwrap_err();
    assert!(
        matches!(err, turborepo_tools::Error::ChecksumMismatch { .. }),
        "{err}"
    );
    assert!(h.tools().read_manifest().unwrap().tools.is_empty());
    assert!(!h.tools().root().join_components(&["go", "1.22.5"]).exists());

    bad.delete();
    h.serve(
        &format!("/go/{file}.sha256"),
        sha256_hex(&archive).into_bytes(),
    );
    let outcomes = installer.install(&declarations, false).await.unwrap();
    assert_eq!(outcomes[0].status, InstallStatus::Installed);
    let forced = installer.install(&declarations, true).await.unwrap();
    assert_eq!(
        forced[0].status,
        InstallStatus::Installed,
        "--force reinstalls"
    );
}
