use serde_json::{Value, json};

use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn declaration(value: Value) -> Result<Declaration, Box<dyn std::error::Error>> {
    discover_package_manager(&value)?.ok_or_else(|| "missing declaration".into())
}
fn dev(version: String, on_fail: &str) -> Value {
    json!({"name":"pnpm","version":version,"onFail":on_fail})
}
#[test]
fn top_and_dev_pins_are_preflighted_and_combined_without_losing_strong_integrity() -> TestResult {
    let version = Version::parse("10.0.0")?;
    let sha256 = "a".repeat(64);
    let sha512 = "b".repeat(128);
    for algorithm in ["sha256", "sha512"] {
        let digest = if algorithm == "sha256" {
            &sha256
        } else {
            &sha512
        };
        let pin = format!("10.0.0+{algorithm}.{digest}");
        for value in [
            json!({"packageManager":format!("pnpm@{pin}")}),
            json!({"devEngines":{"packageManager":dev(pin.clone(), "error")}}),
            json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":dev(pin, "error")}}),
        ] {
            let selected = declaration(value)?
                .locked_integrity(&version, &sha256)?
                .ok_or("pin dropped")?;
            assert_eq!(selected.algorithm, algorithm);
            assert_eq!(&selected.digest, digest);
        }
    }
    for (top, alternative) in [
        (
            format!("pnpm@10.0.0+sha256.{sha256}"),
            format!("10.0.0+sha512.{sha512}"),
        ),
        (
            format!("pnpm@10.0.0+sha512.{sha512}"),
            format!("10.0.0+sha256.{sha256}"),
        ),
    ] {
        let selected = declaration(
            json!({"packageManager":top,"devEngines":{"packageManager":dev(alternative,"error")}}),
        )?
        .locked_integrity(&version, &sha256)?
        .ok_or("combined pin dropped")?;
        assert_eq!(selected.algorithm, "sha512");
        assert_eq!(selected.digest, sha512);
    }
    Ok(())
}
#[test]
fn known_invalid_pins_and_unsupported_algorithms_fail_without_side_effects() -> TestResult {
    let version = Version::parse("10.0.0")?;
    for (algorithm, bytes) in [("sha256", 64), ("sha1", 40), ("sha224", 56), ("sha384", 96)] {
        let pin = format!("10.0.0+{algorithm}.{}", "b".repeat(bytes));
        for value in [
            json!({"packageManager":format!("pnpm@{pin}")}),
            json!({"devEngines":{"packageManager":dev(pin,"error")}}),
        ] {
            assert!(
                declaration(value)?
                    .locked_integrity(&version, &"a".repeat(64))
                    .is_err()
            );
        }
    }
    Ok(())
}
#[test]
fn applicable_or_branches_preserve_version_and_on_fail_rules_or_reject_ambiguity() -> TestResult {
    let version = Version::parse("10.0.0")?;
    let locked = "a".repeat(64);
    let strong = format!("10.0.0+sha512.{}", "b".repeat(128));
    let selected = declaration(json!({"devEngines":{"packageManager":[
        dev(format!("9.0.0+sha1.{}", "c".repeat(40)), "error"), dev(strong.clone(), "error")
    ]}}))?
    .locked_integrity(&version, &locked)?
    .ok_or("applicable OR pin dropped")?;
    assert_eq!(selected.algorithm, "sha512");
    for policy in ["warn", "ignore"] {
        assert!(
            declaration(
                json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":
            dev(format!("9.0.0+sha512.{}", "c".repeat(128)), policy)}})
            )?
            .locked_integrity(&version, &locked)?
            .is_none()
        );
    }
    assert!(discover_package_manager(&json!({"packageManager":"pnpm@10.0.0","devEngines":{"packageManager":dev("9.0.0".into(),"error")}})).is_err());
    for other in [
        "10.x".to_owned(),
        format!("10.0.0+sha512.{}", "c".repeat(128)),
    ] {
        assert!(declaration(json!({"devEngines":{"packageManager":[dev(strong.clone(), "error"),dev(other,"error")]}}))?
            .locked_integrity(&version, &locked).is_err());
    }
    Ok(())
}
