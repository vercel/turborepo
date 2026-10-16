#![allow(clippy::unwrap_used)]

use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::{Value, json};

use super::*;
use crate::source_policy::test_support::LoopbackServer;

const TAG: &str = "20260211";
/// Every lock platform, as one `--all-arches` listing reports from any host.
const TARGETS: [(&str, &str, &str, &str); 8] = [
    ("macos", "aarch64", "none", "aarch64-apple-darwin"),
    ("linux", "x86_64", "gnu", "x86_64-unknown-linux-gnu"),
    ("windows", "x86_64", "none", "x86_64-pc-windows-msvc"),
    ("macos", "x86_64", "none", "x86_64-apple-darwin"),
    ("linux", "aarch64", "gnu", "aarch64-unknown-linux-gnu"),
    ("linux", "x86_64", "musl", "x86_64-unknown-linux-musl"),
    ("linux", "aarch64", "musl", "aarch64-unknown-linux-musl"),
    ("windows", "aarch64", "none", "aarch64-pc-windows-msvc"),
];
/// More architectures and CPU variants uv 0.13 lists with `--all-arches`.
const UNSUPPORTED: [(&str, &str, &str, &str); 6] = [
    ("linux", "x86_64_v3", "gnu", "x86_64_v3-unknown-linux-gnu"),
    ("linux", "x86", "gnu", "i686-unknown-linux-gnu"),
    ("windows", "x86", "none", "i686-pc-windows-msvc"),
    ("linux", "powerpc64le", "gnu", "ppc64le-unknown-linux-gnu"),
    ("linux", "s390x", "gnu", "s390x-unknown-linux-gnu"),
    ("linux", "riscv64gc", "gnu", "riscv64-unknown-linux-gnu"),
];

fn requirement(request: &str) -> PythonRequirement {
    PythonRequirement::read_with(|file, _| {
        Ok((file == ".python-version").then(|| request.as_bytes().to_vec()))
    })
    .unwrap()
    .unwrap()
}
fn filename(version: &str, target: &str) -> String {
    format!("cpython-{version}+{TAG}-{target}-install_only_stripped.tar.gz")
}
fn entry(version: &str, (os, arch, libc, target): (&str, &str, &str, &str)) -> Value {
    json!({"key":format!("cpython-{version}-{os}-{arch}-{libc}"),"version":version,
        "url":format!("{ORIGIN}{RELEASES}{TAG}/{}", filename(version, target).replace('+', "%2B")),
        "os":os,"arch":arch,"libc":libc,"implementation":"cpython","variant":"default"})
}
fn listing_json() -> Value {
    let mut entries: Vec<_> = ["3.12.11", "3.12.12", "3.13.1"]
        .into_iter()
        .flat_map(|version| TARGETS.map(|target| entry(version, target)))
        .collect();
    let mut freethreaded = entry("3.14.0", TARGETS[0]);
    freethreaded["variant"] = json!("freethreaded");
    let mut pypy = entry("3.14.1", TARGETS[0]);
    pypy["implementation"] = json!("pypy");
    let prerelease = entry("3.15.0rc1", TARGETS[0]);
    let mut armv7 = entry("3.14.2", TARGETS[1]);
    armv7["arch"] = json!("armv7");
    entries.extend([freethreaded, pypy, prerelease, armv7]);
    entries.extend(UNSUPPORTED.map(|target| entry("3.12.12", target)));
    Value::Array(entries)
}
fn listing() -> Listing {
    Listing::parse(&serde_json::to_vec(&listing_json()).unwrap()).unwrap()
}
fn sums(version: &str) -> Vec<u8> {
    TARGETS
        .iter()
        .map(|target| format!("{:064x}  {}\n", target.3.len(), filename(version, target.3)))
        .collect::<String>()
        .into_bytes()
}

#[test]
fn listing_selects_the_highest_supported_cpython_release() {
    let listing = listing();
    for (request, expected) in [
        ("3.12", "3.12.12"),
        (">=3.12", "3.13.1"),
        ("3.12.11", "3.12.11"),
    ] {
        assert_eq!(
            listing.select(&requirement(request)).unwrap().to_string(),
            expected
        );
    }
    // Free-threaded, PyPy, prereleases and unsupported targets are ignored.
    assert!(matches!(
        listing.select(&requirement(">=3.14")),
        Err(Error::NoMatchingRelease {
            file: ".python-version",
            ..
        })
    ));
}

#[test]
fn inconsistent_listings_fail_closed() {
    let mutations: [fn(&mut Value); 4] = [
        |v| v[0]["key"] = json!("cpython-3.12.11-macos-aarch64-none/../escape"),
        |v| v[0]["url"] = json!("https://example.com/cpython.tar.gz"),
        |v| v[0]["url"] = json!(format!("{ORIGIN}{RELEASES}{TAG}/../cpython-3.12.11.tar.gz")),
        |v| {
            let duplicate = v[0].clone();
            v.as_array_mut().unwrap().push(duplicate);
        },
    ];
    for mutate in mutations {
        let mut value = listing_json();
        mutate(&mut value);
        assert!(matches!(
            Listing::parse(&serde_json::to_vec(&value).unwrap()),
            Err(Error::Listing)
        ));
    }
    let mut github = listing_json();
    github[0]["url"] = json!(format!(
        "{GITHUB}{TAG}/{}",
        filename("3.12.11", TARGETS[0].3).replace('+', "%2B")
    ));
    assert!(Listing::parse(&serde_json::to_vec(&github).unwrap()).is_ok());
    // Real uv listings include older full `.tar.zst` builds; only that
    // uninstallable target is skipped.
    let mut zstd = listing_json();
    zstd[0]["url"] = json!(format!(
        "{ORIGIN}{RELEASES}20211017/cpython-3.12.11-aarch64-apple-darwin-pgo%2Blto-20211017T1616.\
         tar.zst"
    ));
    let listing = Listing::parse(&serde_json::to_vec(&zstd).unwrap()).unwrap();
    let targets = &listing.0[&Version::new(3, 12, 11)];
    assert!(!targets.contains_key(&Platform::MacosArm64));
    assert!(targets.contains_key(&Platform::LinuxX64Gnu));
}

#[test]
fn host_filtered_listings_fail_closed() {
    // uv before 0.7.22 ignores `--all-arches` here: x86_64 hosts list only
    // x86_64 builds; arm64 Macs list arm64 builds plus emulated Windows x64.
    for (arch, emulated_os) in [("x86_64", None), ("aarch64", Some("windows"))] {
        let host: Vec<_> = TARGETS
            .into_iter()
            .filter(|target| target.1 == arch || Some(target.0) == emulated_os)
            .map(|target| entry("3.12.12", target))
            .collect();
        assert!(matches!(
            Listing::parse(&serde_json::to_vec(&host).unwrap()),
            Err(Error::HostOnlyListing)
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mirrors_serve_the_official_checksum_paths_beneath_their_base() {
    let path = format!("/mirror{RELEASES}{TAG}/SHA256SUMS");
    let upstream = LoopbackServer::new([(path.clone(), sums("3.12.12"))], |_| {}).unwrap();
    let fixture = crate::source_policy::test_support::OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror", upstream.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&base).unwrap();
    let transport = PythonTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let tool = transport.resolve(&requirement("3.12"), &listing()).await;
    assert_eq!(upstream.requests(), [path]);
    let Installation::Managed { artifacts } = &tool.unwrap().installation else {
        panic!("expected managed Python");
    };
    let official = format!("{ORIGIN}{RELEASES}{TAG}/");
    assert!(
        artifacts
            .values()
            .all(|parts| parts["distribution"].url.starts_with(&official))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn transport_binds_published_checksums_and_uv_keys() {
    let upstream = LoopbackServer::new(
        [(format!("{RELEASES}{TAG}/SHA256SUMS"), sums("3.12.12"))],
        |_| {},
    )
    .unwrap();
    let transport = PythonTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let tool = transport
        .resolve(&requirement("3.12"), &listing())
        .await
        .unwrap();
    assert_eq!(
        (tool.adapter.as_str(), tool.version.as_str()),
        ("python", "3.12.12")
    );
    assert_eq!(upstream.requests(), [format!("{RELEASES}{TAG}/SHA256SUMS")]);
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("expected managed Python");
    };
    // Exactly the lock's platforms, never another architecture or CPU variant.
    assert_eq!(
        artifacts.keys().copied().collect::<Vec<_>>(),
        super::TARGETS.map(|target| target.0)
    );
    for (platform, triple) in [
        (Platform::LinuxX64Gnu, "x86_64-unknown-linux-gnu"),
        (Platform::WindowsX64, "x86_64-pc-windows-msvc"),
        (Platform::LinuxArm64Musl, "aarch64-unknown-linux-musl"),
    ] {
        let url = &artifacts[&platform]["distribution"].url;
        assert!(url.ends_with(&format!("{TAG}-{triple}-install_only_stripped.tar.gz")));
    }
    let mac = &artifacts[&Platform::MacosArm64]["distribution"];
    assert_eq!(
        mac.url,
        format!(
            "{ORIGIN}{RELEASES}{TAG}/cpython-3.12.12%\
             2B{TAG}-aarch64-apple-darwin-install_only_stripped.tar.gz"
        )
    );
    assert_eq!(mac.sha256, format!("{:064x}", "aarch64-apple-darwin".len()));
    assert_eq!(
        mac.destination.as_deref(),
        Some("cpython-3.12.12-macos-aarch64-none")
    );
    assert_eq!(mac.executables["python3"], "bin/python3");
    let windows = &artifacts[&Platform::WindowsX64]["distribution"];
    assert_eq!(windows.executables.keys().collect::<Vec<_>>(), ["python"]);
    // A listed build without a published checksum is never trusted.
    assert!(matches!(
        transport.resolve(&requirement("3.13"), &listing()).await,
        Err(Error::MissingChecksum(name)) if name.starts_with("cpython-3.13.1+")
    ));
}

#[test]
fn listing_runs_the_managed_uv_offline_without_user_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let record = temp.path().join("record");
    let output = temp.path().join("listing.json");
    fs::write(&output, serde_json::to_vec(&listing_json()).unwrap()).unwrap();
    let uv = temp.path().join("uv");
    fs::write(
        &uv,
        format!(
            "#!/bin/sh\nprintf '%s|%s|%s' \"$*\" \"$UV_CACHE_DIR\" \"$UV_CONFIG_FILE\" > \
             '{}'\ncat '{}'\n",
            record.display(),
            output.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&uv, fs::Permissions::from_mode(0o755)).unwrap();
    let listing = list(&uv).unwrap();
    assert_eq!(
        listing.select(&requirement("3.13")).unwrap(),
        Version::new(3, 13, 1)
    );
    let recorded = fs::read_to_string(&record).unwrap();
    let parts: Vec<_> = recorded.split('|').collect();
    assert_eq!(
        parts[0],
        "python list --only-downloads --all-versions --all-platforms --all-arches --output-format \
         json --offline --no-config"
    );
    assert!(!parts[1].is_empty() && !Path::new(parts[1]).exists());
    assert_eq!(parts[2], "");
    fs::write(&uv, "#!/bin/sh\nexit 2\n").unwrap();
    assert!(matches!(list(&uv), Err(Error::ListingFailed)));
}
