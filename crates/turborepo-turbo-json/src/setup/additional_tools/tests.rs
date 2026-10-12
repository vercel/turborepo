use miette::Diagnostic;
use serde_json::{Value, json};
use test_case::test_case;

use super::*;
use crate::{RawPackageTurboJson, RawRootTurboJson, RawTurboJson, SetupPolicy, TurboJson};

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn archive() -> Value {
    json!({"url": "https://downloads.example.test/fixture-1.2.3.tar.gz", "sha256": SHA,
        "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
        "executables": {"fixture": "bin/fixture", "fixture-helper": "libexec/helper"}})
}

fn binary(name: &str) -> Value {
    json!({"url": "https://downloads.example.test/fixture.exe", "sha256": SHA,
        "format": "binary", "executables": {name: format!("bin/{name}.exe")}})
}

fn tool() -> Value {
    json!({"version": "1.2.3", "artifacts": {
        "linux-x64-gnu": archive(), "macos-arm64": archive(), "windows-x64": binary("fixture")}})
}

fn parse(tools: Value) -> Result<SetupPolicy, Error> {
    let json = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": tools}})
    .to_string();
    let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")?.try_into()?;
    SetupPolicy::from_raw(raw.setup.as_ref(), &raw.future_flags.unwrap().into_inner())
}

fn with(mutate: impl FnOnce(&mut Value)) -> Result<SetupPolicy, Error> {
    let mut tools = json!({"fixture": tool()});
    mutate(&mut tools);
    parse(tools)
}

#[test]
fn unknown_tools_are_declared_without_registration() {
    let policy = with(|_| {}).unwrap();
    assert!(policy.is_configured());
    let fixture = &policy.additional_tools()["fixture"];
    assert_eq!(fixture.version.as_str(), "1.2.3");
    assert_eq!(
        fixture.artifacts.keys().copied().collect::<Vec<_>>(),
        [
            ArtifactPlatform::MacosArm64,
            ArtifactPlatform::LinuxX64Gnu,
            ArtifactPlatform::WindowsX64
        ]
    );
    let linux = &fixture.artifacts[&ArtifactPlatform::LinuxX64Gnu];
    assert_eq!(*linux.format, ArtifactFormat::TarGz);
    assert_eq!(
        linux.root_prefix.as_deref().unwrap().as_str(),
        "fixture-1.2.3"
    );
    assert_eq!(
        linux.executables["fixture-helper"].as_str(),
        "libexec/helper"
    );
    let windows = &fixture.artifacts[&ArtifactPlatform::WindowsX64];
    assert_eq!(*windows.format, ArtifactFormat::Binary);
    assert_eq!(windows.root_prefix, None);

    // Serialization round-trips the authored field names for `turbo prune`.
    let json = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": {"fixture": tool()}}})
    .to_string();
    let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&raw).unwrap()["setup"]["additionalTools"]["fixture"],
        tool()
    );
}

#[test]
fn repositories_without_additional_tools_are_unchanged() {
    let policy = SetupPolicy::from_raw(None, &Default::default()).unwrap();
    assert!(policy.additional_tools().is_empty());
    assert!(parse(json!({})).unwrap().additional_tools().is_empty());
}

#[test]
fn archives_without_a_top_level_directory_omit_the_root_prefix() {
    let policy = with(|t| {
        let macos = t["fixture"]["artifacts"]["macos-arm64"]
            .as_object_mut()
            .unwrap();
        macos.remove("rootPrefix");
    })
    .unwrap();
    let macos = &policy.additional_tools()["fixture"].artifacts[&ArtifactPlatform::MacosArm64];
    assert_eq!(macos.root_prefix, None);
}

#[test_case(|t| t["Fixture"] = tool(); "uppercase id")]
#[test_case(|t| t["fixture"]["version"] = json!("^1.2.3"); "range")]
#[test_case(|t| t["fixture"]["version"] = json!("1.x"); "wildcard")]
#[test_case(|t| t["fixture"]["version"] = json!("latest"); "channel")]
#[test_case(|t| t["fixture"]["artifacts"] = json!({}); "no platforms")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["url"] = json!("http://example.test/a"); "http")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["url"] = json!("https://user:token@example.test/a"); "userinfo")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["url"] = json!("https://example.test/a?token=x"); "query")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["url"] = json!("https://example.test/a#x"); "fragment")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["sha256"] = json!(SHA.to_uppercase()); "uppercase digest")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["sha256"] = json!("abc"); "short digest")]
#[test_case(|t| t["fixture"]["artifacts"]["windows-x64"]["rootPrefix"] = json!("bin"); "binary prefix")]
#[test_case(|t| t["fixture"]["artifacts"]["windows-x64"]["executables"]["other"] = json!("other"); "binary executables")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["rootPrefix"] = json!("a/b"); "nested prefix")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"]["fixture"] = json!("../fixture"); "escaping path")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"]["fixture"] = json!("bin\\fixture"); "backslash path")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"]["fixture"] = json!("bin/NUL"); "device path")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"]["bin/fixture"] = json!("bin/fixture"); "nested name")]
fn malformed_declarations_point_at_their_source(mutate: fn(&mut Value)) {
    let error = with(mutate).unwrap_err();
    assert!(
        matches!(
            error,
            Error::SetupInvalidAdditionalTool { span: Some(_), .. }
        ),
        "{error:?}"
    );
}

#[test_case(|t| t["fixture"]["artifacts"]["linux-x64"] = archive(); "unknown platform")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["format"] = json!("pkg"); "unknown format")]
#[test_case(|t| t["fixture"]["postinstall"] = json!("./install.sh"); "install hook")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["command"] = json!("make"); "artifact hook")]
#[test_case(|t| { t["fixture"].as_object_mut().unwrap().remove("version"); }; "missing version")]
#[test_case(|t| { t["fixture"]["artifacts"]["macos-arm64"].as_object_mut().unwrap().remove("sha256"); }; "missing digest")]
#[test_case(|t| t["fixture"] = json!("1.2.3"); "shorthand")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"] = json!({}); "no executables")]
fn unknown_or_missing_fields_are_parse_errors(mutate: fn(&mut Value)) {
    let Err(Error::Parse(error)) = with(mutate) else {
        panic!("expected a parse error");
    };
    assert!(!error.diagnostics.is_empty());
    for diagnostic in &error.diagnostics {
        assert!(diagnostic.labels().is_some(), "{diagnostic:?}");
    }
}

#[test_case(|t| t["node"] = tool(), "node"; "builtin id")]
#[test_case(|t| t["uv"] = tool(), "uv"; "future builtin id")]
#[test_case(|t| t["fixture"]["artifacts"]["macos-arm64"]["executables"]["npm"] = json!("bin/npm"), "npm"; "builtin executable")]
#[test_case(|t| t["fixture"]["artifacts"]["windows-x64"] = binary("Cargo.EXE"), "Cargo.EXE"; "windows alias")]
#[test_case(|t| t["other"] = tool(), "fixture"; "another additional tool")]
#[test_case(|t| t["other"] = json!({"version": "1.0.0", "artifacts": {"windows-x64": binary("FIXTURE.cmd")}}), "FIXTURE.cmd"; "windows alias of another tool")]
fn collisions_are_rejected(mutate: fn(&mut Value), expected: &str) {
    let error = with(mutate).unwrap_err();
    assert!(
        matches!(&error, Error::SetupInvalidAdditionalTool { reason, span: Some(_), .. }
            if reason.contains(&format!("{expected:?}"))),
        "{error:?}"
    );
}

#[test]
fn executables_may_repeat_on_disjoint_platforms() {
    let policy = with(|tools| {
        tools["other"] = json!({"version": "2026.10.1",
            "artifacts": {"linux-arm64-gnu": archive(), "macos-x64": archive()}});
        // Distinct names only fold together on Windows.
        tools["fixture"]["artifacts"]["linux-x64-gnu"]["executables"]["fixture.exe"] =
            json!("bin/fixture.exe");
    })
    .unwrap();
    assert_eq!(policy.additional_tools().len(), 2);
}

#[test]
fn additional_tools_are_root_only() {
    let json = json!({"extends": ["//"], "setup": {"additionalTools": {"fixture": tool()}}});
    let raw = RawPackageTurboJson::parse(&json.to_string(), "packages/web/turbo.json").unwrap();
    let error = TurboJson::try_from(RawTurboJson::from(raw)).unwrap_err();
    assert!(
        matches!(error, Error::SetupInPackage { span: Some(_), .. }),
        "{error:?}"
    );
}
