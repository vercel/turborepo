use std::fs;

use tempfile::TempDir;

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture(TempDir);
impl Fixture {
    fn put(&self, file: &str, text: impl AsRef<[u8]>) -> io::Result<()> {
        fs::write(self.0.path().join(file), text)
    }
    fn read(&self) -> Result<NodeRequirements, NodeDiscoveryError> {
        NodeRequirements::read(self.0.path())
    }
    fn resolve(&self) -> Result<ResolvedNode, NodeDiscoveryError> {
        self.read()?.resolve(&catalog()?)
    }
    fn conflict(&self) -> TestResult {
        assert!(matches!(
            self.resolve(),
            Err(NodeDiscoveryError::NoMatchingRelease { .. })
        ));
        Ok(())
    }
    fn error(&self) -> Result<String, Box<dyn std::error::Error>> {
        let error = self.read().err().ok_or("expected discovery error")?;
        Ok(error.to_string())
    }
}

fn root(files: &[(&str, &str)]) -> Result<Fixture, Box<dyn std::error::Error>> {
    let root = Fixture(tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))?);
    for (file, text) in files {
        root.put(file, text)?;
    }
    Ok(root)
}

fn catalog() -> Result<Vec<NodeRelease>, NodeDiscoveryError> {
    [
        ("24.10.0", Some("Krypton")),
        ("22.9.0", Some("Jod")),
        ("26.0.0-rc.2", Some("Future")),
        ("24.2.0", Some("krypton")),
        ("25.1.0", None),
    ]
    .into_iter()
    .map(|(v, lts)| NodeRelease::new(v, lts))
    .collect()
}

#[test]
fn all_sources_intersect_with_ordered_normalized_provenance() -> TestResult {
    let root = root(&[
        (
            "package.json",
            r#"{"devEngines":{"runtime":{"name":"node","version":" >=22 <26 "}},"engines":{"node":"<24.11"}}"#,
        ),
        (".nvmrc", "24.x\r\n"),
        (".node-version", " = v24.10.0 \n"),
    ])?;
    let requests = root.read()?;
    let mut metadata = catalog()?;
    let selected = requests.resolve(&metadata)?;
    assert_eq!(selected.version.to_string(), "24.10.0");
    assert_eq!(selected.selection_source, selected.sources[0]);
    assert_eq!(selected.sources.len(), 4);
    for (source, (location, request)) in selected.sources.iter().zip([
        ("package.json#devEngines.runtime.version", ">=22 <26"),
        (".nvmrc", "24.x"),
        (".node-version", "24.10.0"),
        ("package.json#engines.node", "<24.11"),
    ]) {
        assert_eq!(source.location(), location);
        assert_eq!(source.request.as_deref(), Some(request));
    }
    metadata.reverse();
    assert_eq!(requests.resolve(&metadata)?.version, selected.version);
    Ok(())
}

#[test]
fn each_selection_source_and_precedence_fallback_work() -> TestResult {
    for files in [
        vec![(".nvmrc", "24.x")],
        vec![(".node-version", "^24.0.0")],
        vec![(".nvmrc", "24"), (".node-version", "24.2.0")],
    ] {
        let root = root(&files)?;
        let result = root.resolve()?;
        assert_eq!(result.selection_source.file, files[0].0);
        assert_eq!(result.version.major, 24);
        if files.len() == 2 {
            assert_eq!(result.version.to_string(), "24.2.0");
        }
    }
    let root = root(&[("package.json", r#"{"engines":{"node":"24.x"}}"#)])?;
    let selected = root.resolve()?;
    assert_eq!(selected.version.to_string(), "24.10.0");
    assert_eq!(
        selected.selection_source.location(),
        "package.json#engines.node"
    );
    root.put(".nvmrc", "24.2.0")?;
    assert_eq!(root.resolve()?.selection_source.file, ".nvmrc");
    assert_eq!(root.resolve()?.version.to_string(), "24.2.0");
    Ok(())
}

#[test]
fn aliases_resolve_only_from_injected_lts_metadata() -> TestResult {
    for (alias, expected) in [
        ("node", "25.1.0"),
        ("lts/*", "24.10.0"),
        ("lts/JOD", "22.9.0"),
    ] {
        let root = root(&[(".nvmrc", alias)])?;
        let result = root.read()?.resolve(&catalog()?)?;
        assert_eq!(result.version.to_string(), expected);
        let expected = Some(alias.to_ascii_lowercase());
        assert_eq!(result.sources[0].request, expected);
    }
    for alias in "latest current stable NODE lts lts/ lts/jo* lts/../jod".split_whitespace() {
        let root = root(&[(".nvmrc", alias)])?;
        assert!(root.error()?.contains(".nvmrc"));
    }
    root(&[(".nvmrc", "lts/unknown")])?.conflict()?;
    for (file, field) in [
        (".node-version", ""),
        ("package.json", "engines.node"),
        ("package.json", "devEngines.runtime.version"),
    ] {
        for alias in ["node", "lts/*", "lts/jod"] {
            let contents = match field {
                "" => alias.into(),
                "engines.node" => format!(r#"{{"engines":{{"node":"{alias}"}}}}"#),
                _ => format!(
                    r#"{{"devEngines":{{"runtime":{{"name":"node","version":"{alias}"}}}}}}"#
                ),
            };
            let message = root(&[(file, &contents)])?.error()?;
            assert!(message.contains(file) && message.contains(field));
        }
    }
    Ok(())
}

#[test]
fn conflicts_report_every_file_and_never_return_a_selection() -> TestResult {
    let root = root(&[
        (
            "package.json",
            r#"{"devEngines":{"runtime":{"name":"node","version":"22"}},"engines":{"node":">=22"}}"#,
        ),
        (".nvmrc", "24.x"),
        (".node-version", "24.10.0"),
    ])?;
    let result = root.read()?.resolve(&catalog()?);
    let diagnostic = result.err().ok_or("expected conflict")?.to_string();
    for source in [
        "package.json#devEngines.runtime.version",
        "package.json#engines.node",
        ".nvmrc",
        ".node-version",
    ] {
        assert!(diagnostic.contains(source), "{diagnostic}");
    }
    root.put("package.json", r#"{"engines":{"node":"<24"}}"#)?;
    root.conflict()?;
    Ok(())
}

#[test]
fn runtime_alternatives_are_or_across_sources_and_provenance_matches() -> TestResult {
    let manifest = r#"{"devEngines":{"runtime":[{"name":"bun","version":"1"},{"name":"node","version":"24"},{"name":"node","version":"22"}]}}"#;
    let root = root(&[("package.json", manifest)])?;
    assert_eq!(root.resolve()?.version.to_string(), "24.10.0");
    root.put(".nvmrc", "22.x")?;
    let selected = root.resolve()?;
    assert_eq!(selected.version.to_string(), "22.9.0");
    assert_eq!(selected.sources.len(), 3);
    assert_eq!(selected.selection_source, selected.sources[1]);
    assert_eq!(
        selected.selection_source.field.as_deref(),
        Some("devEngines.runtime[2].version")
    );
    root.put(".node-version", "24")?;
    root.conflict()?;
    root.put(".node-version", "25")?;
    root.put(".nvmrc", "25")?;
    root.conflict()?;
    Ok(())
}

#[test]
fn prereleases_require_opt_in_in_every_version_request() -> TestResult {
    let root = root(&[(".nvmrc", ">=26.0.0-rc.1 <26.0.0")])?;
    assert_eq!(root.resolve()?.version.to_string(), "26.0.0-rc.2");
    root.put("package.json", r#"{"engines":{"node":"*"}}"#)?;
    root.conflict()?;
    root.put(
        "package.json",
        r#"{"engines":{"node":">=26.0.0-rc.1 <27"}}"#,
    )?;
    root.resolve()?;
    root.put(".nvmrc", "26.0.0-rc.2")?;
    root.resolve()?;
    Ok(())
}

#[test]
fn exact_identity_and_range_build_ties_are_reproducible() -> TestResult {
    let root = root(&[(".node-version", "= v24.1.0+one")])?;
    let requests = root.read()?;
    let mut metadata = vec![
        NodeRelease::new("24.1.0+two", Some("jod"))?,
        NodeRelease::new("24.1.0+one", Some("krypton"))?,
    ];
    let selected = requests.resolve(&metadata)?.version;
    assert_eq!(selected.to_string(), "24.1.0+one");
    assert!(requests.resolve(&metadata[..1]).is_err());
    root.put(".node-version", "^24")?;
    root.put("package.json", r#"{"engines":{"node":"24.1.0+one"}}"#)?;
    let requests = root.read()?;
    let selected = requests.resolve(&metadata)?.version;
    metadata.reverse();
    assert_eq!(requests.resolve(&metadata)?.version, selected);
    assert_eq!(selected.to_string(), "24.1.0+two");
    assert!(requests.resolve(&[]).is_err());
    let duplicate = NodeRelease::new("42.0.0", Some("jod"))?;
    for lts in [None, Some("argon"), Some("JOD")] {
        let mut items = metadata.clone();
        items.extend([duplicate.clone(), NodeRelease::new("42.0.0", lts)?]);
        for _ in 0..2 {
            let result = requests.resolve(&items);
            assert_eq!(result.is_ok(), lts == Some("JOD"));
            if let Err(error) = result {
                assert!(matches!(error, NodeDiscoveryError::InvalidMetadata(_)));
            }
            items.reverse();
        }
    }
    Ok(())
}

#[test]
fn malformed_manifests_never_fall_back_to_version_files() -> TestResult {
    let root = root(&[(".nvmrc", "24")])?;
    for json in [
        "{",
        "[]",
        "null",
        "{} trailing",
        r#"{"devEngines":null}"#,
        r#"{"devEngines":{"runtime":"24"}}"#,
        r#"{"devEngines":{"runtime":{"name":"node","version":null}}}"#,
        r#"{"engines":{"node":24}}"#,
        r#"{"engines":null}"#,
        r#"{"engines":{"node":"24","node":"22"}}"#,
        r#"{"devEngines":{"runtime":{"name":"node","version":"24","version":"22"}}}"#,
        r#"{"devEngines":{},"devEngines":{}}"#,
        r#"{"devEngines":{"runtime":[{"name":"node","version":"24"},{"name":"node","version":24}]}}"#,
        r#"{"devEngines":{"runtime":[{"name":"node","version":"24"},{"version":"24"}]}}"#,
        r#"{"devEngines":{"runtime":[{"name":"node","version":"24"},null]}}"#,
        r#"{"devEngines":{"runtime":[{"name":"node","version":"24"},[]]}}"#,
    ] {
        root.put("package.json", json)?;
        assert!(root.error()?.contains("package.json"), "{json}");
    }
    root.put("package.json", "{}")?;
    // Newlines are AND whitespace, not an excuse to discard another request.
    root.put(".node-version", "24\n22")?;
    root.conflict()?;
    for input in [
        "".into(),
        "# comment\n24".into(),
        "\u{2003}24".into(),
        "24.x-beta".into(),
        "* || garbage".into(),
        vec!["24"; 33].join("||"),
        vec!["24"; 65].join(" "),
    ] {
        root.put(".node-version", input)?;
        assert!(root.error()?.contains(".node-version"));
    }
    fs::remove_file(root.0.path().join(".node-version"))?;
    root.put(".nvmrc", [0xff])?;
    assert!(root.error()?.contains("UTF-8"));
    fs::remove_file(root.0.path().join(".nvmrc"))?;
    fs::create_dir(root.0.path().join(".nvmrc"))?;
    assert!(matches!(root.read(), Err(NodeDiscoveryError::Read(..))));
    Ok(())
}

#[test]
fn discovery_resource_boundaries_are_inclusive() -> TestResult {
    let root = root(&[])?;
    for (file, valid, limit) in [
        (".nvmrc", "24", MAX_REQUEST_BYTES),
        ("package.json", "{}", MAX_MANIFEST_BYTES),
    ] {
        root.put(file, format!("{valid}{}", " ".repeat(limit - valid.len())))?;
        assert!(root.read().is_ok());
        root.put(file, " ".repeat(limit + 1))?;
        assert!(matches!(
            root.read(),
            Err(NodeDiscoveryError::TooLarge { .. })
        ));
        root.put(file, valid)?;
    }
    for (count, valid) in [
        (MAX_RUNTIME_ENTRIES, true),
        (MAX_RUNTIME_ENTRIES + 1, false),
    ] {
        let entries = vec![r#"{"name":"node","version":"24"}"#; count].join(",");
        root.put(
            "package.json",
            format!(r#"{{"devEngines":{{"runtime":[{entries}]}}}}"#),
        )?;
        assert_eq!(root.read().is_ok(), valid);
    }
    root.put("package.json", "{}")?;
    let requests = root.read()?;
    let mut releases = vec![NodeRelease::new("24.0.0", None)?; MAX_RELEASES];
    assert!(requests.resolve(&releases).is_ok());
    releases.push(NodeRelease::new("24.0.1", None)?);
    assert!(matches!(
        requests.resolve(&releases),
        Err(NodeDiscoveryError::InvalidMetadata(_))
    ));
    for version in "v24.0.0 24 24.x =24.0.0 9007199254740992.0.0".split_whitespace() {
        assert!(NodeRelease::new(version, None).is_err(), "{version}");
    }
    assert!(NodeRelease::new("24.0.0 ", None).is_err());
    assert!(NodeRelease::new(&format!("24.0.0+{}", "a".repeat(256)), None).is_err());
    assert!(NodeRelease::new("24.0.0", Some("bad/name")).is_err());
    let request = " ".repeat(MAX_REQUEST_BYTES + 1);
    let json = serde_json::json!({"devEngines":{"runtime":{"name":"node","version":request}}});
    root.put("package.json", json.to_string())?;
    assert!(root.error()?.contains("devEngines.runtime.version"));
    root.put(
        "package.json",
        format!("{{\"other\":{}0{}}}", "[".repeat(140), "]".repeat(140)),
    )?;
    assert!(root.error()?.contains("recursion limit"));
    Ok(())
}

#[test]
fn name_only_runtime_preserves_absence_and_companion_prerelease_opt_in() -> TestResult {
    let root = root(&[])?;
    for runtime in [r#"{"name":"node"}"#, r#"[{"name":"node"}]"#] {
        root.put(
            "package.json",
            format!(r#"{{"devEngines":{{"runtime":{runtime}}}}}"#),
        )?;
        for (request, expected) in [("24.x", "24.10.0"), ("26.0.0-rc.2", "26.0.0-rc.2")] {
            root.put(".nvmrc", request)?;
            let selected = root.resolve()?;
            assert_eq!(selected.version.to_string(), expected);
            assert!(selected.sources[0].request.is_none());
            assert_eq!(selected.selection_source, selected.sources[0]);
        }
    }
    Ok(())
}
