use miette::Diagnostic;
use serde_json::{Value, json};
use test_case::test_case;

use super::*;
use crate::{RawRootTurboJson, RawTurboJson, SetupPolicy};

fn parse(setup: Value) -> Result<SetupPolicy, Error> {
    let json = json!({"futureFlags": {"experimentalSetup": true}, "setup": setup}).to_string();
    let raw: RawTurboJson = RawRootTurboJson::parse(&json, "turbo.json")?.try_into()?;
    SetupPolicy::from_raw(raw.setup.as_ref(), &raw.future_flags.unwrap().into_inner())
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

    let empty = parse(json!({"registries": [], "additionalTools": {"jq": "1.8.2"}})).unwrap();
    assert!(empty.registries().is_empty());
    assert!(empty.is_configured());
}

#[test]
fn registries_round_trip_as_written() {
    let registries = json!(["aqua"]);
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

#[test_case(json!(["aqua", "aqua"]), "\"aqua\""; "the built-in registry twice")]
fn invalid_registries_point_at_their_source(registries: Value, expected: &str) {
    let error = parse(json!({"registries": registries})).unwrap_err();
    assert!(
        matches!(&error, Error::SetupInvalidRegistry { reason, span: Some(_), .. }
            if reason.contains(expected)),
        "{error:?}"
    );
}

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
