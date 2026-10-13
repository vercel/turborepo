use serde_json::json;

use super::*;
use crate::version_request::MAX_REQUEST_BYTES;

type TestResult = Result<(), Box<dyn std::error::Error>>;

macro_rules! assert_versions {
    ($d:expr; $($v:expr => $expected:expr),+ $(,)?) => {$(
        assert_eq!($d.matches(&Version::parse($v)?), $expected, $v);
    )+};
}

fn discover(value: Value) -> Result<Declaration, Box<dyn std::error::Error>> {
    discover_package_manager(&value)?.ok_or_else(|| "missing declaration".into())
}

fn dev(version: &str) -> Value {
    json!({"devEngines": {"packageManager": {"name": "pnpm", "version": version}}})
}

fn both(top: &str, version: &str) -> Value {
    let mut value = dev(version);
    value["packageManager"] = json!(top);
    value
}

fn invalid_version(version: &str, message: &str) {
    invalid_at(dev(version), &format!("{DEV}/version"), message);
}

fn invalid_at(value: Value, pointer: &str, message: &str) {
    let Err(error) = discover_package_manager(&value) else {
        panic!("expected invalid declaration");
    };
    assert_eq!(error.sources, [pointer]);
    assert!(error.message.contains(message), "{error}");
    assert!(error.to_string().contains(pointer));
}

#[test]
fn discovers_explicit_exact_and_ranged_declarations() -> TestResult {
    for name in ["npm", "pnpm"] {
        for version in ["9.12.3", "^9.0.0", "9 || 10", ">=9 <11", "9.x"] {
            for value in [
                json!({"packageManager": format!("{name}@{version}")}),
                json!({"devEngines": {"packageManager": {"name": name, "version": version}}}),
            ] {
                let d = discover(value)?;
                assert_eq!(d.manager.name(), name);
                assert_versions!(d; "9.12.3" => true, "11.0.0" => false);
            }
        }
    }
    for value in [json!({}), json!({"devEngines": {"runtime": {}}})] {
        assert!(discover_package_manager(&value)?.is_none());
    }
    let d = discover(json!({"packageManager": "pnpm@ = v9.12.3+build.42 "}))?;
    assert!(!d.matches(&Version::new(9, 12, 3)));
    let top = d.package_manager.ok_or("missing pin")?;
    assert_eq!(top.version.as_deref(), Some("9.12.3+build.42"));
    assert!(top.integrity.is_none());
    let constraint = top.request.as_ref().ok_or("missing version")?;
    assert!(constraint.matches(&Version::new(9, 12, 3)));
    for range in ["9.0.0", "=9.0.0 *", "9.0.0+two"] {
        let d = discover(both("pnpm@9.0.0+build.42", range))?;
        assert_versions!(d; "9.0.0+build.42" => true, "9.0.0" => false);
        assert_eq!(
            d.package_manager.ok_or("missing pin")?.version.as_deref(),
            Some("9.0.0+build.42")
        );
    }
    Ok(())
}

#[test]
fn integrity_lengths_hex_and_build_identity_are_exact() -> TestResult {
    for (algorithm, length) in [
        ("sha1", 40),
        ("sha224", 56),
        ("sha256", 64),
        ("sha384", 96),
        ("sha512", 128),
    ] {
        for build in ["", "+build.42", "+shared.42"] {
            let version = format!("9.12.3{build}+{algorithm}.{}", "AB".repeat(length / 2));
            for value in [
                json!({"packageManager": format!("pnpm@{version}")}),
                dev(&version),
            ] {
                let d = discover(value)?;
                let r = d
                    .package_manager
                    .as_ref()
                    .or_else(|| d.dev_engines.first())
                    .ok_or("missing request")?;
                let version = format!("9.12.3{build}");
                assert_eq!(r.version.as_deref(), Some(version.as_str()));
                let integrity = r.integrity.as_ref().ok_or("missing integrity")?;
                assert_eq!(integrity.algorithm, algorithm);
                assert_eq!(integrity.digest, "ab".repeat(length / 2));
                let constraint = r.request.as_ref().ok_or("missing version")?;
                assert_eq!(constraint.exact_version(), Some(&Version::parse(&version)?));
            }
        }
        for digest in [
            "a".repeat(length - 1),
            "a".repeat(length + 1),
            format!("{}g", "a".repeat(length - 1)),
            format!("{}.extra", "a".repeat(length)),
        ] {
            invalid_version(&format!("9.12.3+{algorithm}.{digest}"), "exactly");
        }
        invalid_version(
            &format!("^9.12.3+{algorithm}.{}", "a".repeat(length)),
            "single exact",
        );
    }
    for suffix in [
        "sha1.abc",
        "sha999.abc",
        "SHA256.abc",
        "md5.abc",
        "blake2b.abc",
        "sha256",
        "sha256.",
    ] {
        invalid_version(
            &format!("9.12.3+{suffix}"),
            if suffix.starts_with("sha256") || suffix.starts_with("sha1.") {
                "exactly"
            } else {
                "unsupported Corepack"
            },
        );
    }
    Ok(())
}

#[test]
fn checksums_cannot_hide_in_ranges_or_multiple_suffixes() -> TestResult {
    let checksum = format!("sha256.{}", "a".repeat(64));
    let second = format!("sha512.{}", "b".repeat(128));
    for suffix in [checksum.as_str(), "sha999.abc", "SHA256.abc", "md5.abc"] {
        for version in [
            format!("9.0.0+{suffix} || 10.0.0+build.42"),
            format!("9.0.0+{suffix} >=9.0.0+build.42"),
            format!("9.0.0+{suffix}+{second}"),
        ] {
            invalid_version(&version, "Corepack");
            invalid_at(
                json!({"packageManager": format!("pnpm@{version}")}),
                TOP,
                "Corepack",
            );
        }
    }
    let d = discover(dev("9.0.0+shared.42 || 10.0.0+build.42"))?;
    assert_versions!(d; "9.0.0" => true, "10.0.0" => true, "11.0.0" => false);
    Ok(())
}

#[test]
fn native_arrays_retain_or_semantics_and_failure_policy() -> TestResult {
    let cases: Vec<(Value, &str)> = serde_json::from_str(
        r#"[
        [{"name":"pnpm","version":"^9"}, "error"],
        [{"name":"pnpm","version":"^9","onFail":"warn"}, "warn"],
        [{"name":"pnpm","version":"^9","onFail":"ignore"}, "ignore"],
        [[{"name":"pnpm","version":"^9"}], "error"],
        [[{"name":"pnpm","version":"^9","onFail":"error"},{"name":"pnpm","version":"10.1.0"}], "error"],
        [[{"name":"pnpm","version":"^9","onFail":"warn"},{"name":"pnpm","version":"10.1.0"}], "error"],
        [[{"name":"pnpm","version":"^9","onFail":"ignore"},{"name":"pnpm","version":"10.1.0"}], "error"],
        [[{"name":"pnpm","version":"^9","onFail":"error"},{"name":"pnpm","version":"10.1.0","onFail":"warn"}], "warn"],
        [[{"name":"pnpm","version":"^9","onFail":"error"},{"name":"pnpm","version":"10.1.0","onFail":"ignore"}], "ignore"],
        [[{"name":"pnpm","version":"^9","onFail":"warn"},{"name":"pnpm","version":"10.1.0","onFail":"ignore"}], "ignore"],
        [[{"name":"pnpm","version":"^9","onFail":"error"},{"name":"pnpm","version":"10.1.0","onFail":"warn"},{"name":"pnpm","version":"10.2.0","onFail":"ignore"}], "ignore"]
    ]"#,
    )?;
    for (group, policy) in cases {
        let entries = group
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(&group));
        let mut root = json!({"devEngines": {"packageManager": group}});
        let d = discover(root.clone())?;
        assert_eq!(d.precedence, Precedence::DevEngines);
        assert_eq!(d.dev_engines.len(), entries.len());
        for (i, (r, entry)) in d.dev_engines.iter().zip(entries).enumerate() {
            assert_eq!(r.version.as_deref(), entry["version"].as_str());
            let suffix = if group.is_array() {
                format!("/{i}")
            } else {
                String::new()
            };
            assert_eq!(r.source, format!("{DEV}{suffix}"));
            assert_eq!(
                format!("{:?}", r.on_fail).to_lowercase(),
                entry["onFail"].as_str().unwrap_or("error")
            );
        }
        assert_versions!(d; "9.2.0" => true, "10.1.0" => entries.len() > 1, "10.2.0" => entries.len() > 2);
        root["packageManager"] = json!("pnpm@11.0.0");
        let result = discover_package_manager(&root);
        if policy == "error" {
            assert!(result.is_err());
        } else {
            let d = result?.ok_or("missing declaration")?;
            assert_eq!(d.warnings.len(), usize::from(policy == "warn"));
            assert_versions!(d; "11.0.0" => true, "9.2.0" => false);
            for warning in d.warnings {
                assert_eq!(warning.sources.len(), entries.len() + 1);
                assert!(warning.to_string().contains(TOP) && warning.to_string().contains(DEV));
            }
        }
        root["packageManager"] = json!("pnpm@>=9 <12");
        let d = discover(root)?;
        assert_eq!(d.precedence, Precedence::Manager);
        assert_eq!(d.dev_engines.len(), entries.len());
        assert_versions!(d; "11.0.0" => policy != "error", "9.2.0" => true, "12.0.0" => false);
    }
    Ok(())
}

#[test]
fn malformed_fields_do_not_fallback_or_weaken_constraints() -> TestResult {
    // (input pointer, diagnostic pointer, message, variants); every field is
    // checked both with and without the other declaration, never falling back.
    let groups: Vec<(&str, &str, &str, Vec<Value>)> = serde_json::from_str(
        r#"[
        ["", "", "object", [null]],
        ["/packageManager", "/packageManager", "", [null, 42, "pnpm", "pnpm@", "pnpm@latest"]],
        ["/devEngines", "/devEngines", "object", [null, [], "pnpm"]],
        ["/devEngines/packageManager", "/devEngines/packageManager", "", [null, 42, []]],
        ["/devEngines/packageManager", "/devEngines/packageManager/0", "", [[null]]],
        ["/devEngines/packageManager", "/devEngines/packageManager/0/name", "", [[{}]]],
        ["/devEngines/packageManager/name", "/devEngines/packageManager/name", "", [null, 42, "", " pnpm", "PNPM", "yarn"]],
        ["/devEngines/packageManager/version", "/devEngines/packageManager/version", "", [null, 42, "", "latest", "01.2.3", "9 ||", ">2 <1"]],
        ["/devEngines/packageManager/onFail", "/devEngines/packageManager/onFail", "", [null, 42, "", "download", "Warn", []]],
        ["/devEngines/packageManager/version", "/devEngines/packageManager/version", "unsupported", ["https://example.com/pnpm.tgz", "file:pnpm.tgz", "npm:pnpm@9", "../pnpm.tgz", "git+ssh://example.com/pnpm"]],
        ["/packageManager", "/packageManager", "unsupported", ["pnpm@https://example.com/pnpm.tgz", "pnpm@file:pnpm.tgz", "pnpm@npm:pnpm@9", "pnpm@../pnpm.tgz", "pnpm@git+ssh://example.com/pnpm"]],
        ["/packageManager", "/packageManager", "unsupported manager", ["bun@1.2.3"]]
    ]"#,
    )?;
    for (path, location, message, variants) in groups {
        for value in variants {
            let mut root = both("pnpm@9.0.0", "9");
            root["devEngines"]["packageManager"]["onFail"] = json!("error");
            *root.pointer_mut(path).ok_or("missing fixture path")? = value;
            invalid_at(root.clone(), &format!("package.json#{location}"), message);
            if !path.is_empty() {
                let key = if path == "/packageManager" {
                    "devEngines"
                } else {
                    "packageManager"
                };
                root.as_object_mut().ok_or("fixture object")?.remove(key);
                invalid_at(root, &format!("package.json#{location}"), message);
            }
        }
    }
    for (version, release, allowed) in [
        (None, "9.0.0", true),
        (None, "9.0.0-beta+build.42", true),
        (Some("*"), "9.0.0", true),
        (Some("*"), "9.0.0-beta+build.42", false),
    ] {
        let mut root = json!({"devEngines": {"packageManager": {"name": "pnpm"}}});
        if let Some(version) = version {
            root["devEngines"]["packageManager"]["version"] = json!(version);
        }
        let d = discover(root.clone())?;
        assert_eq!(d.dev_engines[0].version.as_deref(), version);
        assert_eq!(d.dev_engines[0].request.is_some(), version.is_some());
        let release_version = Version::parse(release)?;
        assert_eq!(d.matches(&release_version), allowed);
        root["packageManager"] = json!(format!("pnpm@{release}"));
        assert_eq!(discover_package_manager(&root).is_ok(), allowed);
        if allowed {
            assert!(discover(root)?.matches(&release_version));
        }
    }
    Ok(())
}

#[test]
fn name_only_declarations_validate_all_candidates() -> TestResult {
    for entry in [json!({"name": "pnpm"}), json!([{"name": "pnpm"}])] {
        let d = discover(json!({"devEngines": {"packageManager": entry}}))?;
        assert!(d.dev_engines[0].version.is_none());
        assert!(d.dev_engines[0].request.is_none());
        assert_versions!(d; "9.0.0" => true, "9.0.0-rc.1+build.42" => true);
        let unsafe_component = 9_007_199_254_740_992;
        for [major, minor, patch] in [
            [unsafe_component, 0, 0],
            [0, unsafe_component, 0],
            [0, 0, unsafe_component],
        ] {
            assert!(!d.matches(&Version::new(major, minor, patch)));
        }
        for (bytes, allowed) in [(256, true), (257, false)] {
            for suffix in [
                format!("-{}", "a".repeat(bytes - 6)),
                format!("+{}", "a".repeat(bytes - 6)),
                format!("-{}+{}", "a".repeat(124), "b".repeat(bytes - 131)),
            ] {
                let release = Version::parse(&format!("1.2.3{suffix}"))?;
                assert_eq!(release.to_string().len(), bytes);
                assert_eq!(d.matches(&release), allowed);
            }
        }
    }
    Ok(())
}

#[test]
fn contradictions_report_all_sources_including_build_and_integrity() -> TestResult {
    let digest = "a".repeat(64);
    let mismatched_digest = "b".repeat(64);
    for (top, version) in [
        ("npm@9.0.0".to_owned(), "9.0.0".to_owned()),
        ("pnpm@9.0.0".to_owned(), "^10".to_owned()),
        ("pnpm@^9".to_owned(), ">=10 <11".to_owned()),
        (
            format!("pnpm@9.0.0+sha256.{digest}"),
            format!("9.0.0+sha256.{mismatched_digest}"),
        ),
    ] {
        let error = discover_package_manager(&both(&top, &version))
            .err()
            .ok_or("expected conflict")?;
        assert_eq!(error.sources, [TOP, DEV]);
        assert!(error.to_string().contains(TOP));
        assert!(error.to_string().contains(DEV));
    }
    let d = discover(both("pnpm@>=9 <11", ">=10 <12"))?;
    assert_versions!(d; "9.0.0" => false, "10.0.0" => true, "11.0.0" => false);
    let d = discover(both(
        &format!("pnpm@9.0.0+sha256.{digest}"),
        &format!("9.0.0+sha512.{}", "c".repeat(128)),
    ))?;
    let pin = d.package_manager.ok_or("missing top")?;
    assert!(pin.integrity.is_some());
    assert!(d.dev_engines[0].integrity.is_some());
    let mixed = json!({"devEngines": {"packageManager": [{"name": "npm", "version": "9"}, {"name": "pnpm", "version": "9"}]}});
    for manager in ["npm", "pnpm"] {
        let mut root = mixed.clone();
        root["packageManager"] = json!(format!("{manager}@9.0.0"));
        let d = discover(root)?;
        assert_eq!(d.manager.name(), manager);
        assert!(d.matches(&Version::new(9, 0, 0)));
        assert!(d.warnings.is_empty());
    }
    let error = discover_package_manager(&mixed)
        .err()
        .ok_or("mixed managers")?;
    assert!(error.message.contains("ambiguous"));
    assert_eq!(error.sources, vec![format!("{DEV}/0"), format!("{DEV}/1")]);
    Ok(())
}

#[test]
fn range_overlap_preserves_branches_prerelease_gates_and_safe_bounds() -> TestResult {
    for (a, b, expected) in [
        ("^9", "10", false),
        ("^9 || ^10", "10", true),
        (">9.0.0 <9.0.1", "*", false),
        (">9.0.0 <=9.0.1", "*", true),
        (">9.0.0-alpha <9.0.0", "*", false),
        (">9.0.0-alpha <9.0.0", ">=9.0.0-alpha.0 <9.0.0", true),
        (">=0.0.0 <0.0.0 || >=1.0.0-alpha <1", "*", false),
        (
            ">9007199254740991.9007199254740991.9007199254740991",
            "*",
            false,
        ),
        (">1.9007199254740991.9007199254740991", "2.0.0", true),
    ] {
        let a = VersionRequest::parse(a)?;
        let b = VersionRequest::parse(b)?;
        assert_eq!(a.intersects(&b), expected);
        assert_eq!(b.intersects(&a), expected);
    }
    Ok(())
}

#[test]
fn alternative_and_request_limits_are_inclusive() -> TestResult {
    let entry = json!({"name": "pnpm", "version": "*"});
    for count in [MAX_ALTERNATIVES, MAX_ALTERNATIVES + 1] {
        let root = json!({"devEngines": {"packageManager": vec![entry.clone(); count]}});
        assert_eq!(
            discover_package_manager(&root).is_ok(),
            count == MAX_ALTERNATIVES
        );
    }
    let version = format!(
        "{}9.0.0+sha512.{}",
        " ".repeat(MAX_REQUEST_BYTES - 5),
        "a".repeat(128)
    );
    assert!(discover(dev(&version))?.dev_engines[0].integrity.is_some());
    invalid_version(&format!(" {version}"), "limit");
    for manager in ["npm", "pnpm"] {
        let top = format!("{manager}@{version}");
        assert!(
            discover(json!({"packageManager": top.clone()}))?
                .package_manager
                .ok_or("missing pin")?
                .integrity
                .is_some()
        );
        invalid_at(json!({"packageManager": format!("{top} ")}), TOP, "limit");
    }
    invalid_version(&format!("{}*", " ".repeat(MAX_REQUEST_BYTES)), "limit");
    Ok(())
}

#[test]
fn shared_families_do_not_expand_setup_capabilities() -> TestResult {
    for family in Manager::ALL {
        let supported = matches!(family, Manager::Npm | Manager::Pnpm);
        let top = json!({"packageManager": format!("{}@9.0.0", family.name())});
        if supported {
            let shared: turborepo_package_manager::Family = discover(top)?.manager;
            assert_eq!(shared, family);
            continue;
        }
        invalid_at(top, TOP, "unsupported manager; setup supports npm and pnpm");
        // Unsupported capabilities win over malformed versions and policies,
        // even with an authoritative pin or an ignored alternative.
        for version in [json!(null), json!(42), json!("latest")] {
            for policy in [json!("ignore"), json!(null)] {
                let entry = json!({"name": family.name(), "version": version, "onFail": policy});
                for (entries, pointer) in [
                    (entry.clone(), format!("{DEV}/name")),
                    (json!([entry]), format!("{DEV}/0/name")),
                ] {
                    let mut root = json!({"devEngines": {"packageManager": entries}});
                    invalid_at(root.clone(), &pointer, "unsupported manager");
                    root["packageManager"] = json!("pnpm@9.0.0");
                    invalid_at(root, &pointer, "unsupported manager");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn shared_entry_adapter_preserves_validation_order() -> TestResult {
    for (version, pointer, message) in [
        (json!(null), format!("{DEV}/version"), "required string"),
        (
            json!("latest"),
            format!("{DEV}/onFail"),
            "expected error, warn, or ignore",
        ),
        (
            json!("x".repeat(MAX_VERSION_FIELD_BYTES + 1)),
            format!("{DEV}/onFail"),
            "expected error, warn, or ignore",
        ),
    ] {
        let root = json!({"devEngines": {"packageManager": {
            "name": "pnpm", "version": version, "onFail": null
        }}});
        let error = discover_package_manager(&root)
            .err()
            .ok_or("expected error")?;
        assert_eq!(error.sources, [pointer]);
        assert_eq!(error.message, message);
        let mut root = root;
        root["packageManager"] = json!("pnpm");
        invalid_at(root, TOP, "expected name@version string");
    }
    invalid_at(
        json!({"devEngines": {"packageManager": [
            {"name": "pnpm", "version": "latest"}, {"name": null}
        ]}}),
        &format!("{DEV}/0/version"),
        "",
    );
    invalid_at(
        json!({"devEngines": {"packageManager": vec![json!(null); MAX_ALTERNATIVES + 1]}}),
        DEV,
        "expected 1..=32 package-manager alternatives",
    );
    Ok(())
}

#[test]
fn diagnostics_are_opaque_and_raw_fields_are_bounded() -> TestResult {
    let private = "private-input";
    for value in [
        json!({"packageManager": format!("https://{private}:{private}@example.invalid/pm")}),
        json!({"devEngines": {"packageManager": {"name": private}}}),
        json!({"devEngines": {"packageManager": [{"name": private}]}}),
        dev(&format!("1.2.3+sha999-{private}.abc")),
    ] {
        let error = discover_package_manager(&value)
            .err()
            .ok_or("expected invalid declaration")?;
        assert!(!format!("{error} {error:?}").contains(private));
    }
    invalid_version(
        &format!("1.2.3+sha{}", "9".repeat(MAX_REQUEST_BYTES + 136)),
        "limit",
    );
    invalid_at(
        json!({"packageManager": "x".repeat(MAX_REQUEST_BYTES + 142)}),
        TOP,
        "limit",
    );
    Ok(())
}
