#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeMap, fs};

use serde_json::{Value, json};

use super::*;

fn fixture() -> Value {
    let part = |name: &str, digest: &str| {
        json!({
            "url":"https://downloads.example.test/tool.tgz", "sha256":digest.repeat(64),
            "format":"tar-gz", "rootPrefix":"node-v24", "destination":null, "executables":{name:format!("bin/{name}")}
        })
    };
    json!({"schemaVersion":0,"tools":{
        "javascript":{"adapter":"node","version":"24.0.0","options":{"conditions":["production"]},
            "declarations":[{"file":"package.json","field":"engines.node","request":"24.x"}],
            "installation":{"kind":"managed","artifacts":{
                "linux-x64-gnu":{"runtime":part("node","a"), "headers":{
                    "url":"https://downloads.example.test/headers.tgz", "sha256":"d".repeat(64),
                    "format":"tar-gz", "rootPrefix":"node-headers", "destination":"include", "executables":{}}},
                "windows-x64":{"runtime":part("node","c")}}}},
        "npm":{"adapter":"npm","version":"11.0.0",
            "declarations":[{"file":"package.json","field":"packageManager","request":"npm@11.0.0"}],
            "installation":{"kind":"managed","artifacts":{"any":{"distribution":{
                "url":"https://downloads.example.test/npm.tgz", "sha256":"b".repeat(64),
                "format":"tar-gz", "rootPrefix":"package", "destination":"npm", "executables":{"npm":"bin/npm-cli.js"}}}}}},
        "pnpm":{"adapter":"pnpm","version":"10.9.0",
            "declarations":[{"file":"turbo.json","field":"/setup","request":"10.9.0"}],
            "installation":{"kind":"verify-system","executables":["pnpm"]}}
    }})
}

fn context() -> ExecutionContext {
    execution(
        OperatingSystem::Linux,
        Architecture::X64,
        Libc::Gnu {
            abi: "glibc-2.31".into(),
        },
        "x86_64-unknown-linux-gnu",
        &["x86-64-v2"],
    )
}

fn execution(
    os: OperatingSystem,
    arch: Architecture,
    libc: Libc,
    target: &str,
    cpu: &[&str],
) -> ExecutionContext {
    ExecutionContext::new(
        HostPlatform::new(os, arch),
        libc,
        target.into(),
        cpu.iter().map(|s| s.to_string()).collect(),
    )
    .unwrap()
}

fn lock(value: &Value) -> Lock {
    Lock::parse(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn snapshot(value: &Value, context: ExecutionContext) -> ExecutionSnapshot {
    ExecutionSnapshot::select(&lock(value), context, &ids(&["javascript", "npm", "pnpm"])).unwrap()
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| s.to_string()).collect()
}

#[test]
fn canonical_identity_golden() {
    let snapshot = snapshot(&fixture(), context());
    let actual: Value = serde_json::from_slice(snapshot.canonical_identity()).unwrap();
    let expected: Value = serde_json::from_slice(include_bytes!("golden.json")).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(snapshot.fingerprint(), include_str!("golden.sha256").trim());
    assert_eq!(actual["domain"], IDENTITY_DOMAIN);
}

#[test]
fn relocated_clones_mirrors_provenance_and_bookkeeping_are_stable() {
    let original = fixture();
    let mut mirrored = original.clone();
    mirrored["tools"]["javascript"]["declarations"] = json!([{"file":".nvmrc","request":"lts/*"}]);
    for tool in mirrored["tools"].as_object_mut().unwrap().values_mut() {
        if let Some(platforms) = tool["installation"].get_mut("artifacts") {
            for parts in platforms.as_object_mut().unwrap().values_mut() {
                for part in parts.as_object_mut().unwrap().values_mut() {
                    part["url"] = json!("https://mirror.example.test/relocated/payload");
                }
            }
        }
    }
    let mut identities = Vec::new();
    for (value, timestamp) in [(original, 1), (mirrored, 9999)] {
        let clone = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        fs::write(
            clone.path().join("turbo.lock"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        fs::write(
            cache.path().join("inventory.json"),
            json!({"installedAt":timestamp,
            "installationRoot":clone.path(), "cacheRoot":cache.path()})
            .to_string(),
        )
        .unwrap();
        let parsed = Lock::parse(&fs::read(clone.path().join("turbo.lock")).unwrap()).unwrap();
        let selected =
            ExecutionSnapshot::select(&parsed, context(), &ids(&["javascript", "npm", "pnpm"]))
                .unwrap();
        assert!(
            !String::from_utf8_lossy(selected.canonical_identity())
                .contains(clone.path().to_str().unwrap())
        );
        identities.push(selected.fingerprint().to_owned());
    }
    assert_eq!(identities[0], identities[1]);
}

#[test]
fn inactive_platforms_and_unselected_open_ids_do_not_change_identity() {
    let original = fixture();
    let mut changed = original.clone();
    changed["tools"]["javascript"]["installation"]["artifacts"]["windows-x64"]["runtime"]
        ["sha256"] = json!("f".repeat(64));
    changed["tools"]["future"] = json!({"adapter":"future-adapter","version":"release@1",
        "declarations":[{"file":"turbo.json"}],"installation":{"kind":"verify-system","executables":["future"]}});
    assert_eq!(
        snapshot(&original, context()).fingerprint(),
        snapshot(&changed, context()).fingerprint()
    );
    assert!(matches!(
        ExecutionSnapshot::select(&lock(&changed), context(), &ids(&["future"])),
        Err(Error::UnsupportedAdapter)
    ));
}

#[test]
fn every_active_semantic_field_invalidates_the_golden() {
    let original = fixture();
    let fingerprint = snapshot(&original, context()).fingerprint().to_owned();
    let node = "/tools/javascript/installation/artifacts/linux-x64-gnu";
    for (pointer, value) in [
        ("/tools/javascript/version".to_string(), json!("24.0.1")),
        (
            "/tools/javascript/options".to_string(),
            json!({"conditions":["development"]}),
        ),
        (format!("{node}/runtime/sha256"), json!("e".repeat(64))),
        (format!("{node}/headers/sha256"), json!("e".repeat(64))),
        (format!("{node}/runtime/format"), json!("zip")),
        (format!("{node}/runtime/rootPrefix"), json!("another-root")),
        (format!("{node}/runtime/destination"), json!("runtime")),
        (
            format!("{node}/runtime/executables/node"),
            json!("other/node"),
        ),
        (
            format!("{node}/runtime/executables"),
            json!({"nodejs":"bin/node"}),
        ),
        ("/tools/npm/version".to_string(), json!("11.0.1")),
        ("/tools/pnpm/version".to_string(), json!("10.9.1")),
        (
            "/tools/pnpm/installation/executables".to_string(),
            json!(["pnpm", "pnpx"]),
        ),
    ] {
        let mut changed = original.clone();
        *changed
            .pointer_mut(&pointer)
            .unwrap_or_else(|| panic!("missing {pointer}")) = value;
        assert_ne!(
            snapshot(&changed, context()).fingerprint(),
            fingerprint,
            "ignored {pointer}"
        );
    }
    let mut renamed = original.clone();
    let part = renamed["tools"]["javascript"]["installation"]["artifacts"]["linux-x64-gnu"]
        .as_object_mut()
        .unwrap();
    let headers = part.remove("headers").unwrap();
    part.insert("sdk".into(), headers);
    assert_ne!(snapshot(&renamed, context()).fingerprint(), fingerprint);
}

#[test]
fn any_payload_never_erases_execution_isolation() {
    let mut any = fixture();
    let parts = any["tools"]["javascript"]["installation"]["artifacts"]["linux-x64-gnu"].clone();
    any["tools"]["javascript"]["installation"]["artifacts"] = json!({"any":parts});
    // An equivalent native selector does not change actual bytes or layout.
    assert_eq!(
        snapshot(&any, context()).fingerprint(),
        snapshot(&fixture(), context()).fingerprint()
    );
    let base = snapshot(&any, context());
    let contexts = [
        execution(
            OperatingSystem::Windows,
            Architecture::X64,
            Libc::None,
            "x86_64-unknown-linux-gnu",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Macos,
            Architecture::Arm64,
            Libc::None,
            "x86_64-unknown-linux-gnu",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Linux,
            Architecture::Arm64,
            Libc::Gnu {
                abi: "glibc-2.31".into(),
            },
            "x86_64-unknown-linux-gnu",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Linux,
            Architecture::X64,
            Libc::Musl {
                abi: "musl-1.2.5".into(),
            },
            "x86_64-unknown-linux-gnu",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Linux,
            Architecture::X64,
            Libc::Gnu {
                abi: "glibc-2.38".into(),
            },
            "x86_64-unknown-linux-gnu",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Linux,
            Architecture::X64,
            Libc::Gnu {
                abi: "glibc-2.31".into(),
            },
            "wasm32-wasip2",
            &["x86-64-v2"],
        ),
        execution(
            OperatingSystem::Linux,
            Architecture::X64,
            Libc::Gnu {
                abi: "glibc-2.31".into(),
            },
            "x86_64-unknown-linux-gnu",
            &["x86-64-v3"],
        ),
    ];
    let native_output = b"\x7fELF\x02native-addon:gnu-2.31:x64:v2".to_vec();
    let cache = BTreeMap::from([(base.fingerprint().to_owned(), native_output.clone())]);
    let compatible = snapshot(&any, context());
    assert_eq!(cache.get(compatible.fingerprint()), Some(&native_output));
    for context in contexts {
        let incompatible = snapshot(&any, context);
        assert_ne!(incompatible.fingerprint(), base.fingerprint());
        assert!(!cache.contains_key(incompatible.fingerprint()));
    }
    any["tools"]["javascript"]["installation"]["artifacts"]["any"]["runtime"]["sha256"] =
        json!("e".repeat(64));
    let different_bytes = snapshot(&any, context());
    assert!(!cache.contains_key(different_bytes.fingerprint()));
}

#[test]
fn system_requirements_are_exact_distinct_and_order_independent() {
    let mut value = fixture();
    value["tools"]["pnpm"]["installation"]["executables"] = json!(["pnpm", "pnpx"]);
    let selected = snapshot(&value, context());
    let projected: Value = serde_json::from_slice(selected.canonical_identity()).unwrap();
    assert_eq!(
        projected["tools"]["pnpm"]["installation"],
        json!({"kind":"verify-system", "executables":["pnpm","pnpx"]})
    );
    value["tools"]["pnpm"]["installation"]["executables"] = json!(["pnpx", "pnpm"]);
    assert_eq!(
        snapshot(&value, context()).fingerprint(),
        selected.fingerprint()
    );
    value["tools"]["pnpm"]["installation"] = json!({"kind":"managed", "artifacts":{"any":{"runtime":{
        "url":"https://downloads.example.test/pnpm", "sha256":"a".repeat(64), "format":"binary", "executables":{"pnpm":"pnpm"}}}}});
    assert_ne!(
        snapshot(&value, context()).fingerprint(),
        selected.fingerprint()
    );
}

#[test]
fn selection_is_owned_and_missing_or_unsupported_inputs_fail_closed() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ExecutionSnapshot>();
    let mut value = fixture();
    let selected = snapshot(&value, context());
    let fingerprint = selected.fingerprint().to_owned();
    value["tools"]["javascript"]["version"] = json!("25.0.0");
    value["tools"]["javascript"]["installation"]["artifacts"]["linux-x64-gnu"]["runtime"]
        ["sha256"] = json!("e".repeat(64));
    assert_eq!(selected.fingerprint(), fingerprint);
    assert_eq!(selected.tools()["javascript"].version, "24.0.0");
    assert!(!format!("{selected:?}").contains("https://"));
    assert_ne!(snapshot(&value, context()).fingerprint(), fingerprint);
    assert_eq!(
        selected.context().artifact_platform(),
        Platform::LinuxX64Gnu
    );
    let lock = lock(&fixture());
    assert!(matches!(
        ExecutionSnapshot::select(&lock, context(), &BTreeSet::new()),
        Err(Error::EmptySelection)
    ));
    assert!(matches!(
        ExecutionSnapshot::select(&lock, context(), &ids(&["missing"])),
        Err(Error::MissingTool)
    ));
    let mac = execution(
        OperatingSystem::Macos,
        Architecture::X64,
        Libc::None,
        "x86_64-apple-darwin",
        &["generic"],
    );
    assert!(matches!(
        ExecutionSnapshot::select(&lock, mac, &ids(&["javascript"])),
        Err(Error::MissingArtifacts)
    ));
}

#[test]
fn validated_boundary_rejects_noncanonical_options_collisions_and_excess() {
    for options in [json!({"flags":["z","a"]}), json!({"flags":["a","a"]})] {
        let mut value = fixture();
        value["tools"]["javascript"]["options"] = options;
        assert!(Lock::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut value = fixture();
    value["tools"]["pnpm"]["installation"]["executables"] = json!(["node.exe"]);
    assert!(Lock::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    for (target, cpu) in [
        ("a".repeat(129), ids(&["generic"])),
        (
            "target".into(),
            (0..33).map(|i| format!("cpu-{i}")).collect(),
        ),
    ] {
        assert!(
            ExecutionContext::new(
                HostPlatform::new(OperatingSystem::Linux, Architecture::X64),
                Libc::Gnu {
                    abi: "glibc-2.31".into()
                },
                target,
                cpu
            )
            .is_err()
        );
    }
}

#[test]
fn explicit_context_rejects_unknown_or_incomplete_compatibility() {
    let linux = HostPlatform::new(OperatingSystem::Linux, Architecture::X64);
    for (host, libc, target, cpu) in [
        (linux, Libc::None, "target", ids(&["generic"])),
        (
            HostPlatform::new(OperatingSystem::Unknown, Architecture::X64),
            Libc::None,
            "target",
            ids(&["generic"]),
        ),
        (
            HostPlatform::new(OperatingSystem::Macos, Architecture::Unknown),
            Libc::None,
            "target",
            ids(&["generic"]),
        ),
        (
            linux,
            Libc::Gnu { abi: "".into() },
            "target",
            ids(&["generic"]),
        ),
        (
            linux,
            Libc::Gnu {
                abi: "glibc-2.31".into(),
            },
            "",
            ids(&["generic"]),
        ),
        (
            linux,
            Libc::Gnu {
                abi: "glibc-2.31".into(),
            },
            "target",
            BTreeSet::new(),
        ),
        (
            linux,
            Libc::Gnu {
                abi: "/machine/path".into(),
            },
            "target",
            ids(&["generic"]),
        ),
    ] {
        assert!(matches!(
            ExecutionContext::new(host, libc, target.into(), cpu),
            Err(Error::Execution)
        ));
    }
}
