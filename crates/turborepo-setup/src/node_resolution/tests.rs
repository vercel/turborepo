use semver::Version;
use serde_json::{Value, json};

use super::*;
use crate::{NodeArtifact, node_provision::NodePlan};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn entry(version: &str) -> Value {
    json!({"version":format!("v{version}"), "lts":"Krypton", "npm":"11.6.1",
        "files":["osx-x64-tar","osx-arm64-tar","linux-x64","linux-arm64",
        "win-x64-zip","win-arm64-zip"]})
}
fn index(entries: &[Value]) -> Vec<u8> {
    serde_json::to_vec(entries).unwrap()
}
fn sums(version: &str) -> String {
    let version = Version::parse(version).unwrap();
    PLATFORMS
        .iter()
        .map(|&platform| {
            let (target, _) = lock_target(platform).unwrap();
            let official = NodeArtifact::for_platform(&version, target).unwrap();
            format!("{}  {}\n", "A".repeat(64), official.filename())
        })
        .collect()
}
fn selection(
    requirements: &NodeRequirements,
    entries: &[Value],
    version: &str,
    bundled: bool,
) -> Result<ToolResolution, Error> {
    resolve(
        requirements,
        &index(entries),
        ChecksumManifest {
            version,
            bytes: sums(version).as_bytes(),
        },
        bundled,
    )
}
fn lock(tool: Tool) -> Lock {
    Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("node".into(), tool)]),
    })
    .unwrap()
}

#[test]
fn native_aliases_ranges_exact_and_latest_stable_semantics_are_reused() -> TestResult {
    let mut entries: Vec<_> = ["22.1.0", "24.0.0", "24.1.0", "25.0.0", "26.0.0-rc.1"]
        .into_iter()
        .map(entry)
        .collect();
    entries[3]["lts"] = json!(false);
    entries[4]["lts"] = json!(false);
    for (request, expected) in [
        ("node", "25.0.0"),
        ("lts/*", "24.1.0"),
        ("lts/KRYPTON", "24.1.0"),
        ("^24.0.0", "24.1.0"),
        ("24.x", "24.1.0"),
        (">=22 <25", "24.1.0"),
        (" v24.0.0\n", "24.0.0"),
        ("26.0.0-rc.1", "26.0.0-rc.1"),
    ] {
        let requirements = NodeRequirements::from_sources(None, Some(request), None)?;
        let resolved = selection(&requirements, &entries, expected, false)?;
        assert_eq!(resolved.native().version.to_string(), expected);
        assert_eq!(
            resolved.native().sources,
            requirements.sources().cloned().collect::<Vec<_>>()
        );
        assert_eq!(resolved.tool().version, expected);
        let mut reversed = entries.clone();
        reversed.reverse();
        assert_eq!(
            resolved.tool(),
            selection(&requirements, &reversed, expected, false)?.tool()
        );
    }
    let exact = NodeRequirements::from_sources(None, Some("24.0.0+vendor.1"), None)?;
    let built = selection(
        &exact,
        &[entry("24.0.0"), entry("24.0.0+vendor.1")],
        "24.0.0+vendor.1",
        false,
    )?;
    assert_eq!(built.tool().version, "24.0.0+vendor.1");
    Ok(())
}

#[test]
fn native_priority_raw_ranges_and_runtime_policy_survive_lock_serialization() -> TestResult {
    let manifest = r#"{"devEngines":{"runtime":[{"name":"node","version":"22.x","onFail":"error"},{"name":"node","version":"24.x","onFail":"error"}]},"engines":{"node":" >=22.0.0 <25 "}}"#;
    let requirements =
        NodeRequirements::from_sources(Some(manifest), Some(" lts/KRYPTON\n"), Some("24.x"))?;
    let resolved = selection(
        &requirements,
        &[entry("22.1.0"), entry("24.1.0"), entry("25.0.0")],
        "24.1.0",
        false,
    )?;
    assert_eq!(
        resolved.native().selection_source.field.as_deref(),
        Some("devEngines.runtime[1].version")
    );
    assert_eq!(
        resolved.native().sources[2].request.as_deref(),
        Some("lts/krypton")
    );
    assert_eq!(
        resolved.native().sources[4].request.as_deref(),
        Some(">=22.0.0 <25")
    );
    let policy = &resolved.native().sources[5..];
    assert_eq!(policy.len(), 2);
    assert!(policy.iter().all(|s| s.request.as_deref() == Some("error")));
    let locked = lock(resolved.clone().into_tool());
    assert_eq!(locked.tools()["node"].declarations.len(), 7);
    let bytes = locked.canonical_bytes()?;
    assert_eq!(bytes, Lock::parse(&bytes)?.canonical_bytes()?);
    // Canonical serialization sorts locations, but the result retains priority.
    assert_eq!(
        resolved.native().selection_source.field.as_deref(),
        Some("devEngines.runtime[1].version")
    );
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("package.json"), manifest)?;
    std::fs::write(root.path().join(".nvmrc"), " lts/KRYPTON\n")?;
    std::fs::write(root.path().join(".node-version"), "24.x")?;
    assert!(locked.matches_native(&lock::probe_native(root.path())?)?);
    Ok(())
}

#[test]
fn all_six_exact_artifacts_are_consumable_without_claiming_promotion_support() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("24.x"), None)?;
    for bundled in [false, true] {
        let resolved = selection(&requirements, &[entry("24.1.0")], "24.1.0", bundled)?;
        let locked = lock(resolved.into_tool());
        let tool = &locked.tools()["node"];
        assert_eq!(
            tool.options.get(BUNDLED_NPM),
            bundled.then(|| vec!["11.6.1".into()]).as_ref()
        );
        let Installation::Managed { artifacts } = &tool.installation else {
            panic!("managed")
        };
        assert_eq!(artifacts.len(), 6);
        for platform in PLATFORMS {
            let (target, _) = lock_target(platform).ok_or("target")?;
            let windows = target.os() == OperatingSystem::Windows;
            let official = NodeArtifact::for_platform(&Version::new(24, 1, 0), target)?;
            let artifact = &artifacts[&platform]["distribution"];
            assert_eq!(artifact.url, official.url());
            assert_eq!(artifact.sha256, "a".repeat(64));
            assert_eq!(
                artifact.format,
                if windows { Format::Zip } else { Format::TarGz }
            );
            assert_eq!(
                artifact.root_prefix.as_deref(),
                official
                    .filename()
                    .strip_suffix(if windows { ".zip" } else { ".tar.gz" })
            );
            assert!(artifact.destination.is_none());
            assert_eq!(
                artifact.executables["node"],
                if windows { "node.exe" } else { "bin/node" }
            );
            assert_eq!(artifact.executables.len(), if bundled { 3 } else { 1 });
            if bundled {
                assert_eq!(
                    artifact.executables["npm"],
                    if windows { "npm.cmd" } else { "bin/npm" }
                );
                assert_eq!(
                    artifact.executables["npx"],
                    if windows { "npx.cmd" } else { "bin/npx" }
                );
            }
            let plan = NodePlan::from_lock(&locked, platform)?;
            assert_eq!(plan.inventory_tool().executables, artifact.executables);
        }
        for unsupported in [
            Platform::Any,
            Platform::LinuxX64Musl,
            Platform::LinuxArm64Musl,
        ] {
            assert!(!artifacts.contains_key(&unsupported));
            assert!(NodePlan::from_lock(&locked, unsupported).is_err());
        }
    }
    Ok(())
}

#[test]
fn availability_and_manifest_binding_never_fall_back_or_return_partial_selection() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("24.x"), None)?;
    let mut latest = entry("24.1.0");
    latest["files"] = json!(["linux-x64"]);
    let resolved = selection(
        &requirements,
        &[entry("24.0.0"), latest.clone()],
        "24.1.0",
        false,
    )?;
    let Installation::Managed { artifacts } = &resolved.tool().installation else {
        panic!("managed")
    };
    assert_eq!(
        artifacts.keys().copied().collect::<Vec<_>>(),
        [Platform::LinuxX64Gnu]
    );
    assert_eq!(resolved.tool().version, "24.1.0");
    let bytes = index(&[latest.clone()]);
    for version in ["24.0.0", "v24.1.0", "24.x", "24.1.0\0", "24.1.0 "] {
        assert!(matches!(
            resolve(
                &requirements,
                &bytes,
                ChecksumManifest {
                    version,
                    bytes: sums("24.1.0").as_bytes()
                },
                false
            ),
            Err(Error::ManifestVersion)
        ));
    }
    for checksum in [
        sums("24.0.0"),
        format!("{}  node-v24.1.0-linux-x64.zip\n", "a".repeat(64)),
    ] {
        assert!(matches!(
            resolve(
                &requirements,
                &bytes,
                ChecksumManifest {
                    version: "24.1.0",
                    bytes: checksum.as_bytes()
                },
                false
            ),
            Err(Error::Metadata(node_metadata::Error::MissingChecksum))
        ));
    }
    latest["files"] = json!([]);
    assert!(matches!(
        selection(&requirements, &[entry("24.0.0"), latest], "24.1.0", false),
        Err(Error::Lock(_))
    ));
    // One missing advertised checksum invalidates the whole portable result.
    assert!(matches!(
        resolve(
            &requirements,
            &index(&[entry("24.1.0")]),
            ChecksumManifest {
                version: "24.1.0",
                bytes: format!("{}  node-v24.1.0-linux-x64.tar.gz\n", "a".repeat(64)).as_bytes()
            },
            false
        ),
        Err(Error::Metadata(node_metadata::Error::MissingChecksum))
    ));
    Ok(())
}

#[test]
fn caller_composition_preserves_managers_and_rejects_bundled_ownership_collisions() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("24.x"), None)?;
    let mut tools = BTreeMap::new();
    for (id, version) in [("npm", "10.9.0"), ("pnpm", "10.18.0")] {
        tools.insert(
            id.into(),
            Tool {
                adapter: id.into(),
                version: version.into(),
                declarations: vec![Declaration {
                    file: "package.json".into(),
                    field: Some(format!("/{id}")),
                    request: Some(version.into()),
                }],
                options: BTreeMap::new(),
                installation: Installation::VerifySystem {
                    executables: vec![id.into()],
                },
            },
        );
    }
    let previous = tools.clone();
    tools.insert(
        "node".into(),
        selection(&requirements, &[entry("24.1.0")], "24.1.0", false)?.into_tool(),
    );
    let combined = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: tools.clone(),
    })?;
    for id in ["npm", "pnpm"] {
        assert_eq!(combined.tools()[id], previous[id]);
    }
    tools.insert(
        "node".into(),
        selection(&requirements, &[entry("24.1.0")], "24.1.0", true)?.into_tool(),
    );
    assert!(
        Lock::new(Document {
            schema_version: lock::SCHEMA_VERSION,
            tools
        })
        .is_err()
    );
    for npm in [json!(null), json!("")] {
        let mut value = entry("24.1.0");
        value["npm"] = npm;
        assert!(selection(&requirements, &[value.clone()], "24.1.0", false).is_ok());
        assert!(matches!(
            selection(&requirements, &[value], "24.1.0", true),
            Err(Error::MissingBundledNpm)
        ));
    }
    Ok(())
}

#[test]
fn malformed_native_snapshots_match_native_reader_failures_without_fallback() -> TestResult {
    for (package, nvmrc, node_version) in [
        (Some("[]"), Some("24.x"), None),
        (Some("null"), Some("24.x"), None),
        (Some("{broken"), Some("24.x"), None),
        (
            Some(r#"{"engines":{"node":"24.x","node":"22.x"}}"#),
            Some("24.x"),
            None,
        ),
        (Some(r#"{"engines":{"node":null}}"#), Some("24.x"), None),
        (
            Some(
                r#"{"devEngines":{"runtime":{"name":"node","version":"24.x","onFail":"ignore"}}}"#,
            ),
            Some("24.x"),
            None,
        ),
        (
            Some(r#"{"devEngines":{"runtime":{"name":"node","version":"24.x","onFail":"warn"}}}"#),
            Some("24.x"),
            None,
        ),
        (
            Some(r#"{"engines":{"node":"24.x\u0000"}}"#),
            Some("24.x"),
            None,
        ),
        (None, Some("24.x\0"), None),
        (None, Some("node"), Some("lts/*")),
        (None, Some("24.x"), Some("https://secret.example/token")),
    ] {
        let injected = NodeRequirements::from_sources(package, nvmrc, node_version).unwrap_err();
        let root = tempfile::tempdir()?;
        for (name, value) in [
            ("package.json", package),
            (".nvmrc", nvmrc),
            (".node-version", node_version),
        ] {
            if let Some(value) = value {
                std::fs::write(root.path().join(name), value)?;
            }
        }
        assert_eq!(
            injected.to_string(),
            NodeRequirements::read(root.path()).unwrap_err().to_string()
        );
    }
    let absent = NodeRequirements::from_sources(None, None, None)?;
    assert!(matches!(
        selection(&absent, &[entry("24.1.0")], "24.1.0", false),
        Err(Error::Discovery(NodeDiscoveryError::NoSelection))
    ));
    let conflict = NodeRequirements::from_sources(None, Some("22.x"), Some("24.x"))?;
    assert!(matches!(
        selection(&conflict, &[entry("24.1.0")], "24.1.0", false),
        Err(Error::Discovery(
            NodeDiscoveryError::NoMatchingRelease { .. }
        ))
    ));
    Ok(())
}

#[test]
fn ambiguous_metadata_and_all_input_bounds_fail_closed() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("24.x"), None)?;
    for bytes in [
        index(&[entry("24.1.0"), entry("24.1.0")]),
        br#"[{"version":"v24.1.0","version":"v22.0.0","files":[]}]"#.to_vec(),
        vec![0xff],
        vec![0; node_metadata::MAX_INDEX_BYTES + 1],
    ] {
        assert!(
            resolve(
                &requirements,
                &bytes,
                ChecksumManifest {
                    version: "24.1.0",
                    bytes: sums("24.1.0").as_bytes()
                },
                false
            )
            .is_err()
        );
    }
    for bytes in [
        format!("{}{}", sums("24.1.0"), sums("24.1.0")).into_bytes(),
        vec![0; node_metadata::MAX_CHECKSUM_BYTES + 1],
        b"secret-token\0".to_vec(),
    ] {
        let error = resolve(
            &requirements,
            &index(&[entry("24.1.0")]),
            ChecksumManifest {
                version: "24.1.0",
                bytes: &bytes,
            },
            false,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret-token"));
    }
    let oversized = " ".repeat(crate::node_discovery::MAX_MANIFEST_BYTES + 1);
    assert!(matches!(
        NodeRequirements::from_sources(Some(&oversized), Some("24.x"), None),
        Err(NodeDiscoveryError::TooLarge { .. })
    ));
    let oversized = " ".repeat(crate::version_request::MAX_REQUEST_BYTES + 1);
    for (nvmrc, node_version) in [
        (Some(oversized.as_str()), None),
        (None, Some(oversized.as_str())),
    ] {
        assert!(matches!(
            NodeRequirements::from_sources(None, nvmrc, node_version),
            Err(NodeDiscoveryError::TooLarge { .. })
        ));
    }
    Ok(())
}

#[test]
fn bundled_ownership_does_not_invent_npx_for_older_npm() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("8.0.0"), None)?;
    for npm in ["5.0.0", "5.1.0", "5.2.0", "11.6.1"] {
        let mut value = entry("8.0.0");
        value["npm"] = json!(npm);
        assert!(selection(&requirements, &[value.clone()], "8.0.0", false).is_ok());
        let bundled = selection(&requirements, &[value], "8.0.0", true);
        if npm == "5.0.0" || npm == "5.1.0" {
            assert!(matches!(bundled, Err(Error::UnsupportedBundledNpm)));
        } else {
            assert!(bundled.is_ok());
        }
    }
    Ok(())
}

#[test]
fn provisioning_rejects_unknown_or_nonexact_bundled_identity_options() -> TestResult {
    let requirements = NodeRequirements::from_sources(None, Some("24.x"), None)?;
    let tool = selection(&requirements, &[entry("24.1.0")], "24.1.0", true)?.into_tool();
    for (key, values) in [
        ("bundled-npm", vec!["11.x"]),
        ("bundled-npm", vec!["5.1.0"]),
        ("bundled-npm", vec!["v11.6.1"]),
        ("artifact-version", vec!["11.6.1"]),
        ("bundled-npm", vec!["10.0.0", "11.6.1"]),
    ] {
        let mut bad = tool.clone();
        bad.options =
            BTreeMap::from([(key.into(), values.into_iter().map(str::to_owned).collect())]);
        assert!(NodePlan::from_lock(&lock(bad), Platform::LinuxX64Gnu).is_err());
    }
    let mut bad = tool;
    let Installation::Managed { artifacts } = &mut bad.installation else {
        panic!("managed")
    };
    artifacts
        .get_mut(&Platform::LinuxX64Gnu)
        .ok_or("target")?
        .get_mut("distribution")
        .ok_or("part")?
        .executables
        .remove("npm");
    assert!(NodePlan::from_lock(&lock(bad), Platform::LinuxX64Gnu).is_err());
    Ok(())
}
