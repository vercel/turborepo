use super::*;

const ARTIFACT: &str = "/tools/protoc/installation/artifacts/linux-x64-gnu/artifact";
const GOLDEN: &str = "84f065765efeaab8b335c70b33c434d1e053b91908c50690e903e13859c90c14";

/// A `setup.additionalTools` entry exactly as `lock::additional` projects it.
fn locked() -> Value {
    let artifact = |sha: &str| {
        json!({"url": "https://downloads.example.test/protoc-28.3.tar.gz",
            "sha256": sha.repeat(64), "format": "tar-gz", "rootPrefix": "protoc-28.3",
            "executables": {"protoc": "bin/protoc"}})
    };
    json!({"schemaVersion": 0, "tools": {"protoc": {"adapter": "artifact", "version": "28.3",
        "declarations": [{"file": "turbo.json", "field": "/setup/additionalTools/protoc",
            "request": "28.3"}],
        "installation": {"kind": "managed", "artifacts": {
            "linux-x64-gnu": {"artifact": artifact("a")},
            "macos-arm64": {"artifact": artifact("b")}}}}}})
}

fn select(value: &Value) -> ExecutionSnapshot {
    ExecutionSnapshot::select(&lock(value), context(), &ids(&["protoc"])).unwrap()
}

#[test]
fn additional_tools_hash_artifact_identity_and_layout() {
    let original = locked();
    let selected = select(&original);
    let identity: Value = serde_json::from_slice(selected.canonical_identity()).unwrap();
    assert_eq!(
        identity["tools"]["protoc"],
        json!({"adapter": "artifact", "version": "28.3", "options": {},
            "installation": {"kind": "managed", "parts": {"artifact": {
                "sha256": "a".repeat(64), "format": "tar-gz", "rootPrefix": "protoc-28.3",
                "destination": null, "executables": {"protoc": "bin/protoc"}}}}})
    );
    assert_eq!(selected.fingerprint(), GOLDEN);
    // The version text stays the same in every case.
    for (field, value) in [
        ("sha256", json!("c".repeat(64))),
        ("format", json!("zip")),
        ("rootPrefix", json!("protoc")),
        ("executables", json!({"protoc": "protoc"})),
        ("executables", json!({"protoc3": "bin/protoc"})),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(&format!("{ARTIFACT}/{field}")).unwrap() = value;
        assert_ne!(
            select(&changed).fingerprint(),
            selected.fingerprint(),
            "ignored {field}"
        );
    }
}

#[test]
fn sources_provenance_inactive_platforms_and_location_do_not_churn() {
    let original = locked();
    let mut mirrored = original.clone();
    *mirrored.pointer_mut(&format!("{ARTIFACT}/url")).unwrap() =
        json!("https://mirror.example.test/relocated/protoc.tar.gz");
    mirrored["tools"]["protoc"]["declarations"][0]["file"] = json!("turbo.jsonc");
    mirrored["tools"]["protoc"]["installation"]["artifacts"]["macos-arm64"]["artifact"]["sha256"] =
        json!("d".repeat(64));
    let mut fingerprints = BTreeSet::new();
    for value in [original, mirrored] {
        let clone = tempfile::tempdir().unwrap();
        let path = clone.path().join("turbo.lock");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let parsed = Lock::parse(&fs::read(&path).unwrap()).unwrap();
        let selected = ExecutionSnapshot::select(&parsed, context(), &ids(&["protoc"])).unwrap();
        let canonical = String::from_utf8_lossy(selected.canonical_identity()).into_owned();
        assert!(!canonical.contains("example.test"));
        assert!(!canonical.contains(clone.path().to_str().unwrap()));
        fingerprints.insert(selected.fingerprint().to_owned());
    }
    assert_eq!(fingerprints.len(), 1);
}

#[test]
fn builtin_identity_is_unchanged_beside_additional_tools() {
    let mut value = fixture();
    value["tools"]["protoc"] = locked()["tools"]["protoc"].clone();
    let builtin = snapshot(&value, context());
    assert_eq!(
        builtin.fingerprint(),
        include_str!("../golden.sha256").trim()
    );
    let every = ids(&["javascript", "npm", "pnpm", "protoc"]);
    let all = ExecutionSnapshot::select(&lock(&value), context(), &every).unwrap();
    assert_ne!(all.fingerprint(), builtin.fingerprint());
    let mac = execution(
        OperatingSystem::Macos,
        Architecture::X64,
        Libc::None,
        "x86_64-apple-darwin",
        &["generic"],
    );
    assert!(matches!(
        ExecutionSnapshot::select(&lock(&locked()), mac, &ids(&["protoc"])),
        Err(Error::MissingArtifacts)
    ));
}
