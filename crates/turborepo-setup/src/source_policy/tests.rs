use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;

// Owned fixture inventory: never inspect the contributor's home/env/system.
struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    system: PathBuf,
    node: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self, io::Error> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("repo/nested");
        let home = directory.path().join("home");
        let system = directory.path().join("system/npmrc");
        let node = directory.path().join("selected/bin/node");
        for path in [
            &root,
            &home,
            system.parent().ok_or_else(|| io::Error::other("fixture"))?,
            node.parent().ok_or_else(|| io::Error::other("fixture"))?,
        ] {
            fs::create_dir_all(path)?;
        }
        fs::write(&node, b"never execute this")?;
        Ok(Self {
            directory,
            root,
            home,
            system,
            node,
        })
    }
    fn check(&self, env: &[(OsString, OsString)]) -> Result<OfficialSourcePolicy, Error> {
        inspect_inputs(
            &self.root,
            &self.home,
            Some(&self.node),
            env,
            vec![self.system.clone()],
            Some(self.directory.path()),
        )
    }
}

#[test]
fn config_owner_and_metadata_table() -> TestResult {
    for owner in [
        ConfigOwner::Repository,
        ConfigOwner::Ancestor,
        ConfigOwner::User,
        ConfigOwner::System,
        ConfigOwner::Prefix,
        ConfigOwner::RootPolicy,
    ] {
        assert_eq!(validate(owner, Observation::Missing), Ok(()));
        assert_eq!(
            validate(owner, Observation::Unsafe),
            Err(Error::Inspection(owner))
        );
        assert_eq!(
            validate(owner, Observation::Readable),
            Err(Error::Configured(owner))
        );
    }
    for contents in [
        "",
        "registry=https://registry.npmjs.org",
        "script-shell=private-secret",
        "//private-registry/:_authToken=private-secret",
    ] {
        for owner in [
            ConfigOwner::Repository,
            ConfigOwner::Ancestor,
            ConfigOwner::User,
            ConfigOwner::System,
            ConfigOwner::Prefix,
        ] {
            let fixture = Fixture::new()?;
            let path = match owner {
                ConfigOwner::Repository => fixture.root.join(".npmrc"),
                ConfigOwner::Ancestor => fixture
                    .root
                    .parent()
                    .ok_or("fixture parent")?
                    .join(".npmrc"),
                ConfigOwner::User => fixture.home.join(".npmrc"),
                ConfigOwner::System => fixture.system.clone(),
                _ => fixture.directory.path().join("selected/etc/npmrc"),
            };
            fs::create_dir_all(path.parent().ok_or("fixture parent")?)?;
            fs::write(&path, contents)?;
            let error = fixture.check(&[]).err().ok_or("expected rejection")?;
            assert_eq!(error, Error::Configured(owner));
            assert!(!format!("{error} {error:?}").contains("private-secret"));
            assert!(!format!("{error} {error:?}").contains(&path.display().to_string()));
        }
    }
    Ok(())
}

#[test]
fn environment_prefix_and_known_system_locations() -> TestResult {
    let fixture = Fixture::new()?;
    assert_eq!(fixture.check(&[])?.registry(), PUBLIC_NPM_REGISTRY);
    for key in [
        "npm_config_registry",
        "NpM_CoNfIg_PrEfIx",
        "NPM_CONFIG_GLOBALCONFIG",
        "npm_config_userconfig",
        "NPM_CONFIG_SCRIPT_SHELL",
        "COREPACK_NPM_REGISTRY",
        "NVM_NODEJS_ORG_MIRROR",
        "DESTDIR",
    ] {
        for value in ["", "private-secret"] {
            assert_eq!(
                fixture.check(&[(key.into(), value.into())]).err(),
                Some(Error::Environment)
            );
        }
    }
    let prefix = fixture.directory.path().join("custom-prefix");
    fs::create_dir_all(prefix.join("etc"))?;
    fs::write(prefix.join("etc/npmrc"), b"")?;
    assert_eq!(
        fixture
            .check(&[("PREFIX".into(), prefix.into_os_string())])
            .err(),
        Some(Error::Configured(ConfigOwner::Prefix))
    );
    #[cfg(unix)]
    assert_eq!(
        system_configs(),
        [
            "/etc/npmrc",
            "/usr/etc/npmrc",
            "/usr/local/etc/npmrc",
            "/opt/homebrew/etc/npmrc"
        ]
        .map(PathBuf::from)
    );
    Ok(())
}

#[test]
fn root_json_and_jsonc_policy_is_not_silently_ignored() -> TestResult {
    for name in CONFIG_FILES {
        let fixture = Fixture::new()?;
        let path = fixture.root.join(name);
        fs::write(
            &path,
            "{ // safe normal configuration\n \"futureFlags\": {\"experimentalSetup\":true}, }",
        )?;
        assert!(fixture.check(&[]).is_ok());
        for contents in [
            "{\"setup\":{}}",
            "{\"setup\":null}",
            "{\"setup\":{\"registry\":\"private-secret\"}}",
        ] {
            fs::write(&path, contents)?;
            assert_eq!(
                fixture.check(&[]).err(),
                Some(Error::Configured(ConfigOwner::RootPolicy))
            );
        }
        fs::write(path, "invalid private-secret")?;
        assert_eq!(
            fixture.check(&[]).err(),
            Some(Error::Inspection(ConfigOwner::RootPolicy))
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_invocation_and_actual_node_prefix_are_inspected() -> TestResult {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new()?;
    let shortcut = fixture.directory.path().join("shortcut");
    symlink(&fixture.root, &shortcut)?;
    let ancestor_config = fixture.root.parent().ok_or("fixture")?.join(".npmrc");
    fs::write(&ancestor_config, b"")?;
    assert_eq!(
        inspect_inputs(
            &shortcut,
            &fixture.home,
            Some(&fixture.node),
            &[],
            vec![],
            Some(fixture.directory.path())
        )
        .err(),
        Some(Error::Configured(ConfigOwner::Ancestor))
    );
    fs::remove_file(ancestor_config)?;
    let bundled = fixture
        .directory
        .path()
        .join("selected/lib/node_modules/npm/npmrc");
    fs::create_dir_all(bundled.parent().ok_or("fixture")?)?;
    fs::write(&bundled, b"prefix=/unreported/private-prefix")?;
    assert_eq!(
        fixture.check(&[]).err(),
        Some(Error::Configured(ConfigOwner::Prefix))
    );
    fs::remove_file(bundled)?;
    let selected = fixture.directory.path().join("wrapper/bin/node");
    fs::create_dir_all(selected.parent().ok_or("fixture")?)?;
    symlink(&fixture.node, &selected)?;
    fs::create_dir_all(fixture.directory.path().join("selected/etc"))?;
    fs::write(fixture.directory.path().join("selected/etc/npmrc"), b"")?;
    assert_eq!(
        inspect_inputs(
            &fixture.root,
            &fixture.home,
            Some(&selected),
            &[],
            vec![],
            Some(fixture.directory.path())
        )
        .err(),
        Some(Error::Configured(ConfigOwner::Prefix))
    );
    Ok(())
}

#[test]
fn nonfile_unreadable_and_unknown_node_layout_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.home.join(".npmrc"))?;
    assert_eq!(
        fixture.check(&[]).err(),
        Some(Error::Inspection(ConfigOwner::User))
    );
    fs::remove_dir(fixture.home.join(".npmrc"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = fixture.home.join(".npmrc");
        symlink(fixture.home.join("missing"), &path)?;
        assert_eq!(
            fixture.check(&[]).err(),
            Some(Error::Inspection(ConfigOwner::User))
        );
        fs::remove_file(&path)?;
        fs::write(&path, b"private-secret")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0))?;
        assert_eq!(
            fixture.check(&[]).err(),
            Some(Error::Inspection(ConfigOwner::User))
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        fs::remove_file(path)?;
    }
    let unknown = fixture.directory.path().join("task-executable");
    fs::write(&unknown, b"not current Node")?;
    assert_eq!(
        inspect_inputs(
            &fixture.root,
            &fixture.home,
            Some(&unknown),
            &[],
            vec![],
            Some(fixture.directory.path())
        )
        .err(),
        Some(Error::Locations)
    );
    Ok(())
}
