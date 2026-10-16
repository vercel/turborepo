use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;

// Every filesystem input is owned; never inspect or mutate host
// home/env/system.
struct Fixture {
    directory: tempfile::TempDir,
    boundary: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        let directory = tempfile::tempdir()?;
        let boundary = fs::canonicalize(directory.path())?;
        let root = boundary.join("repo/nested");
        fs::create_dir_all(&root)?;
        Ok(Self {
            directory,
            boundary,
            root,
        })
    }

    fn check(&self) -> Result<ToolDownloadPolicy, Error> {
        inspect_inputs(&self.root, Some(&self.boundary))
    }

    fn ambient_paths(&self) -> Vec<PathBuf> {
        [
            "repo/nested/.npmrc",
            "repo/.npmrc",
            ".npmrc",
            "home/.npmrc",
            "home/.config/pnpm/rc",
            "system/etc/npmrc",
            "selected/etc/npmrc",
            "selected/lib/node_modules/npm/npmrc",
            "selected/node_modules/npm/npmrc",
            "selected/bin/node",
        ]
        .map(|path| self.boundary.join(path))
        .to_vec()
    }
}

#[test]
fn public_inspection_requires_only_an_invocation() {
    // Check the public signature without scanning the contributor's ancestors.
    let _: fn(&Path) -> Result<ToolDownloadPolicy, Error> = ToolDownloadPolicy::inspect;
}

#[test]
fn root_policy_observations_fail_closed() {
    let owner = ConfigOwner::RootPolicy;
    assert_eq!(validate(Observation::Missing), Ok(()));
    assert_eq!(validate(Observation::PolicyAbsent), Ok(()));
    assert_eq!(
        validate(Observation::Readable),
        Err(Error::Configured(owner))
    );
    assert_eq!(validate(Observation::Unsafe), Err(Error::Inspection(owner)));
}

#[test]
fn ambient_files_are_ignored_even_when_empty_invalid_or_nonfiles() -> TestResult {
    let fixture = Fixture::new()?;
    assert_eq!(fixture.check()?.registry(), PUBLIC_NPM_REGISTRY);
    for path in fixture.ambient_paths() {
        fs::create_dir_all(path.parent().ok_or("ambient parent")?)?;
        for contents in [
            b"".as_slice(),
            b"registry=https://private-secret.invalid/\n//registry/:_authToken=private-secret",
            b"\xff\0invalid private-secret",
        ] {
            fs::write(&path, contents)?;
            assert!(fixture.check().is_ok(), "{}", path.display());
        }
        fs::remove_file(&path)?;
        fs::create_dir(&path)?;
        assert!(fixture.check().is_ok(), "{}", path.display());
        fs::remove_dir(&path)?;
    }
    // Ambient registry configuration cannot override or invalidate applied mirrors.
    fs::write(
        fixture.root.join("turbo.json"),
        r#"{"setup":{"mirrors":{"npmRegistry":"https://mirror.invalid/npm"}}}"#,
    )?;
    fs::write(fixture.root.join(".npmrc"), "registry=private-secret")?;
    assert!(fixture.check().is_ok());
    Ok(())
}

#[cfg(unix)]
#[test]
fn ambient_symlinks_and_unreadable_files_are_ignored() -> TestResult {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture = Fixture::new()?;
    let outside = tempfile::tempdir()?;
    let target = outside.path().join("private-secret");
    fs::write(&target, b"private-secret")?;
    for path in fixture.ambient_paths() {
        fs::create_dir_all(path.parent().ok_or("ambient parent")?)?;
        for target in [&target, &outside.path().join("missing")] {
            symlink(target, &path)?;
            assert!(fixture.check().is_ok());
            fs::remove_file(&path)?;
        }
        fs::write(&path, b"private-secret")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0))?;
        let result = fixture.check();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        assert!(result.is_ok());
        fs::remove_file(path)?;
    }
    // Even intermediate ambient directory links must not trigger inspection.
    fs::remove_dir_all(fixture.boundary.join("home"))?;
    symlink(outside.path(), fixture.boundary.join("home"))?;
    assert!(fixture.check().is_ok());
    Ok(())
}

#[test]
fn root_json_and_jsonc_applied_policy_shapes_pass() -> TestResult {
    for name in CONFIG_FILES {
        let fixture = Fixture::new()?;
        let path = fixture.root.join(name);
        for contents in [
            "{}",
            "{ // normal configuration\n \"futureFlags\": {\"experimentalSetup\":true}, }",
            r#"{"setup":{"tools":"managed","dependencies":"skip","rust":{},"javascript":{"tools":"verify-system"}}}"#,
            r#"{"setup":{"python":{"dependencies":"skip"},"go":{"tools":"managed"}}}"#,
            r#"{"setup":{"additionalTools":{},"tools":"skip"}}"#,
            r#"{"setup":{"additionalTools":{}}}"#,
            // Preserve applied_policy's shape validation, not value validation.
            r#"{"setup":{"tools":"future-value","additionalTools":null}}"#,
            r#"{"setup":{"mirrors":{}}}"#,
            r#"{"setup":{"tools":"managed","mirrors":{"node":"https://mirror.invalid/dist","npmRegistry":"https://mirror.invalid/npm","bun":"https://mirror.invalid/github","uv":"https://mirror.invalid/astral","python":"https://mirror.invalid/astral","go":"https://mirror.invalid/go","rust":"https://mirror.invalid/rust"}}}"#,
        ] {
            fs::write(&path, contents)?;
            assert!(fixture.check().is_ok(), "{name}: {contents}");
        }
    }
    Ok(())
}

#[test]
fn unsupported_root_policy_shapes_are_configured_without_echoing_secrets() -> TestResult {
    for name in CONFIG_FILES {
        let fixture = Fixture::new()?;
        let path = fixture.root.join(name);
        for contents in [
            r#"{"setup":{}}"#,
            r#"{"setup":null}"#,
            r#"{"setup":[]}"#,
            r#"{"setup":"private-secret"}"#,
            r#"{"setup":{"registry":"private-secret"}}"#,
            r#"{"setup":{"tools":"skip","mirror":"private-secret"}}"#,
            r#"{"setup":{"javascript":{"registry":"private-secret"}}}"#,
            r#"{"setup":{"tools":{"url":"private-secret"}}}"#,
            r#"{"setup":{"dependencies":false}}"#,
            r#"{"setup":{"lifecycleScripts":[]}}"#,
            r#"{"setup":{"javascript":"skip"}}"#,
            r#"{"setup":{"mirrors":"private-secret"}}"#,
            r#"{"setup":{"mirrors":{"deno":"private-secret"}}}"#,
            r#"{"setup":{"mirrors":{"node":{"url":"private-secret"}}}}"#,
            // An applied duplicate does not hide an unapplied setup property.
            r#"{"setup":{"tools":"skip"},"setup":{"registry":"private-secret"}}"#,
        ] {
            fs::write(&path, contents)?;
            let error = fixture.check().err().ok_or("expected policy rejection")?;
            assert_eq!(error, Error::Configured(ConfigOwner::RootPolicy));
            assert_redacted(&error, &path);
        }
    }
    Ok(())
}

fn assert_redacted(error: &Error, path: &Path) {
    let text = format!("{error:?} {error}");
    assert!(!text.contains("private-secret"));
    assert!(!text.contains(&path.display().to_string()));
}

#[test]
fn invalid_nonobject_oversized_and_nonfile_root_configs_fail_inspection() -> TestResult {
    for name in CONFIG_FILES {
        let fixture = Fixture::new()?;
        let path = fixture.root.join(name);
        for contents in [
            b"".to_vec(),
            b"invalid private-secret".to_vec(),
            b"{\"private-secret\":".to_vec(),
            b"[]".to_vec(),
            b"null".to_vec(),
            b"\"private-secret\"".to_vec(),
            b"true".to_vec(),
            b"42".to_vec(),
            b"\xffprivate-secret".to_vec(),
            // A valid object beyond the read limit must not pass.
            format!("{{\"private-secret\":\"{}\"}}", "x".repeat(1_048_576)).into_bytes(),
        ] {
            fs::write(&path, contents)?;
            let error = fixture.check().err().ok_or("expected inspection failure")?;
            assert_eq!(error, Error::Inspection(ConfigOwner::RootPolicy));
            assert_redacted(&error, &path);
        }
        fs::remove_file(&path)?;
        fs::create_dir(&path)?;
        assert_eq!(
            fixture.check().err(),
            Some(Error::Inspection(ConfigOwner::RootPolicy))
        );
        fs::remove_dir(path)?;
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn root_config_symlinks_and_unreadable_files_fail_inspection() -> TestResult {
    use std::os::unix::fs::{PermissionsExt, symlink};

    for name in CONFIG_FILES {
        let fixture = Fixture::new()?;
        let path = fixture.root.join(name);
        let target = fixture.boundary.join("private-secret");
        fs::write(&target, "{}")?;
        for target in [&target, &fixture.boundary.join("missing")] {
            symlink(target, &path)?;
            assert_eq!(
                fixture.check().err(),
                Some(Error::Inspection(ConfigOwner::RootPolicy))
            );
            fs::remove_file(&path)?;
        }
        fs::write(&path, "{}")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0))?;
        // Privileged test processes may still read mode-000 files.
        let unreadable = fs::File::open(&path).is_err();
        let result = fixture.check();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        if unreadable {
            assert_eq!(
                result.err(),
                Some(Error::Inspection(ConfigOwner::RootPolicy))
            );
        }
    }
    Ok(())
}

#[test]
fn every_ancestor_and_both_root_config_names_are_checked() -> TestResult {
    let fixture = Fixture::new()?;
    for ancestor in [
        &fixture.root,
        &fixture.boundary.join("repo"),
        &fixture.boundary,
    ] {
        for name in CONFIG_FILES {
            let path = ancestor.join(name);
            fs::write(&path, r#"{"setup":{}}"#)?;
            assert_eq!(
                fixture.check().err(),
                Some(Error::Configured(ConfigOwner::RootPolicy))
            );
            fs::remove_file(path)?;
        }
    }
    fs::write(fixture.root.join("turbo.json"), "{}")?;
    fs::write(fixture.root.join("turbo.jsonc"), r#"{"setup":{}}"#)?;
    assert_eq!(
        fixture.check().err(),
        Some(Error::Configured(ConfigOwner::RootPolicy))
    );
    // The boundary itself is a valid invocation and terminates the scan.
    assert!(inspect_inputs(&fixture.boundary, Some(&fixture.boundary)).is_ok());
    assert!(fixture.directory.path().exists());
    Ok(())
}

#[test]
fn owned_boundary_stops_before_configs_in_its_parent() -> TestResult {
    let fixture = Fixture::new()?;
    let boundary = fixture.boundary.join("repo");
    for name in CONFIG_FILES {
        fs::write(fixture.boundary.join(name), r#"{"setup":{}}"#)?;
    }
    assert!(inspect_inputs(&fixture.root, Some(&boundary)).is_ok());
    // Moving the boundary outward includes those same files.
    assert_eq!(
        fixture.check().err(),
        Some(Error::Configured(ConfigOwner::RootPolicy))
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn lexical_and_canonical_invocation_ancestors_are_both_checked() -> TestResult {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    let lexical = fixture.boundary.join("shortcut-parent");
    fs::create_dir(&lexical)?;
    let shortcut = lexical.join("shortcut");
    symlink(&fixture.root, &shortcut)?;
    assert!(inspect_inputs(&shortcut, Some(&fixture.boundary)).is_ok());
    for ancestor in [&lexical, &fixture.boundary.join("repo")] {
        for name in CONFIG_FILES {
            let path = ancestor.join(name);
            fs::write(&path, r#"{"setup":{}}"#)?;
            assert_eq!(
                inspect_inputs(&shortcut, Some(&fixture.boundary)).err(),
                Some(Error::Configured(ConfigOwner::RootPolicy))
            );
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[test]
fn invalid_locations_and_owned_boundary_escapes_are_rejected() -> TestResult {
    let fixture = Fixture::new()?;
    let outside = tempfile::tempdir()?;
    // PathBuf::join normalizes `..` on Windows verbatim paths; keep the traversal
    // literal.
    let parent_traversal = |base: &Path| {
        let mut path = base.as_os_str().to_os_string();
        path.push(std::path::MAIN_SEPARATOR_STR);
        path.push("..");
        PathBuf::from(path)
    };
    for invocation in [
        PathBuf::from("relative"),
        fixture.root.join("missing"),
        outside.path().to_path_buf(),
        parent_traversal(&fixture.boundary),
        parent_traversal(&fixture.root),
    ] {
        assert_eq!(
            inspect_inputs(&invocation, Some(&fixture.boundary)).err(),
            Some(Error::Locations),
            "invocation: {invocation:?}"
        );
    }
    for boundary in [PathBuf::from("relative"), fixture.boundary.join("missing")] {
        assert_eq!(
            inspect_inputs(&fixture.root, Some(&boundary)).err(),
            Some(Error::Locations)
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn canonical_invocation_escape_is_rejected_before_policy_inspection() -> TestResult {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("turbo.json"), "private-secret")?;
    let shortcut = fixture.root.join("escape");
    symlink(outside.path(), &shortcut)?;
    assert_eq!(
        inspect_inputs(&shortcut, Some(&fixture.boundary)).err(),
        Some(Error::Locations)
    );
    Ok(())
}
