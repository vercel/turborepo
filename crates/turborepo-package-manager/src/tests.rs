use serde_json::json;

use super::*;

#[test]
fn every_known_family_shares_exact_case_sensitive_recognition() {
    for family in Family::ALL {
        assert_eq!(Family::parse(family.name()), Some(family));
        for name in [
            family.name().to_uppercase(),
            format!(" {}", family.name()),
            format!("{} ", family.name()),
        ] {
            assert_eq!(Family::parse(&name), None);
        }
    }
    for unknown in [
        "",
        "corepack",
        "berry",
        "pnpm9",
        "other",
        "https://private.invalid",
    ] {
        assert_eq!(Family::parse(unknown), None);
    }
}

#[test]
fn specs_preserve_raw_version_grammar_for_each_consumer() {
    for family in Family::ALL {
        for version in [
            "1.2.3",
            "^1 || 2",
            " = v1.2.3 ",
            "1.2.3+sha512.abc",
            "https://example.invalid/pm.tgz",
            "",
        ] {
            let input = format!("{}@{version}", family.name());
            let spec = parse_spec(&input).unwrap();
            assert_eq!(spec.family, family);
            assert_eq!(spec.name, family.name());
            assert_eq!(spec.version, version);
        }
    }
    assert_eq!(parse_spec("npm"), Err(SpecError::ExpectedNameAtVersion));
    assert_eq!(parse_spec("unknown@1.2.3"), Err(SpecError::UnknownName));
    let error = parse_spec("private-input@1.2.3").unwrap_err();
    assert_eq!(error.to_string(), "unknown package-manager name");
    let error: &dyn std::error::Error = &error;
    assert!(error.source().is_none());
}

#[test]
fn entries_preserve_absence_constraints_and_raw_failure_policy() {
    for family in Family::ALL {
        let value = json!({"name": family.name(), "onFail": "warn", "extra": true});
        let entry = parse_entry(&value).unwrap();
        assert_eq!(entry.family, family);
        assert_eq!(entry.version, None);
        assert_eq!(entry.version().unwrap(), None);
        assert_eq!(entry.on_fail, Some(&value["onFail"]));
        for version in ["", "*", " ^1 || 2 ", "1.2.3+build"] {
            let value = json!({"name": family.name(), "version": version, "onFail": 42});
            let entry = parse_entry(&value).unwrap();
            assert_eq!(entry.version, Some(&value["version"]));
            assert_eq!(entry.version().unwrap(), Some(version));
            assert_eq!(entry.on_fail, Some(&value["onFail"]));
        }
    }
}

#[test]
fn malformed_entry_structure_has_input_free_field_errors() {
    for (value, expected) in [
        (json!(null), EntryError::ExpectedObject),
        (json!([]), EntryError::ExpectedObject),
        (json!({}), EntryError::MissingName),
        (json!({"name": 42}), EntryError::NameNotString),
        (json!({"name": ""}), EntryError::EmptyName),
        (json!({"name": " pnpm"}), EntryError::NameWhitespace),
        (json!({"name": "private-input"}), EntryError::UnknownName),
    ] {
        let error = parse_entry(&value).unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?}").contains("private-input"));
        assert!(!error.to_string().contains("private-input"));
    }
    assert_eq!(EntryError::ExpectedObject.field(), None);
    assert_eq!(EntryError::MissingName.field(), Some("name"));
    assert_eq!(EntryError::VersionNotString.field(), Some("version"));
}

#[test]
fn entry_version_types_can_be_checked_after_consumer_capabilities() {
    for family in Family::ALL {
        for version in [
            json!(null),
            json!(42),
            json!([]),
            json!({"private-input": true}),
        ] {
            let value = json!({"name": family.name(), "version": version, "onFail": null});
            let entry = parse_entry(&value).unwrap();
            assert_eq!(entry.family, family);
            assert!(std::ptr::eq(entry.version.unwrap(), &value["version"]));
            assert!(std::ptr::eq(entry.on_fail.unwrap(), &value["onFail"]));
            let error = entry.version().unwrap_err();
            assert_eq!(error, EntryError::VersionNotString);
            assert!(!format!("{error} {error:?}").contains("private-input"));
            let error: &dyn std::error::Error = &error;
            assert!(error.source().is_none());
        }
    }
}
