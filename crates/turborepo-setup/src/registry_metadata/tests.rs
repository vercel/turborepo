#![allow(clippy::unwrap_used)]

use std::error::Error as StdError;

use RegistryMetadataError::*;
use serde_json::{Value, json};

use super::*;

const SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
const SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn document(name: &str, version: &str) -> Value {
    json!({"name":name,"version":version,"dist": {
        "tarball":format!("https://registry.npmjs.org/{name}/-/{name}-{version}.tgz"),
        "integrity":format!("sha512-{}", STANDARD.encode(hex::decode(SHA512).unwrap())),
        "shasum":"legacy checksum is not a fallback"
    },"scripts":{"postinstall":"this helper must never execute it"},"unrelated":{"nested":[1,true,null]}})
}
fn parse(
    value: &Value,
    manager: Manager,
    version: &str,
    pin: Option<&CorepackIntegrity>,
) -> Result<RegistryArtifact, RegistryMetadataError> {
    parse_release(&serde_json::to_vec(value).unwrap(), manager, version, pin)
}

#[test]
fn exact_npm_and_pnpm_documents_retain_native_integrity() {
    for (manager, name, versions) in [
        (
            Manager::Npm,
            "npm",
            ["8.19.4", "10.8.2", "11.0.0", "11.1.0-beta.1"],
        ),
        (
            Manager::Pnpm,
            "pnpm",
            ["7.33.7", "9.15.9", "10.1.0", "10.0.0-alpha.1+build.2"],
        ),
    ] {
        for version in versions {
            let value = document(name, version);
            let pin = CorepackIntegrity {
                algorithm: "sha512",
                digest: SHA512.to_uppercase(),
            };
            for authored in [None, Some(&pin)] {
                let artifact = parse(&value, manager, version, authored).unwrap();
                assert_eq!(artifact.manager, manager);
                assert_eq!(artifact.version.to_string(), version);
                assert_eq!(artifact.tarball, value["dist"]["tarball"].as_str().unwrap());
                assert_eq!(
                    artifact.integrity,
                    CorepackIntegrity {
                        algorithm: "sha512",
                        digest: SHA512.into()
                    }
                );
                assert!(artifact.additional_sha256.is_none());
            }
        }
    }
}

#[test]
fn authored_sha256_is_an_additional_pin_not_a_registry_or_weak_hash_fallback() {
    let value = document("pnpm", "9.15.9");
    let pin = CorepackIntegrity {
        algorithm: "sha256",
        digest: SHA256.to_uppercase(),
    };
    let artifact = parse(&value, Manager::Pnpm, "9.15.9", Some(&pin)).unwrap();
    assert_eq!(
        artifact.additional_sha256,
        Some(CorepackIntegrity {
            algorithm: "sha256",
            digest: SHA256.into()
        })
    );
    assert_eq!(artifact.integrity.digest, SHA512);
    let wrong = CorepackIntegrity {
        algorithm: "sha512",
        digest: "0".repeat(128),
    };
    assert_eq!(
        parse(&value, Manager::Pnpm, "9.15.9", Some(&wrong)),
        Err(IntegrityConflict)
    );
    for algorithm in ["sha1", "sha224", "sha384", "SHA512", "unknown"] {
        let pin = CorepackIntegrity {
            algorithm,
            digest: SHA512.into(),
        };
        assert_eq!(
            parse(&value, Manager::Pnpm, "9.15.9", Some(&pin)),
            Err(InvalidIntegrity)
        );
    }
    for (algorithm, length) in [("sha512", 128), ("sha256", 64)] {
        for digest in [
            "".into(),
            "0".repeat(length - 1),
            "0".repeat(length + 1),
            "z".repeat(length),
            "é".repeat(length / 2),
        ] {
            let pin = CorepackIntegrity { algorithm, digest };
            assert_eq!(
                parse(&value, Manager::Pnpm, "9.15.9", Some(&pin)),
                Err(InvalidIntegrity)
            );
        }
    }
    let mut missing = value;
    missing["dist"].as_object_mut().unwrap().remove("integrity");
    assert_eq!(
        parse(&missing, Manager::Pnpm, "9.15.9", Some(&pin)),
        Err(InvalidMetadata)
    );
}

#[test]
fn names_exact_versions_and_manager_families_cannot_be_substituted() {
    let valid = document("npm", "10.8.2");
    for manager in [Manager::Yarn, Manager::Bun, Manager::Nub, Manager::Aube] {
        assert_eq!(
            parse(&valid, manager, "10.8.2", None),
            Err(UnsupportedManager)
        );
    }
    for version in [
        "",
        "latest",
        "10",
        "10.x",
        "^10.8.2",
        "~10.8.2",
        "v10.8.2",
        "10.08.2",
        " 10.8.2",
        "10.8.2 ",
        "npm:10.8.2",
        "10.8.2/../../secret",
        "9007199254740992.0.0",
    ] {
        assert_eq!(
            parse(&valid, Manager::Npm, version, None),
            Err(InvalidVersion),
            "{version}"
        );
    }
    assert_eq!(
        parse(
            &valid,
            Manager::Npm,
            &format!("1.0.0+{}", "a".repeat(256)),
            None
        ),
        Err(InvalidVersion)
    );
    for name in ["pnpm", "NPM", "@npm/cli", "../npm", "npm\0", "npm "] {
        let mut value = valid.clone();
        value["name"] = json!(name);
        assert_eq!(
            parse(&value, Manager::Npm, "10.8.2", None),
            Err(IdentityMismatch)
        );
    }
    for version in ["10.8.1", "v10.8.2", "10.8.2+metadata", "10.8.2 "] {
        let mut value = valid.clone();
        value["version"] = json!(version);
        assert_eq!(
            parse(&value, Manager::Npm, "10.8.2", None),
            Err(IdentityMismatch)
        );
    }
    assert_eq!(
        parse(&document("pnpm", "9.15.9"), Manager::Npm, "9.15.9", None),
        Err(IdentityMismatch)
    );
}

#[test]
fn canonical_urls_are_compared_before_any_url_normalization() {
    let base = "https://registry.npmjs.org/npm/-/npm-10.8.2.tgz";
    for url in [
        base.replace("https:", "http:"),
        base.replace("registry.npmjs.org", "REGISTRY.npmjs.org"),
        base.replace("registry.npmjs.org", "registry.npmjs.org:443"),
        base.replace("registry.npmjs.org", "registry.npmjs.org.evil.test"),
        base.replace("https://", "https://user:private-password@"),
        base.replace("https://", "https://@"),
        base.replace("/npm/-/", "/npm/other/../-/"),
        base.replace("/npm/-/", "/npm/%2e/-/"),
        base.replace("/npm/-/", "/npm//-/"),
        base.replace("/npm/-/", "/npm\\-/"),
        base.replace("npm-10.8.2", "npm-10.8.1"),
        base.replace("/npm/-/", "/pnpm/-/"),
        format!("{base}?token=private-query"),
        format!("{base}#private-fragment"),
        format!("{base}/"),
        "//registry.npmjs.org/npm/-/npm-10.8.2.tgz".into(),
        "file:///private-path".into(),
    ] {
        let mut value = document("npm", "10.8.2");
        value["dist"]["tarball"] = json!(url);
        assert_eq!(
            parse(&value, Manager::Npm, "10.8.2", None),
            Err(InvalidTarball)
        );
    }
}

#[test]
fn sri_requires_single_sha512_canonical_padding_and_complete_digest() {
    let good = format!("sha512-{}", STANDARD.encode([255; 64]));
    assert!(sri_sha512(&good).is_ok());
    let mut bad_bits = STANDARD.encode([0; 64]).into_bytes();
    bad_bits[85] = b'B';
    for sri in [
        "".into(),
        "sha1-legacy".into(),
        good.replace("sha512-", "SHA512-"),
        good.replace("sha512-", "sha256-"),
        good.trim_end_matches('=').into(),
        format!("{good}="),
        good.replace('/', "_"),
        good.replace('/', "-"),
        format!(" {good}"),
        format!("{good} "),
        format!("{good} sha256-other"),
        format!("{good}?options"),
        format!("sha512-{}", String::from_utf8(bad_bits).unwrap()),
        format!("sha512-{}", STANDARD.encode([0; 63])),
        format!("sha512-{}", STANDARD.encode([0; 65])),
    ] {
        let mut value = document("npm", "10.8.2");
        value["dist"]["integrity"] = json!(sri);
        assert_eq!(
            parse(&value, Manager::Npm, "10.8.2", None),
            Err(InvalidIntegrity)
        );
    }
}

#[test]
fn release_and_distribution_must_be_objects_not_positional_arrays() {
    let value = document("npm", "10.8.2");
    let url = value["dist"]["tarball"].clone();
    let sri = value["dist"]["integrity"].clone();
    for bad in [
        json!(["npm", "10.8.2", [url, sri]]),
        json!(["npm", "10.8.2", value["dist"]]),
        json!({"name":"npm", "version":"10.8.2", "dist":[url, sri]}),
        json!({"name":"npm", "version":"10.8.2", "dist":null}),
        json!({"name":"npm", "version":"10.8.2", "dist":"private-body"}),
    ] {
        assert_eq!(
            parse(&bad, Manager::Npm, "10.8.2", None),
            Err(InvalidMetadata)
        );
    }
}

#[test]
fn byte_bounds_malformed_duplicate_fields_and_redaction() {
    let raw = serde_json::to_vec(&document("npm", "10.8.2")).unwrap();
    for length in 0..raw.len() {
        assert!(parse_release(&raw[..length], Manager::Npm, "10.8.2", None).is_err());
    }
    let mut exact = raw.clone();
    exact.resize(MAX_METADATA_BYTES, b' ');
    assert!(parse_release(&exact, Manager::Npm, "10.8.2", None).is_ok());
    exact.push(b' ');
    assert_eq!(
        parse_release(&exact, Manager::Npm, "10.8.2", None),
        Err(TooLarge)
    );
    for (old, new) in [
        ("\"name\":\"npm\"", "\"name\":\"npm\",\"name\":\"pnpm\""),
        (
            "\"version\":\"10.8.2\"",
            "\"version\":\"10.8.2\",\"version\":\"10.8.2\"",
        ),
        (
            "\"integrity\":",
            "\"integrity\":\"sha1-weak\",\"integrity\":",
        ),
        (
            "\"tarball\":",
            "\"tarball\":\"https://evil.test/secret\",\"tarball\":",
        ),
        ("\"dist\":", "\"dist\":{},\"dist\":"),
    ] {
        let bad = String::from_utf8(raw.clone()).unwrap().replace(old, new);
        assert_eq!(
            parse_release(bad.as_bytes(), Manager::Npm, "10.8.2", None),
            Err(InvalidMetadata)
        );
    }
    for key in ["name", "version", "dist"] {
        let mut value = document("npm", "10.8.2");
        value.as_object_mut().unwrap().remove(key);
        assert_eq!(
            parse(&value, Manager::Npm, "10.8.2", None),
            Err(InvalidMetadata)
        );
    }
    let mut bad = raw;
    bad.extend(b" private-body");
    let error = parse_release(&bad, Manager::Npm, "10.8.2", None)
        .err()
        .unwrap();
    assert_eq!(error, InvalidMetadata);
    let diagnostic = format!("{error} {error:?}");
    for secret in [
        "private-body",
        "private-query",
        "private-password",
        "registry.npmjs.org",
        SHA512,
    ] {
        assert!(!diagnostic.contains(secret));
    }
    assert!(error.source().is_none());
}
