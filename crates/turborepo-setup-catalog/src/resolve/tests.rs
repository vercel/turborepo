use std::io::Write;

use sha2::{Digest, Sha256};
use turborepo_setup::{
    lock::{Format, Installation, additional},
    source_policy::test_support::LoopbackServer,
};

use super::*;
use crate::{Catalog, Registry, plan::tests::REGISTRY, tests::request};

fn catalog() -> Catalog<'static> {
    Catalog::new(vec![Registry::aqua_for_tests(
        "raw\texample/raw\n",
        REGISTRY,
    )])
}

const HOST: Platform = Platform::LinuxX64Gnu;

fn tar_gz(entries: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, contents, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        archive
            .append_data(&mut header, path, contents.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&archive.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn serve(routes: &[(&str, Vec<u8>)]) -> LoopbackServer {
    let routes = routes
        .iter()
        .map(|(path, body)| ((*path).to_owned(), body.clone()));
    LoopbackServer::new(routes, |_| {}).unwrap()
}

async fn run(
    server: &LoopbackServer,
    id: &str,
    version: &str,
    from: Option<&str>,
    host: Platform,
) -> Result<Resolved, Error> {
    let request = request(id, version, from);
    let plan = catalog().plan(&request)?;
    let transport = CatalogTransport::loopback_http_for_tests(server.origin())?;
    resolve(&request, &plan, host, &transport).await
}

fn artifacts(tool: &Tool) -> BTreeMap<Platform, &Artifact> {
    let Installation::Managed { artifacts } = &tool.installation else {
        panic!("catalog tools are managed");
    };
    artifacts
        .iter()
        .map(|(platform, parts)| {
            assert_eq!(parts.keys().collect::<Vec<_>>(), [additional::PART]);
            (*platform, &parts[additional::PART])
        })
        .collect()
}

#[tokio::test]
async fn the_release_name_that_exists_is_found_and_its_bytes_are_pinned() {
    let release = "/example/raw/releases/download/26.1";
    let binary = |target: &str| format!("#!/bin/sh\necho raw for {target}\n").into_bytes();
    let server = serve(&[
        (&format!("{release}/raw-darwin-amd64"), binary("macos-x64")),
        (
            &format!("{release}/raw-darwin-arm64"),
            binary("macos-arm64"),
        ),
        (&format!("{release}/raw-linux-amd64"), binary("linux-x64")),
        (
            &format!("{release}/raw-windows-amd64.exe"),
            b"MZ raw for windows".to_vec(),
        ),
    ]);
    let resolved = run(&server, "raw", "26.1", None, HOST).await.unwrap();
    assert_eq!(resolved.release, "26.1");
    assert_eq!(resolved.source, "aqua:example/raw");
    assert_eq!(
        resolved.skipped,
        BTreeMap::from([(Platform::LinuxArm64Gnu, Unavailable::Missing)])
    );

    let tool = &resolved.tool;
    assert_eq!(tool.adapter, additional::ADAPTER);
    assert_eq!(tool.version, "26.1");
    assert_eq!(
        tool.options,
        BTreeMap::from([("source".to_owned(), vec!["aqua:example/raw".to_owned()])])
    );
    let artifacts = artifacts(tool);
    assert_eq!(
        artifacts.keys().copied().collect::<Vec<_>>(),
        [
            Platform::MacosX64,
            Platform::MacosArm64,
            Platform::LinuxX64Gnu,
            Platform::WindowsX64
        ]
    );
    let linux = artifacts[&Platform::LinuxX64Gnu];
    assert_eq!(
        linux.url,
        "https://github.com/example/raw/releases/download/26.1/raw-linux-amd64"
    );
    assert_eq!(linux.sha256, Some(sha256(&binary("linux-x64"))));
    assert_eq!(linux.format, Format::Binary);
    assert_eq!(linux.root_prefix, None);
    assert_eq!(
        artifacts[&Platform::WindowsX64].sha256,
        Some(sha256(b"MZ raw for windows"))
    );
    assert_eq!(
        artifacts[&Platform::WindowsX64].executables["raw"],
        "raw.exe"
    );

    let lock = lock::Lock::new(lock::Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("raw".to_owned(), tool.clone())]),
    });
    assert!(lock.is_ok(), "{lock:?}");

    let requests = server.requests();
    assert!(requests[0].contains("/v26.1/"), "{requests:?}");
}

#[tokio::test]
async fn a_host_without_a_download_fails_with_the_reason() {
    let release = "/example/raw/releases/download/v26.1";
    let script = b"#!/bin/sh\n".to_vec();
    let server = serve(&[(&format!("{release}/raw-darwin-arm64"), script)]);
    let error = run(&server, "raw", "26.1", None, HOST).await.unwrap_err();
    assert!(
        matches!(&error, Error::Host { tool, platform, reason, .. }
            if tool == "raw" && platform == "linux-x64-gnu" && reason == &Unavailable::Missing),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("raw 26.1 cannot be installed on this machine (linux-x64-gnu)"),
        "{message}"
    );
    let musl = run(&server, "raw", "26.1", None, Platform::LinuxX64Musl).await;
    assert!(matches!(musl, Err(Error::Host { .. })), "{musl:?}");
    assert!(
        run(&server, "raw", "26.1", None, Platform::MacosArm64)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_version_without_a_release_names_what_was_tried() {
    let server = serve(&[]);
    let error = run(&server, "raw", "26.1", None, HOST).await.unwrap_err();
    assert!(
        matches!(&error, Error::NoRelease { tried, .. } if tried == &["v26.1", "26.1"]),
        "{error:?}"
    );
    let message = error.to_string();
    for part in ["raw 26.1", "v26.1, 26.1", "aqua:example/raw", "`artifacts`"] {
        assert!(message.contains(part), "{part}: {message}");
    }

    let server = serve(&[]);
    let from = Some("aqua:example/http-tool");
    let error = run(&server, "http-tool", "2.0.0", from, HOST).await;
    assert!(matches!(error, Err(Error::NoRelease { .. })), "{error:?}");
    let requests: BTreeSet<_> = server.requests().into_iter().collect();
    assert_eq!(
        requests.iter().collect::<Vec<_>>(),
        [
            "/http-tool/2.0.0/http-tool_darwin_amd64.zip",
            "/http-tool/2.0.0/http-tool_darwin_arm64.zip",
            "/http-tool/2.0.0/http-tool_linux_amd64.zip"
        ]
    );
}

#[tokio::test]
async fn archives_are_unpacked_once_and_unusable_ones_are_left_out() {
    let release = "/example/prefixed/releases/download/tool-1.2.3";
    let archive = |root: &str, mode: u32, helper: bool| {
        let (bin, helper_path) = (
            format!("{root}/bin/prefixed"),
            format!("{root}/libexec/helper"),
        );
        let mut entries = vec![(bin.as_str(), "#!/bin/sh\necho prefixed\n", mode)];
        if helper {
            entries.push((helper_path.as_str(), "#!/bin/sh\necho helper\n", 0o755));
        }
        tar_gz(&entries)
    };
    let linux = archive("prefixed-1.2.3-linux-x86_64", 0o755, true);
    let server = serve(&[
        (
            &format!("{release}/prefixed-1.2.3-linux-x86_64.tar.gz"),
            linux.clone(),
        ),
        (
            &format!("{release}/prefixed-1.2.3-macos-arm64.tar.gz"),
            archive("prefixed-1.2.3-macos-arm64", 0o755, false),
        ),
        (
            &format!("{release}/prefixed-1.2.3-linux-arm64.tar.gz"),
            archive("prefixed-1.2.3-linux-arm64", 0o644, true),
        ),
        (
            &format!("{release}/prefixed-1.2.3-macos-x86_64.tar.gz"),
            b"not an archive".to_vec(),
        ),
        (
            &format!("{release}/prefixed-1.2.3-windows-x86_64.zip"),
            turborepo_setup::source_policy::test_support::stored_zip_entries(&[
                ("prefixed-1.2.3-windows-x86_64/bin/prefixed.exe", b"MZ"),
                ("prefixed-1.2.3-windows-x86_64/libexec/helper.exe", b"MZ"),
            ]),
        ),
    ]);
    let from = Some("aqua:example/prefixed");
    let resolved = run(&server, "prefixed", "1.2.3", from, HOST).await.unwrap();
    assert_eq!(resolved.release, "tool-1.2.3");
    let artifacts = artifacts(&resolved.tool);
    let locked = artifacts[&HOST];
    assert_eq!(locked.sha256, Some(sha256(&linux)));
    assert_eq!(locked.format, Format::TarGz);
    assert_eq!(locked.root_prefix, None);
    assert_eq!(
        locked.executables["prefixed"],
        "prefixed-1.2.3-linux-x86_64/bin/prefixed"
    );
    assert!(matches!(
        resolved.skipped[&Platform::MacosArm64],
        Unavailable::Archive(_)
    ));
    assert!(matches!(
        resolved.skipped[&Platform::MacosX64],
        Unavailable::Archive(_)
    ));
    let windows = artifacts[&Platform::WindowsX64];
    assert_eq!(windows.format, Format::Zip);
    assert_eq!(
        windows.executables["prefixed-helper"],
        "prefixed-1.2.3-windows-x86_64/libexec/helper.exe"
    );
    if cfg!(unix) {
        assert_eq!(
            artifacts.keys().copied().collect::<Vec<_>>(),
            [HOST, Platform::WindowsX64]
        );
        assert_eq!(
            resolved.skipped[&Platform::LinuxArm64Gnu],
            Unavailable::NotExecutable("prefixed-1.2.3-linux-arm64/bin/prefixed".into())
        );
    }
    let requests = server.requests();
    assert!(
        requests.iter().all(|path| path.contains("/tool-1.2.3/")),
        "{requests:?}"
    );
}

#[tokio::test]
async fn a_binary_must_begin_as_a_program_for_its_platform() {
    let release = "/example/raw/releases/download/v26.1";
    let (elf, pe, script) = (b"\x7fELF\x02", b"MZ\x90", b"#!/bin/sh\n");
    let (mach_o, universal) = (b"\xcf\xfa\xed\xfe\x0c", b"\xca\xfe\xba\xbe\x00");
    let page = b"<!DOCTYPE html><html><body>Not found</body></html>";
    let publish = |bodies: [&[u8]; 5]| {
        let assets = [
            "raw-darwin-amd64",
            "raw-darwin-arm64",
            "raw-linux-amd64",
            "raw-linux-arm64",
            "raw-windows-amd64.exe",
        ];
        let routes = assets.iter().zip(bodies);
        let routes = routes.map(|(asset, body)| (format!("{release}/{asset}"), body.to_vec()));
        LoopbackServer::new(routes, |_| {}).unwrap()
    };

    let server = publish([mach_o, universal, elf, script, pe]);
    let resolved = run(&server, "raw", "26.1", None, HOST).await.unwrap();
    assert_eq!(artifacts(&resolved.tool).len(), 5);
    assert_eq!(resolved.skipped, BTreeMap::new());

    let server = publish([elf, page, elf, pe, script]);
    let resolved = run(&server, "raw", "26.1", None, HOST).await.unwrap();
    assert_eq!(
        artifacts(&resolved.tool).keys().collect::<Vec<_>>(),
        [&HOST]
    );
    for platform in [
        Platform::MacosX64,
        Platform::MacosArm64,
        Platform::LinuxArm64Gnu,
        Platform::WindowsX64,
    ] {
        assert_eq!(resolved.skipped[&platform], Unavailable::NotAProgram);
    }
    let reason = Unavailable::NotAProgram.to_string();
    assert!(reason.contains("not an executable for this platform"));

    let server = publish([page; 5]);
    let error = run(&server, "raw", "26.1", None, HOST).await.unwrap_err();
    assert!(
        matches!(&error, Error::NoPlatform { platforms, .. }
            if platforms.values().all(|reason| reason == &Unavailable::NotAProgram)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_release_name_that_serves_nothing_installable_does_not_stop_the_next_one() {
    let page = b"<!DOCTYPE html><html><body>Not found</body></html>";
    let (mach_o, elf, pe): (&[u8], &[u8], &[u8]) =
        (b"\xcf\xfa\xed\xfe\x0c", b"\x7fELF\x02", b"MZ\x90");
    let assets = [
        ("raw-darwin-amd64", mach_o),
        ("raw-darwin-arm64", mach_o),
        ("raw-linux-amd64", elf),
        ("raw-linux-arm64", elf),
        ("raw-windows-amd64.exe", pe),
    ];
    let release =
        |name: &str, asset: &str| format!("/example/raw/releases/download/{name}/{asset}");
    let pages = assets.map(|(asset, _)| (release("v26.1", asset), page.to_vec()));
    let programs = assets.map(|(asset, program)| (release("26.1", asset), program.to_vec()));
    let server = LoopbackServer::new(pages.into_iter().chain(programs), |_| {}).unwrap();

    let resolved = run(&server, "raw", "26.1", None, HOST).await.unwrap();
    assert_eq!(resolved.release, "26.1");
    assert_eq!(artifacts(&resolved.tool).len(), 5);
    assert_eq!(resolved.skipped, BTreeMap::new());
}

#[tokio::test]
async fn one_download_serves_every_platform_that_shares_it() {
    let bytes = tar_gz(&[("raw", "#!/bin/sh\necho raw\n", 0o755)]);
    let server = serve(&[(
        "/other/raw/releases/download/v1.0.0/raw.tar.gz",
        bytes.clone(),
    )]);
    let from = Some("aqua:other/raw");
    let resolved = run(&server, "raw", "1.0.0", from, HOST).await.unwrap();
    let artifacts = artifacts(&resolved.tool);
    assert_eq!(artifacts.len(), 4);
    for artifact in artifacts.values() {
        assert_eq!(artifact.sha256, Some(sha256(&bytes)));
    }
    assert!(matches!(
        resolved.skipped[&Platform::WindowsX64],
        Unavailable::Archive(_)
    ));
}

#[tokio::test]
async fn recipes_without_a_usable_download_fail_without_a_request() {
    let server = serve(&[]);
    let from = Some("aqua:example/from-source");
    let error = run(&server, "from-source", "1.0.0", from, HOST).await;
    let error = error.unwrap_err();
    assert!(
        matches!(&error, Error::NoPlatform { platforms, .. } if platforms.len() == 5),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(message.contains("linux-x64-gnu: the catalog installs it with go_install"));
    assert_eq!(server.hits(), 0);

    let release = "/example/xz/releases/download/v5.8.1";
    let server = serve(&[(
        &format!("{release}/xz-v5.8.1-windows-amd64.zip"),
        b"zip".to_vec(),
    )]);
    let error = run(&server, "xz", "5.8.1", Some("aqua:example/xz"), HOST).await;
    assert!(matches!(error, Err(Error::NoPlatform { .. })), "{error:?}");
}

#[tokio::test]
async fn only_a_missing_download_is_skipped_and_an_outage_stops_resolution() {
    let request = request("raw", "26.1", None);
    let plan = catalog().plan(&request).unwrap();
    let transport = CatalogTransport::loopback_http_for_tests("http://127.0.0.1:9").unwrap();
    let error = resolve(&request, &plan, HOST, &transport)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, Error::Download { tool, .. } if tool == "raw"),
        "{error:?}"
    );
}
