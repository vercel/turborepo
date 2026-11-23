use miette::Diagnostic;
use serde_json::{Value, json};
use test_case::test_case;

use super::*;
use crate::{AdditionalToolSource, RawRootTurboJson, RawTurboJson, SetupPolicy};

fn parse(setup: Value) -> Result<SetupPolicy, Error> {
    let json = json!({"futureFlags": {"experimentalSetup": true}, "setup": setup}).to_string();
    let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")?.try_into()?;
    SetupPolicy::from_raw(raw.setup.as_ref(), &raw.future_flags.unwrap().into_inner())
}

fn acme() -> Value {
    json!({"name": "acme", "path": "tools/registry.yaml"})
}

fn file(name: &str, path: &str) -> SetupRegistry {
    SetupRegistry::File {
        name: name.into(),
        path: path.into(),
    }
}

#[test]
fn registries_keep_their_order_and_default_to_the_built_in_one() {
    let absent = parse(json!({})).unwrap();
    assert_eq!(absent.registries(), [SetupRegistry::Aqua]);
    assert!(!absent.is_configured());
    let unset = SetupPolicy::from_raw(None, &Default::default()).unwrap();
    assert_eq!(unset.registries(), [SetupRegistry::Aqua]);

    let written = parse(json!({"registries": ["aqua"]})).unwrap();
    assert_eq!(written, absent);

    let ordered = parse(json!({"registries": [acme(), "aqua"]})).unwrap();
    assert_eq!(
        ordered.registries(),
        [file("acme", "tools/registry.yaml"), SetupRegistry::Aqua]
    );
    assert_eq!(ordered.registries()[0].name(), "acme");
    assert_eq!(ordered.registries()[1].name(), "aqua");
    assert!(ordered.is_configured());

    let short = parse(json!({"registries": [{"name": "acme", "path": "registry.yml"}]})).unwrap();
    assert_eq!(short.registries(), [file("acme", "registry.yml")]);

    let blocked = parse(json!({"registries": [acme()]})).unwrap();
    assert_eq!(blocked.registries(), [file("acme", "tools/registry.yaml")]);

    let empty = parse(json!({"registries": [], "additionalTools": {"jq": "1.8.2"}})).unwrap();
    assert!(empty.registries().is_empty());
    assert!(empty.is_configured());
}

#[test]
fn registries_round_trip_as_written() {
    let registries = json!([acme(), "aqua", {"name": "team_2", "path": "registry.yaml"}]);
    let json = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"registries": registries}})
    .to_string();
    let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&raw).unwrap()["setup"]["registries"],
        registries
    );
}

#[test]
fn from_names_a_declared_registry_and_a_package_of_one_or_more_segments() {
    let policy = parse(json!({
        "registries": [acme(), "aqua"],
        "additionalTools": {
            "acme-cli": "1.4.2",
            "deploy": {"version": "2.0.0", "from": "acme:deploy-tool"},
            "rg": {"version": "15.2.0", "from": "aqua:BurntSushi/ripgrep"},
        },
    }))
    .unwrap();
    let from = |id: &str| match &policy.additional_tools()[id].source {
        AdditionalToolSource::Catalog { from } => from.clone(),
        AdditionalToolSource::Artifacts(_) => panic!("expected a registry declaration"),
    };
    assert_eq!(from("acme-cli"), None);
    assert_eq!(from("deploy").as_deref(), Some("acme:deploy-tool"));
    assert_eq!(from("rg").as_deref(), Some("aqua:BurntSushi/ripgrep"));

    let default =
        parse(json!({"additionalTools": {"jq": {"version": "1.8.2", "from": "aqua:jq"}}}));
    assert!(default.is_ok(), "{default:?}");
}

#[test]
fn sources_split_into_a_registry_and_a_package_id() {
    assert_eq!(
        registry_package("aqua:jqlang/jq"),
        Some(("aqua", "jqlang/jq"))
    );
    assert_eq!(
        registry_package("acme:acme-cli"),
        Some(("acme", "acme-cli"))
    );
    assert_eq!(
        registry_package("team_2:a.b/c_d/e-f"),
        Some(("team_2", "a.b/c_d/e-f"))
    );
    let longest = format!("aqua:{}/{}", "a".repeat(127), "b".repeat(128));
    assert!(registry_package(&longest).is_some());
    assert_eq!(registry_package(&format!("{longest}c")), None);
    let registry = "r".repeat(64);
    assert!(registry_package(&format!("{registry}:jq")).is_some());
    assert_eq!(registry_package(&format!("{registry}r:jq")), None);
    for source in [
        "jqlang/jq",
        "aqua:",
        ":jq",
        "aqua:jqlang/",
        "aqua:/jq",
        "aqua:jqlang//jq",
        "aqua:jqlang/jq@1.8.2",
        "aqua:jqlang/jq ",
        "aqua:jqlang\\jq",
        "aqua:acme:jq",
        "AQUA:jqlang/jq",
        "1aqua:jq",
        "my.registry:jq",
    ] {
        assert_eq!(registry_package(source), None, "{source}");
    }
}

#[test_case(json!([acme(), acme()]), "\"acme\""; "a duplicate name")]
#[test_case(json!([acme(), {"name": "acme", "path": "other.yaml"}]), "\"acme\""; "one name for two files")]
#[test_case(json!([{"name": "aqua", "path": "tools/registry.yaml"}]), "`aqua`"; "the built-in name on a file")]
#[test_case(json!(["aqua", acme(), "aqua"]), "\"aqua\""; "the built-in registry twice")]
#[test_case(json!([{"name": "Acme", "path": "tools/registry.yaml"}]), "registry names"; "an uppercase name")]
#[test_case(json!([{"name": "", "path": "tools/registry.yaml"}]), "registry names"; "an empty name")]
#[test_case(json!([{"name": "acme", "path": "../registry.yaml"}]), "`path`"; "a path above the root")]
#[test_case(json!([{"name": "acme", "path": "/etc/registry.yaml"}]), "`path`"; "an absolute path")]
#[test_case(json!([{"name": "acme", "path": "tools\\registry.yaml"}]), "`path`"; "a backslash path")]
#[test_case(json!([{"name": "acme", "path": ""}]), "`path`"; "an empty path")]
#[test_case(json!([{"name": "acme", "path": ".env"}]), "`.yaml` or `.yml`"; "a file that is not YAML")]
#[test_case(json!([{"name": "acme", "path": "tools/registry"}]), "`.yaml` or `.yml`"; "a path without an extension")]
#[test_case(json!([{"name": "acme", "path": "tools/registry.yaml.txt"}]), "`.yaml` or `.yml`"; "a path that only contains the extension")]
#[test_case(json!([{"name": "acme", "path": "tools/registry.YAML"}]), "`.yaml` or `.yml`"; "an uppercase extension")]
fn invalid_registries_point_at_their_source(registries: Value, expected: &str) {
    let error = parse(json!({"registries": registries})).unwrap_err();
    assert!(
        matches!(&error, Error::SetupInvalidRegistry { reason, span: Some(_), .. }
            if reason.contains(expected)),
        "{error:?}"
    );
}

#[test_case(json!([{"name": "acme", "path": "tools/registry.yaml", "url": "https://example.test"}]); "an unknown field")]
#[test_case(json!([{"name": "acme"}]); "a missing path")]
#[test_case(json!([{"path": "tools/registry.yaml"}]); "a missing name")]
#[test_case(json!(["mise"]); "an unknown built-in registry")]
#[test_case(json!([1]); "a number")]
#[test_case(json!("aqua"); "a registry instead of a list")]
fn malformed_registries_are_parse_errors(registries: Value) {
    let Err(Error::Parse(error)) = parse(json!({"registries": registries})) else {
        panic!("expected a parse error");
    };
    assert!(!error.diagnostics.is_empty());
    for diagnostic in &error.diagnostics {
        assert!(diagnostic.labels().is_some(), "{diagnostic:?}");
    }
}

#[test_case(json!([acme(), "aqua"]), "nope:thing", "\"nope\""; "an undeclared registry")]
#[test_case(json!([acme()]), "aqua:jqlang/jq", "\"aqua\""; "the built-in registry when the list omits it")]
#[test_case(json!([]), "aqua:jqlang/jq", "\"aqua\""; "any registry when the list is empty")]
#[test_case(json!(["aqua"]), "acme:acme-cli", "\"acme\""; "a file registry that is not declared")]
fn from_must_name_a_declared_registry(registries: Value, from: &str, expected: &str) {
    let error = parse(json!({
        "registries": registries,
        "additionalTools": {"thing": {"version": "1.0.0", "from": from}},
    }))
    .unwrap_err();
    assert!(
        matches!(&error, Error::SetupInvalidAdditionalTool { tool, reason, span: Some(_), .. }
            if tool == "thing" && reason.contains(expected) && reason.contains("setup.registries")),
        "{error:?}"
    );

    let undeclared =
        parse(json!({"additionalTools": {"thing": {"version": "1.0.0", "from": "acme:thing"}}}));
    assert!(
        matches!(&undeclared, Err(Error::SetupInvalidAdditionalTool { reason, .. })
            if reason.contains("\"acme\"")),
        "{undeclared:?}"
    );
}
