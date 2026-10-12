#![allow(clippy::unwrap_used)]

use super::*;
use crate::{
    lock::Platform, rust_metadata::tests::fixture, source_policy::test_support::LoopbackServer,
};

fn toolchain(body: &str) -> Toolchain {
    Toolchain::parse("rust-toolchain.toml", &format!("[toolchain]\n{body}")).unwrap()
}

fn parts(tool: &Tool, platform: Platform) -> &BTreeMap<String, Artifact> {
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("managed rust");
    };
    &artifacts[&platform]
}

#[test]
fn stable_resolves_to_the_exact_release_with_default_profile_parts() {
    let manifest = fixture("1.99.0", "2026-10-01");
    let tool = resolve(&toolchain("channel = \"stable\"\n"), manifest.as_bytes()).unwrap();
    assert_eq!(
        (tool.adapter.as_str(), tool.version.as_str()),
        ("rust", "1.99.0")
    );
    assert_eq!(tool.options["profile"], ["default"]);
    assert!(!tool.options.contains_key("components"));
    assert_eq!(tool.declarations[0].request.as_deref(), Some("stable"));
    let Installation::Managed { artifacts } = &tool.installation else {
        unreachable!()
    };
    // Only hosts the manifest publishes rustc for are recorded.
    assert_eq!(
        artifacts.keys().collect::<Vec<_>>(),
        [&Platform::MacosArm64, &Platform::LinuxX64Gnu]
    );
    let linux = parts(&tool, Platform::LinuxX64Gnu);
    // rust-mingw is a profile member this host lacks, exactly as rustup skips it.
    assert_eq!(
        linux.keys().collect::<Vec<_>>(),
        ["cargo", "clippy-preview", "rust-std", "rustc"]
    );
    let std = &linux["rust-std"];
    assert_eq!(
        std.root_prefix.as_deref(),
        Some("rust-std-1.99.0-x86_64-unknown-linux-gnu/rust-std-x86_64-unknown-linux-gnu")
    );
    assert!(std.executables.is_empty() && std.format == Format::TarGz);
    let clippy = &parts(&tool, Platform::MacosArm64)["clippy-preview"];
    assert_eq!(
        clippy.root_prefix.as_deref(),
        Some("clippy-1.99.0-aarch64-apple-darwin/clippy-preview")
    );
    assert_eq!(clippy.executables["cargo-clippy"], "bin/cargo-clippy");
    assert_eq!(linux["rustc"].executables["rustdoc"], "bin/rustdoc");
}

#[test]
fn release_and_dated_requests_must_match_the_manifest() {
    let stable = fixture("1.99.0", "2026-10-01");
    for channel in ["1.99", "1.99.0"] {
        let tool = resolve(
            &toolchain(&format!("channel = \"{channel}\"\n")),
            stable.as_bytes(),
        );
        assert_eq!(tool.unwrap().version, "1.99.0");
    }
    let nightly = fixture("1.101.0-nightly", "2026-10-11");
    for (channel, manifest) in [
        ("1.98", &stable),
        ("1.9", &stable),
        ("1.99.1", &stable),
        ("nightly", &stable),
        ("stable", &nightly),
        ("beta", &nightly),
        ("nightly-2026-10-10", &nightly),
    ] {
        let toolchain = toolchain(&format!("channel = \"{channel}\"\n"));
        assert!(
            matches!(
                resolve(&toolchain, manifest.as_bytes()),
                Err(Error::Channel)
            ),
            "{channel}"
        );
    }
    for channel in ["nightly", "nightly-2026-10-11"] {
        let toolchain = toolchain(&format!("channel = \"{channel}\"\n"));
        let tool = resolve(&toolchain, nightly.as_bytes()).unwrap();
        assert_eq!(tool.version, "nightly-2026-10-11");
        let rustc = &parts(&tool, Platform::LinuxX64Gnu)["rustc"];
        assert_eq!(
            rustc.root_prefix.as_deref(),
            Some("rustc-nightly-x86_64-unknown-linux-gnu/rustc")
        );
    }
}

#[test]
fn components_renames_and_targets_extend_every_recorded_host() {
    let manifest = fixture("1.99.0", "2026-10-01");
    let tool = resolve(
        &toolchain(
            "channel = \"stable\"\nprofile = \"minimal\"\ncomponents = [\"rust-src\", \
             \"clippy\"]\ntargets = [\"wasm32-unknown-unknown\", \"x86_64-unknown-linux-gnu\"]\n",
        ),
        manifest.as_bytes(),
    )
    .unwrap();
    assert_eq!(tool.options["profile"], ["minimal"]);
    assert_eq!(tool.options["components"], ["clippy", "rust-src"]);
    assert_eq!(
        tool.options["targets"],
        ["wasm32-unknown-unknown", "x86_64-unknown-linux-gnu"]
    );
    for host in [Platform::LinuxX64Gnu, Platform::MacosArm64] {
        let parts = parts(&tool, host);
        let wasm = &parts["rust-std-wasm32-unknown-unknown"];
        assert_eq!(
            wasm.root_prefix.as_deref(),
            Some("rust-std-1.99.0-wasm32-unknown-unknown/rust-std-wasm32-unknown-unknown")
        );
        assert_eq!(
            parts["rust-src"].root_prefix.as_deref(),
            Some("rust-src-1.99.0/rust-src")
        );
        assert!(parts.contains_key("clippy-preview"));
    }
    // The macOS host also cross-targets Linux; Linux's own std is its host part.
    assert!(parts(&tool, Platform::MacosArm64).contains_key("rust-std-x86_64-unknown-linux-gnu"));
    assert_eq!(parts(&tool, Platform::LinuxX64Gnu).len(), 6);
}

#[test]
fn unpublished_components_and_targets_are_named() {
    let manifest = fixture("1.99.0", "2026-10-01");
    for (body, missing) in [
        ("components = [\"miri\"]\n", "miri"),
        (
            "targets = [\"riscv64gc-unknown-none-elf\"]\n",
            "riscv64gc-unknown-none-elf",
        ),
    ] {
        let toolchain = toolchain(&format!("channel = \"stable\"\n{body}"));
        assert!(matches!(
            resolve(&toolchain, manifest.as_bytes()),
            Err(Error::Unavailable(name)) if name == missing
        ));
    }
    assert!(matches!(
        resolve(
            &toolchain("channel = \"stable\"\n"),
            b"manifest-version = \"2\""
        ),
        Err(Error::Metadata(_))
    ));
}

#[tokio::test]
async fn official_resolution_reads_only_the_requested_channel_manifest() {
    let manifest = fixture("1.101.0-nightly", "2026-10-11");
    let path = "/dist/2026-10-11/channel-rust-nightly.toml";
    let server = LoopbackServer::new([(path.into(), manifest.into_bytes())], |_| {}).unwrap();
    let transport = RustupTransport::loopback_http_for_tests(server.origin()).unwrap();
    let tool = resolve_official(&transport, &toolchain("channel = \"nightly-2026-10-11\"\n"))
        .await
        .unwrap();
    assert_eq!(tool.version, "nightly-2026-10-11");
    assert_eq!(server.requests(), [path]);
}

mod lock_updates {
    use std::{fs, path::Path, process::Command};

    use super::*;
    use crate::lock::{
        Ecosystems, Snapshot,
        reconcile::{self, Mode},
    };

    fn upstream(stable: &str) -> LoopbackServer {
        let mut routes = vec![
            (
                "/rustup/release-stable.toml".to_owned(),
                b"schema-version = '1'\nversion = '1.29.1'\n".to_vec(),
            ),
            (
                "/dist/channel-rust-stable.toml".into(),
                fixture(stable, "2026-10-01").into_bytes(),
            ),
            (
                "/dist/channel-rust-1.99.0.toml".into(),
                fixture("1.99.0", "2026-10-01").into_bytes(),
            ),
        ];
        for platform in rustup::PLATFORMS {
            let url = rustup::installer_url(&semver::Version::new(1, 29, 1), platform).unwrap();
            let path = url.strip_prefix("https://static.rust-lang.org").unwrap();
            routes.push((
                format!("{path}.sha256"),
                format!("{}\n", "a".repeat(64)).into_bytes(),
            ));
        }
        LoopbackServer::new(routes, |_| {}).unwrap()
    }

    fn update(root: &Path, server: &LoopbackServer, mode: Mode) -> Lock {
        let transport = RustupTransport::loopback_http_for_tests(server.origin()).unwrap();
        let snapshot = Snapshot::capture_with(
            root,
            Ecosystems {
                rust: true,
                ..Ecosystems::default()
            },
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        reconcile::reconcile(&snapshot, mode, false, |request| {
            let tools = runtime
                .block_on(resolve_request(&request, &transport))
                .unwrap();
            Ok(Lock::new(Document {
                schema_version: lock::SCHEMA_VERSION,
                tools,
            })?)
        })
        .unwrap()
        .lock
    }

    #[test]
    fn edits_keep_the_locked_release_until_an_explicit_refresh() {
        let root = tempfile::tempdir().unwrap();
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .arg(root.path())
            .status();
        assert!(git.unwrap().success());
        fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
        let file = root.path().join("rust-toolchain.toml");
        fs::write(
            &file,
            "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\n",
        )
        .unwrap();
        let created = update(root.path(), &upstream("1.99.0"), Mode::Local);
        assert_eq!(created.tools()["rust"].version, "1.99.0");
        assert_eq!(created.tools()["rustup"].version, "1.29.1");

        // A newer stable exists, but only the component edit re-resolves.
        let newer = upstream("1.100.0");
        let edited = "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\ncomponents = \
                      [\"rust-src\"]\n";
        fs::write(&file, edited).unwrap();
        let targeted = update(root.path(), &newer, Mode::Local);
        assert_eq!(newer.requests(), ["/dist/channel-rust-1.99.0.toml"]);
        assert_eq!(targeted.tools()["rust"].version, "1.99.0");
        assert_eq!(targeted.tools()["rust"].options["components"], ["rust-src"]);
        assert_eq!(targeted.tools()["rustup"], created.tools()["rustup"]);

        let refreshed = update(root.path(), &newer, Mode::Refresh);
        assert_eq!(refreshed.tools()["rust"].version, "1.100.0");
        assert!(
            newer
                .requests()
                .contains(&"/dist/channel-rust-stable.toml".to_owned())
        );

        // Removing the file drops both selections without any resolution.
        fs::remove_file(&file).unwrap();
        let offline = LoopbackServer::new([], |_| {}).unwrap();
        assert!(
            update(root.path(), &offline, Mode::Local)
                .tools()
                .is_empty()
        );
        assert_eq!(offline.hits(), 0);
    }
}
