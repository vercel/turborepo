use serde_json::{Value, json};

use super::*;

fn artifact(name: &str) -> Value {
    json!({"url":"https://downloads.example.test/tool.tar.gz", "sha256":"a".repeat(64),
        "format":"tar-gz", "rootPrefix":"tool-1.0.0", "executables":{name:format!("bin/{name}")}})
}
fn tool(name: &str, platform: &str) -> Value {
    json!({"adapter":"generic-download", "version":"release@2026-10-06", "declarations":[
        {"file":"turbo.json", "field":"/setup/additionalTools", "request":"1.0.0"}],
        "installation":{"kind":"managed", "artifacts":{platform:{"distribution":artifact(name)}}}})
}
fn fixture() -> Value {
    json!({"schemaVersion":0,"tools":{"fixture":tool("fixture", "any")}})
}
fn parse(value: &Value) -> Result<Lock, Error> {
    Lock::parse(&serde_json::to_vec(value).unwrap())
}
fn reject(mutator: impl FnOnce(&mut Value)) {
    let mut value = fixture();
    mutator(&mut value);
    assert!(
        parse(&value).is_err(),
        "accepted malformed fixture: {value}"
    );
    if let Ok(document) = serde_json::from_value(value) {
        assert!(Lock::new(document).is_err());
    }
}

#[test]
fn portable_artifact_sets_and_external_requirements() {
    let mut value = fixture();
    value["tools"]["node"] = tool("node", "macos-arm64");
    value["tools"]["node"]["adapter"] = json!("node");
    value["tools"]["node"]["version"] = json!("24.0.0");
    value["tools"]["node"]["installation"]["artifacts"]["windows-x64"] =
        json!({"distribution":artifact("node")});
    value["tools"]["rust"] = tool("cargo", "linux-x64-gnu");
    value["tools"]["rust"]["adapter"] = json!("rust");
    value["tools"]["rust"]["version"] = json!("1.99.0-nightly@2026-10-06");
    value["tools"]["rust"]["options"] =
        json!({"components":["clippy", "rustfmt"],"targets":["aarch64-unknown-linux-gnu"]});
    let mut component = artifact("rustfmt");
    component["destination"] = json!("components/rustfmt");
    value["tools"]["rust"]["installation"]["artifacts"]["linux-x64-gnu"]["rustfmt"] = component;
    value["tools"]["go"] = tool("go", "any");
    value["tools"]["go"]["installation"] = json!({"kind":"verify-system", "executables":["go"]});
    let lock = parse(&value).unwrap();
    assert_eq!(lock.tools().len(), 4);
    assert_eq!(
        parse(&serde_json::to_value(lock.document()).unwrap()).unwrap(),
        lock
    );
}

#[test]
fn strict_json_schema_and_duplicate_keys_at_every_depth() {
    for invalid in [
        b"{\"schemaVersion\":0,\"schemaVersion\":0,\"tools\":{}}".as_slice(),
        b"{\"schemaVersion\":0,\"tools\":{\"x\":{},\"x\":{}}}",
        b"{\"schemaVersion\":0,\"tools\":{},\"tools\":{}}",
        b"{\"schemaVersion\":0,\"tools\":{},\"dependencyGraph\":{}}",
    ] {
        assert!(Lock::parse(invalid).is_err());
    }
    let bytes = serde_json::to_string(&fixture()).unwrap();
    for (field, replacement) in [
        ("\"version\":", "\"version\":\"evil\",\"version\":"),
        ("\"sha256\":", "\"sha256\":\"evil\",\"sha256\":"),
        ("\"executables\":", "\"executables\":{},\"executables\":"),
        ("\"file\":", "\"file\":\"evil\",\"file\":"),
    ] {
        assert_eq!(
            Lock::parse(bytes.replace(field, replacement).as_bytes()),
            Err(Error::Json)
        );
    }
    for path in [
        "",
        "/tools/fixture",
        "/tools/fixture/declarations/0",
        "/tools/fixture/installation",
        "/tools/fixture/installation/artifacts/any/distribution",
    ] {
        reject(|v| {
            v.pointer_mut(path).unwrap()["unknown"] = json!(true);
        });
    }
    reject(|v| v["schemaVersion"] = json!(1));
    reject(|v| v["schemaVersion"] = json!(-1));
    reject(|v| v["tools"]["fixture"]["installation"]["artifacts"]["linux-x64"] = json!({}));
    for unknown in ["artifacts", "executablesExtra", "unknown"] {
        reject(|v| {
            v["tools"]["fixture"]["installation"] =
                json!({"kind":"verify-system", "executables":["fixture"], unknown:{}})
        });
    }
}

#[test]
fn bounds_and_recursion_remain_enforced() {
    assert_eq!(
        Lock::parse(&vec![b' '; MAX_LOCK_BYTES + 1]),
        Err(Error::TooLarge)
    );
    let recursive = format!("{}0{}", "[".repeat(150), "]".repeat(150));
    assert_eq!(Lock::parse(recursive.as_bytes()), Err(Error::Json));
    let error = serde_json::from_str::<UniqueJson>(&recursive)
        .err()
        .unwrap();
    assert!(error.to_string().contains("recursion limit exceeded"));
    assert!(
        serde_json::from_str::<UniqueJson>(&format!("{}0{}", "[".repeat(10), "]".repeat(10)))
            .is_ok()
    );
    reject(|v| v["tools"] = json!(null));
    reject(|v| v["tools"]["fixture"]["version"] = json!("x".repeat(129)));
    reject(|v| v["tools"]["fixture"]["declarations"] = json!([]));
    reject(|v| v["tools"]["fixture"]["declarations"][0]["request"] = json!("x".repeat(4097)));
    reject(|v| v["tools"]["fixture"]["installation"]["artifacts"] = json!({}));
    reject(|v| v["tools"]["fixture"]["options"] = json!({"components":["rustfmt", "clippy"]}));
    reject(|v| v["tools"]["fixture"]["options"] = json!({"components":["clippy", "clippy"]}));
    reject(|v| {
        v["tools"]["fixture"]["installation"] = json!({"kind":"verify-system", "executables":[]})
    });
}

#[test]
fn transport_digest_and_layout_are_strict_without_secret_diagnostics() {
    let pointer = "/tools/fixture/installation/artifacts/any/distribution";
    for url in [
        "http://downloads.example.test/tool",
        "https://user:secret@example.test/tool",
        "https://example.test/tool?token=secret",
        "https://example.test/tool#secret",
        "https://example.test\\tool",
        "https://@example.test/tool",
        "https:///@example.test/tool",
        "https:////@example.test/tool",
    ] {
        let mut value = fixture();
        value.pointer_mut(pointer).unwrap()["url"] = json!(url);
        let error = parse(&value).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("secret"));
    }
    for digest in ["", "aa", &"A".repeat(64), &"g".repeat(64)] {
        reject(|v| v.pointer_mut(pointer).unwrap()["sha256"] = json!(digest));
    }
    for path in [
        "/bin/tool",
        "../tool",
        "a/../tool",
        "a//b",
        "C:/tool",
        "a\\b",
        "CON",
        "nul.txt",
        "com1.exe",
        "LPT9",
        "tool.",
        "tool ",
        "",
        "./bin",
    ] {
        reject(|v| v.pointer_mut(pointer).unwrap()["rootPrefix"] = json!(path));
        reject(|v| v.pointer_mut(pointer).unwrap()["executables"] = json!({"fixture":path}));
        reject(|v| v["tools"]["fixture"]["declarations"][0]["file"] = json!(path));
    }
    reject(|v| v.pointer_mut(pointer).unwrap()["format"] = json!("binary"));
    let mut binary = fixture();
    let part = binary.pointer_mut(pointer).unwrap();
    part["format"] = json!("binary");
    part.as_object_mut().unwrap().remove("rootPrefix");
    assert!(parse(&binary).is_ok());
}

#[test]
fn collision_scope_includes_any_system_and_windows_aliases_not_inactive_platforms() {
    let mut value = fixture();
    value["tools"]["other"] = tool("fixture", "macos-x64");
    assert!(parse(&value).is_err());
    value["tools"]["fixture"] = tool("fixture", "linux-x64-gnu");
    assert!(parse(&value).is_ok()); // Mutually exclusive platforms.
    value["tools"]["other"] = tool("FIXTURE.exe", "windows-x64");
    value["tools"]["fixture"] = tool("fixture", "windows-x64");
    assert!(parse(&value).is_err());
    value["tools"]["other"]["installation"] =
        json!({"kind":"verify-system", "executables":["fixture"]});
    assert!(parse(&value).is_err());
    reject(|v| {
        v["tools"]["fixture"]["installation"]["artifacts"]["macos-x64"] =
            json!({"part":artifact("other")})
    });
}

#[test]
fn collection_and_path_boundaries_and_resource_only_components() {
    let mut document: Document = serde_json::from_value(fixture()).unwrap();
    let template = document.tools.remove("fixture").unwrap();
    for i in 0..MAX_TOOLS {
        let mut tool = template.clone();
        tool.installation = Installation::VerifySystem {
            executables: vec![format!("tool-{i}")],
        };
        document.tools.insert(format!("tool-{i}"), tool);
    }
    assert!(Lock::new(document.clone()).is_ok());
    document.tools.insert("extra".into(), template.clone());
    assert!(Lock::new(document).is_err());
    let mut tool = template;
    tool.declarations = (0..MAX_DECLARATIONS)
        .map(|i| Declaration {
            file: format!("manifest-{i}"),
            field: None,
            request: None,
        })
        .collect();
    let mut doc = Document {
        schema_version: 0,
        tools: BTreeMap::from([("fixture".into(), tool)]),
    };
    assert!(Lock::new(doc.clone()).is_ok());
    doc.tools
        .get_mut("fixture")
        .unwrap()
        .declarations
        .push(Declaration {
            file: "extra".into(),
            field: None,
            request: None,
        });
    assert!(Lock::new(doc).is_err());
    assert!(portable_path(&"x".repeat(255)));
    assert!(!portable_path(&"x".repeat(256)));
    assert!(portable_path(&vec!["x"; 32].join("/")));
    assert!(!portable_path(&vec!["x"; 33].join("/")));
    let mut value = fixture();
    let parts = &mut value["tools"]["fixture"]["installation"]["artifacts"]["any"];
    let mut resource = artifact("unused");
    resource["executables"] = json!({});
    for i in 1..MAX_PARTS {
        parts[format!("part-{i}")] = resource.clone();
    }
    assert!(parse(&value).is_ok());
    value["tools"]["fixture"]["installation"]["artifacts"]["any"]["extra"] = resource;
    assert!(parse(&value).is_err());
    reject(|v| {
        v["tools"]["fixture"]["installation"] =
            json!({"kind":"verify-system", "executables":["fixture","fixture.cmd"]})
    });
    reject(|v| {
        v["tools"]["fixture"]["installation"]["artifacts"]["any"]["second"] = artifact("fixture")
    });
}

#[test]
fn current_builtin_versions_are_exact_and_constructor_is_not_a_bypass() {
    for version in ["24", "24.x", "v24.0.0", "^24.0.0", "lts/*", "24.0.0 "] {
        reject(|v| {
            v["tools"]["fixture"]["adapter"] = json!("node");
            v["tools"]["fixture"]["version"] = json!(version);
        });
    }
    let mut document: Document = serde_json::from_value(fixture()).unwrap();
    document.tools.values_mut().next().unwrap().declarations[0].file = "../outside".into();
    assert!(Lock::new(document).is_err());
}
