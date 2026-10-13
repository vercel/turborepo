use serde_json::json;

use super::*;
use crate::{lock::Platform, source_policy::test_support::LoopbackServer};

fn requirement(request: &str) -> UvRequirement {
    let text = format!("[tool.uv]\nrequired-version = {request:?}\n");
    UvRequirement::read_with(|file, _| Ok((file == "pyproject.toml").then(|| text.clone().into())))
        .unwrap()
        .unwrap()
}
fn digest(target: &str) -> String {
    format!("{:064x}", target.len() * 7919)
}
fn index(versions: &[&str]) -> Vec<u8> {
    versions
        .iter()
        .map(|version| {
            let artifacts: Vec<_> = uv::PLATFORMS
                .iter()
                .map(|&platform| {
                    let target = UvArtifact::for_platform(&Version::new(0, 0, 0), platform)
                        .unwrap()
                        .target();
                    json!({"platform":target,"variant":"default","sha256":digest(target)})
                })
                .collect();
            json!({"version":version,"artifacts":artifacts}).to_string() + "\n"
        })
        .collect::<String>()
        .into_bytes()
}
fn sums(version: &str, platforms: &[Platform]) -> Vec<u8> {
    let version = Version::parse(version).unwrap();
    let mut text = format!("{} *source.tar.gz\n", "f".repeat(64));
    for &platform in platforms {
        let artifact = UvArtifact::for_platform(&version, platform).unwrap();
        text += &format!("{} *{}\n", digest(artifact.target()), artifact.filename());
    }
    text.into_bytes()
}
fn parsed(versions: &[&str]) -> ReleaseIndex {
    ReleaseIndex::parse(&index(versions)).unwrap()
}

#[test]
fn ranges_select_the_highest_release_and_bind_every_published_target() {
    let index = parsed(&["0.12.24", "0.13.0", "0.12.3", "0.9.0"]);
    for (request, expected) in [
        (">=0.12, <0.13", "0.12.24"),
        ("==0.12.*", "0.12.24"),
        ("~=0.12.3", "0.12.24"),
        ("0.12.3", "0.12.3"),
        (">=0.5", "0.13.0"),
    ] {
        assert_eq!(
            select(&requirement(request), &index).unwrap().to_string(),
            expected
        );
    }
    let version = Version::new(0, 13, 0);
    let tool = resolve(
        &requirement(">=0.5"),
        &index,
        &version,
        &sums("0.13.0", &uv::PLATFORMS),
    )
    .unwrap();
    assert_eq!(
        (tool.adapter.as_str(), tool.version.as_str()),
        ("uv", "0.13.0")
    );
    assert_eq!(tool.declarations[0].request.as_deref(), Some(">=0.5"));
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("expected managed uv");
    };
    assert_eq!(artifacts.len(), uv::PLATFORMS.len());
    let mac = &artifacts[&Platform::MacosArm64]["distribution"];
    assert_eq!(
        mac.url,
        "https://releases.astral.sh/github/uv/releases/download/0.13.0/uv-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(mac.sha256, digest("aarch64-apple-darwin"));
}

#[test]
fn unpublished_targets_are_omitted_and_inconsistent_metadata_fails() {
    let index = parsed(&["0.13.0"]);
    let version = Version::new(0, 13, 0);
    let request = requirement(">=0.5");
    let partial = sums("0.13.0", &[Platform::LinuxX64Gnu]);
    let tool = resolve(&request, &index, &version, &partial).unwrap();
    let Installation::Managed { artifacts } = tool.installation else {
        panic!("expected managed uv");
    };
    assert_eq!(
        artifacts.keys().collect::<Vec<_>>(),
        [&Platform::LinuxX64Gnu]
    );
    assert!(matches!(
        resolve(&request, &index, &version, &sums("0.13.0", &[])),
        Err(Error::MissingChecksums(_))
    ));
    let conflicting = String::from_utf8(partial)
        .unwrap()
        .replace(&digest("x86_64-unknown-linux-gnu"), &"0".repeat(64));
    assert!(matches!(
        resolve(&request, &index, &version, conflicting.as_bytes()),
        Err(Error::ChecksumConflict(_))
    ));
    assert!(
        resolve(
            &requirement("<0.13"),
            &index,
            &version,
            &sums("0.13.0", &[])
        )
        .is_err()
    );
    let error = select(&requirement(">=1.0"), &index).unwrap_err();
    assert_eq!(
        error.to_string(),
        "no published uv release satisfies pyproject.toml tool.uv.required-version = \">=1.0\""
    );
}

#[tokio::test(flavor = "current_thread")]
async fn transport_reads_only_the_index_and_selected_checksums() {
    let upstream = LoopbackServer::new(
        [
            (uv::INDEX_PATH.into(), index(&["0.12.24", "0.13.0"])),
            (
                "/github/uv/releases/download/0.12.24/sha256.sum".into(),
                sums("0.12.24", &uv::PLATFORMS),
            ),
        ],
        |_| {},
    )
    .unwrap();
    let transport = UvTransport::loopback_http_for_tests(upstream.origin()).unwrap();
    let tool = transport.resolve(&requirement("<0.13")).await.unwrap();
    assert_eq!(tool.version, "0.12.24");
    assert_eq!(
        upstream.requests(),
        [
            uv::INDEX_PATH,
            "/github/uv/releases/download/0.12.24/sha256.sum"
        ]
    );
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("expected managed uv");
    };
    // Lock provenance stays official; the fixture origin is transport only.
    assert!(
        artifacts
            .values()
            .all(|parts| parts["distribution"].url.starts_with(uv::ORIGIN))
    );
    // A release missing its checksum file fails instead of trusting the index.
    assert!(transport.resolve(&requirement(">=0.13")).await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn mirrors_serve_the_official_index_and_release_paths_beneath_their_base() {
    let paths = [
        format!("/mirror{}", uv::INDEX_PATH),
        "/mirror/github/uv/releases/download/0.13.0/sha256.sum".into(),
    ];
    let bodies = [index(&["0.13.0"]), sums("0.13.0", &uv::PLATFORMS)];
    let upstream = LoopbackServer::new(paths.clone().into_iter().zip(bodies), |_| {}).unwrap();
    let fixture = crate::test_support::OwnedSetupFixture::new().unwrap();
    let base = format!("{}/mirror", upstream.origin());
    let mirror = crate::mirror::Mirror::loopback_for_tests(&base).unwrap();
    let transport = UvTransport::mirrored(&fixture.policy().unwrap(), &mirror).unwrap();
    let tool = transport.resolve(&requirement(">=0.13")).await.unwrap();
    assert_eq!(upstream.requests(), paths);
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("expected managed uv");
    };
    assert!(
        artifacts
            .values()
            .all(|parts| parts["distribution"].url.starts_with(uv::ORIGIN))
    );
}
