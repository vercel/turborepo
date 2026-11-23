use turborepo_setup::{artifact_provision::ArtifactTransport, mirror::Mirror};
use turborepo_setup_catalog::{CatalogTransport, Registry};

use super::*;

#[path = "registry_tests.rs"]
mod registries;

const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];
const UPDATE: &[&str] = &["--tools-only", "--no-frozen", "--update-lock"];
const NAMES: &str = "fixture\texample/fixture\n";
const REGISTRY: &str = r#"
packages:
  - repo_owner: example
    repo_name: fixture
    asset: fixture-{{trimV .Version}}-{{.OS}}-{{.Arch}}.tar.gz
    files:
      - name: fixture
        src: fixture-{{trimV .Version}}/bin/fixture
"#;
const RELEASE: &str = "/example/fixture/releases/download/v1.2.3";

fn artifact() -> Vec<u8> {
    archive(
        "fixture-1.2.3",
        &[
            ("bin/fixture", "#!/bin/sh\necho fixture 1.2.3\n"),
            ("share/data.txt", "fixture resource"),
        ],
    )
}

fn asset() -> String {
    let (os, arch) = match provision::platform().unwrap() {
        turborepo_setup::lock::Platform::MacosArm64 => ("darwin", "arm64"),
        turborepo_setup::lock::Platform::MacosX64 => ("darwin", "amd64"),
        turborepo_setup::lock::Platform::LinuxX64Gnu => ("linux", "amd64"),
        _ => ("linux", "arm64"),
    };
    format!("fixture-1.2.3-{os}-{arch}.tar.gz")
}

fn declare(f: &Fixture, tools: Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": tools}});
    fs::write(f.owned.root().join("turbo.json"), config.to_string()).unwrap();
}

fn fixture() -> (Fixture, LoopbackServer) {
    let mut f = Fixture::new();
    f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
    f.declare(json!({}));
    declare(&f, json!({"fixture": "1.2.3"}));
    let routes = [
        (f.node_path.clone(), f.node.clone()),
        (format!("{RELEASE}/{}", asset()), artifact()),
    ];
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    (f, u)
}

fn run(f: &Fixture, u: &LoopbackServer, flags: &[&str]) -> Result<i32, Error> {
    run_with(f, u, flags, |_| {})
}

fn run_with(
    f: &Fixture,
    u: &LoopbackServer,
    flags: &[&str],
    configure: impl Fn(&mut provision::Transports),
) -> Result<i32, Error> {
    let args = f.args(flags);
    crate::cli::dispatch_setup(&args, |args, setup_args| {
        let mut transports = f.transports(u.origin());
        transports.catalog = Some(catalog::Fixture {
            aqua: Registry::aqua_for_tests(NAMES, REGISTRY),
            transport: CatalogTransport::loopback_http_for_tests(u.origin()).unwrap(),
        });
        configure(&mut transports);
        run_with_policy(args, setup_args, Some(transports), f.git.clone(), || {
            Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
        })
    })
    .unwrap()
}

fn lock(f: &Fixture) -> Vec<u8> {
    fs::read(f.owned.root().join("turbo.lock")).unwrap()
}

fn catalog_requests(u: &LoopbackServer) -> usize {
    let requests = u.requests();
    let catalog = requests.iter().filter(|path| path.contains("/releases/"));
    catalog.count()
}

#[test]
fn local_setup_resolves_locks_installs_and_then_reuses_the_lock() {
    let (f, u) = fixture();
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);

    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();
    let locked: Value = serde_json::from_slice(&lock(&f)).unwrap();
    let tool = &locked["tools"]["fixture"];
    assert_eq!(tool["adapter"], "artifact");
    assert_eq!(tool["version"], "1.2.3");
    assert_eq!(tool["options"], json!({"source": ["aqua:example/fixture"]}));
    assert_eq!(
        tool["declarations"],
        json!([{"file": "turbo.json", "field": "/setup/additionalTools/fixture",
            "request": "1.2.3"}])
    );
    assert_eq!(
        tool["installation"]["artifacts"],
        json!({host.as_str().unwrap(): {"artifact": {
            "url": format!("https://github.com{RELEASE}/{}", asset()),
            "sha256": format!("{:x}", Sha256::digest(artifact())),
            "format": "tar-gz",
            "executables": {"fixture": "fixture-1.2.3/bin/fixture"}}}})
    );

    let store = Installed::open(&f.owned);
    let current = store.current().unwrap().unwrap();
    let ids: Vec<_> = current.tools.iter().map(|tool| tool.id.as_str()).collect();
    assert_eq!(ids, ["fixture", "node"]);
    let output = std::process::Command::new(current.bin.join("fixture"))
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"fixture 1.2.3\n");
    let resource = current.tree("fixture").join("fixture-1.2.3/share/data.txt");
    assert_eq!(fs::read(resource).unwrap(), b"fixture resource");
    drop(store);

    let (bytes, hits, manifest) = (lock(&f), u.hits(), f.manifest().unwrap());
    for flags in [LOCAL, FROZEN, &["--plan", "--tools-only"]] {
        assert_eq!(run(&f, &u, flags).unwrap(), 0, "{flags:?}");
        assert_eq!(u.hits(), hits, "{flags:?}");
        assert_eq!(lock(&f), bytes, "{flags:?}");
    }
    assert_eq!(f.manifest().unwrap(), manifest);
    f.no_execution();
}

#[test]
fn frozen_and_forced_setup_name_a_changed_declaration_without_resolving() {
    let (f, u) = fixture();
    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    let (bytes, hits, manifest) = (lock(&f), u.hits(), f.manifest().unwrap());

    for changed in [
        json!({"fixture": "1.2.4"}),
        json!({"fixture": {"version": "1.2.3", "from": "aqua:example/other"}}),
        json!({"fixture": "1.2.3", "added": "1.0.0"}),
    ] {
        declare(&f, changed.clone());
        let id = if changed.get("added").is_some() {
            "added"
        } else {
            "fixture"
        };
        for flags in [FROZEN, &["--tools-only", "--no-frozen", "--force"]] {
            let error = run(&f, &u, flags).unwrap_err();
            assert!(
                matches!(&error, Error::AdditionalToolDrift(ids)
                    if ids.iter().map(String::as_str).collect::<Vec<_>>() == [id]),
                "{changed} {flags:?}: {error}"
            );
            assert!(error.to_string().contains(&format!("{id:?}")), "{error}");
        }
        assert_eq!(u.hits(), hits, "{changed}");
        assert_eq!(lock(&f), bytes, "{changed}");
    }
    assert_eq!(f.manifest().unwrap(), manifest);
    f.no_execution();
}

#[test]
fn offline_setup_refuses_an_unresolved_declaration() {
    let (f, u) = fixture();
    let bytes = lock(&f);
    for flags in [
        &["--tools-only", "--no-frozen", "--offline"][..],
        &["--plan", "--tools-only", "--no-frozen", "--offline"],
    ] {
        let error = run(&f, &u, flags).unwrap_err();
        assert!(
            matches!(&error, Error::CatalogOffline(ids) if ids.contains("fixture")),
            "{flags:?}: {error}"
        );
        let message = error.to_string();
        assert!(message.contains("\"fixture\""), "{message}");
        assert!(message.contains("without --offline"), "{message}");
    }
    assert_eq!(u.hits(), 0);
    assert_eq!(lock(&f), bytes);
    assert_eq!(f.manifest(), None);
}

#[test]
fn a_plan_resolves_a_proposal_and_writes_nothing() {
    let (f, u) = fixture();
    let bytes = lock(&f);
    assert_eq!(
        run(&f, &u, &["--plan", "--tools-only", "--no-frozen"]).unwrap(),
        0
    );
    let requests = u.requests();
    assert!(requests.contains(&format!("{RELEASE}/{}", asset())));
    assert!(
        requests.iter().all(|path| path.starts_with(RELEASE)),
        "{requests:?}"
    );
    assert_eq!(lock(&f), bytes);
    assert_eq!(f.manifest(), None);
    assert!(Installed::open(&f.owned).current().unwrap().is_none());
    f.no_execution();

    assert_eq!(run(&f, &u, LOCAL).unwrap(), 0);
    let locked: Value = serde_json::from_slice(&lock(&f)).unwrap();
    assert_eq!(
        locked["tools"]["fixture"]["options"]["source"],
        json!(["aqua:example/fixture"])
    );
}

#[test]
fn unresolvable_declarations_fail_before_anything_changes() {
    let (f, u) = fixture();
    let bytes = lock(&f);

    declare(&f, json!({"unknown": "1.0.0"}));
    let error = run(&f, &u, LOCAL).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("\"unknown\" is not a tool name") && message.contains("\"artifacts\""),
        "{message}"
    );
    assert_eq!(u.hits(), 0);

    declare(&f, json!({"fixture": "9.9.9"}));
    let error = run(&f, &u, LOCAL).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("cannot find fixture 9.9.9") && message.contains("v9.9.9, 9.9.9"),
        "{message}"
    );

    declare(&f, json!({"fixture": "1.2.3"}));
    let other = if asset().contains("linux") {
        "fixture-1.2.3-darwin-arm64.tar.gz"
    } else {
        "fixture-1.2.3-linux-amd64.tar.gz"
    };
    let elsewhere =
        LoopbackServer::new([(format!("{RELEASE}/{other}"), artifact())], |_| {}).unwrap();
    let error = run(&f, &elsewhere, LOCAL).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("fixture 1.2.3 cannot be installed on this machine"),
        "{message}"
    );

    assert_eq!(lock(&f), bytes);
    assert_eq!(f.manifest(), None);
    f.no_execution();
}

#[test]
fn no_lock_setup_never_resolves() {
    let (f, u) = fixture();
    let bytes = lock(&f);
    for locked in [true, false] {
        if !locked {
            fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
        }
        let error = run(&f, &u, &["--no-lock", "--tools-only"]).unwrap_err();
        assert!(error.to_string().contains("\"fixture\""), "{error}");
        assert_eq!(catalog_requests(&u), 0, "{:?}", u.requests());
        if locked {
            assert_eq!(lock(&f), bytes);
        }
    }
    assert!(!f.owned.root().join("turbo.lock").exists());
    assert_eq!(f.manifest(), None);
    f.no_execution();
}

#[test]
fn a_failed_precondition_stops_setup_before_any_catalog_request() {
    let (f, u) = fixture();
    fs::remove_file(f.owned.root().join(".nvmrc")).unwrap();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    for flags in [LOCAL, &["--plan", "--tools-only", "--no-frozen"]] {
        let error = run(&f, &u, flags).unwrap_err();
        assert!(
            matches!(error, Error::Unsupported(reason) if reason.contains("Node declaration")),
            "{flags:?}"
        );
        assert_eq!(u.hits(), 0, "{flags:?}");
    }
    assert!(!f.owned.root().join("turbo.lock").exists());
    assert_eq!(f.manifest(), None);
}

#[test]
fn update_lock_resolves_an_incomplete_entry_again_and_leaves_a_complete_one_alone() {
    let (f, _) = fixture();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let windows = archive("fixture-1.2.3", &[("bin/fixture.exe", "MZ")]);
    let publish = |assets: &[&str]| {
        let assets = assets.iter().map(|target| {
            let bytes = if target.starts_with("windows") {
                windows.clone()
            } else {
                artifact()
            };
            (format!("{RELEASE}/fixture-1.2.3-{target}.tar.gz"), bytes)
        });
        let routes = super::first_lock::routes(&f, "24.0.0", "10.0.0");
        LoopbackServer::new(routes.into_iter().chain(assets), |_| {}).unwrap()
    };
    let locked = |f: &Fixture| -> Value {
        let locked: Value = serde_json::from_slice(&lock(f)).unwrap();
        locked["tools"]["fixture"]["installation"]["artifacts"].clone()
    };
    let platforms = |f: &Fixture| -> std::collections::BTreeSet<String> {
        locked(f).as_object().unwrap().keys().cloned().collect()
    };
    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();

    let host_asset = asset();
    let host_target = host_asset
        .trim_start_matches("fixture-1.2.3-")
        .trim_end_matches(".tar.gz");
    let first = publish(&[host_target]);
    assert_eq!(run(&f, &first, LOCAL).unwrap(), 0);
    assert_eq!(platforms(&f), [host.as_str().unwrap().to_owned()].into());

    let targets = [
        "darwin-amd64",
        "darwin-arm64",
        "linux-amd64",
        "linux-arm64",
        "windows-amd64",
    ];
    let later = publish(&targets);
    let bytes = lock(&f);
    assert_eq!(run(&f, &later, LOCAL).unwrap(), 0);
    assert_eq!(catalog_requests(&later), 0);
    assert_eq!(lock(&f), bytes);

    assert_eq!(run(&f, &later, UPDATE).unwrap(), 0);
    let recorded = platforms(&f);
    let recorded: Vec<_> = recorded.iter().map(String::as_str).collect();
    assert_eq!(
        recorded,
        [
            "linux-arm64-gnu",
            "linux-x64-gnu",
            "macos-arm64",
            "macos-x64",
            "windows-x64"
        ]
    );
    assert_eq!(
        locked(&f)["windows-x64"]["artifact"]["sha256"],
        format!("{:x}", Sha256::digest(&windows))
    );

    let (bytes, asked) = (lock(&f), catalog_requests(&later));
    assert_eq!(run(&f, &later, UPDATE).unwrap(), 0);
    assert_eq!(catalog_requests(&later), asked);
    assert_eq!(lock(&f), bytes);
    f.no_execution();
}

#[test]
fn an_artifact_mirror_serves_the_install_and_never_resolution() {
    let (f, origin) = fixture();
    let mirrored = format!("/mirror/github.com{RELEASE}/{}", asset());
    let mirror = LoopbackServer::new([(mirrored.clone(), artifact())], |_| {}).unwrap();
    let run = |flags: &[&str]| {
        run_with(&f, &origin, flags, |transports| {
            let policy = f.owned.policy_at(&f.owned.root().join("apps/web/src"));
            let base = format!("{}/mirror", mirror.origin());
            let base = Mirror::loopback_for_tests(&base).unwrap();
            let transport = ArtifactTransport::mirrored(&policy.unwrap(), &base);
            transports.artifact = Some(transport.unwrap());
        })
    };

    assert_eq!(run(&["--plan", "--tools-only", "--no-frozen"]).unwrap(), 0);
    assert!(catalog_requests(&origin) > 0);
    assert_eq!(mirror.hits(), 0);

    assert_eq!(run(LOCAL).unwrap(), 0);
    let requests = mirror.requests();
    assert!(
        !requests.is_empty() && requests.iter().all(|path| path == &mirrored),
        "{requests:?}"
    );
    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();
    let locked: Value = serde_json::from_slice(&lock(&f)).unwrap();
    let pinned = &locked["tools"]["fixture"]["installation"]["artifacts"][host.as_str().unwrap()];
    assert_eq!(
        pinned["artifact"]["url"],
        format!("https://github.com{RELEASE}/{}", asset())
    );
    assert_eq!(
        pinned["artifact"]["sha256"],
        format!("{:x}", Sha256::digest(artifact()))
    );
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    assert!(current.tools.iter().any(|tool| tool.id == "fixture"));
    f.no_execution();
}
