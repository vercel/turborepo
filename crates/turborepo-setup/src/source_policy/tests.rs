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
        "NPM_CONFIG_HTTPS_PROXY",
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
        system_configs(&[])?,
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

/// `npx`/`npm exec`/`npm run` (npm 11) and pnpm 11+ default wrapper keys.
fn wrapper_env(fixture: &Fixture) -> Vec<(OsString, OsString)> {
    let at = |path: &str| fixture.directory.path().join(path).into_os_string();
    let mut env: Vec<(OsString, OsString)> = [
        ("npm_config_cache", at("home/.npm")),
        ("npm_config_call", "turbo setup".into()),
        ("npm_config_global_prefix", at("selected")),
        ("npm_config_globalconfig", at("selected/etc/npmrc")),
        ("npm_config_init_module", at("home/.npm-init.js")),
        (
            "npm_config_local_prefix",
            fixture.root.clone().into_os_string(),
        ),
        ("npm_config_loglevel", "silent".into()),
        ("npm_config_node_gyp", at("node-gyp/bin/node-gyp.js")),
        ("npm_config_noproxy", "".into()),
        ("npm_config_npm_version", "11.13.0".into()),
        ("npm_config_package", "turbo".into()),
        ("npm_config_prefix", at("selected")),
        (
            "npm_config_user_agent",
            "npm/11.13.0 node/v24.0.0 linux x64".into(),
        ),
        ("npm_config_userconfig", fixture.home.join(".npmrc").into()),
        ("npm_config_yes", "true".into()),
        ("NPM_CONFIG_SCRIPT_SHELL", "/bin/sh".into()),
        (
            "npm_config_user-agent",
            "pnpm/11.0.0 node/v24.0.0 linux x64".into(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect();
    env.push(("npm_lifecycle_event".into(), "npx".into()));
    env
}

/// `pnpm run` (pnpm 10.34) defaults; pnpm 8 to 10.0 export a subset.
fn pnpm10_run_env(fixture: &Fixture) -> Vec<(OsString, OsString)> {
    let at = |path: &str| fixture.directory.path().join(path).into_os_string();
    [
        ("npm_config__jsr_registry", "https://npm.jsr.io/".into()),
        ("npm_config_frozen_lockfile", "".into()),
        ("npm_config_globalconfig", at("home/.config/pnpm/rc")),
        ("npm_config_node_gyp", at("pnpm/node-gyp/bin/node-gyp.js")),
        ("npm_config_npm_globalconfig", at("system/npmrc")),
        ("npm_config_registry", "https://registry.npmjs.org/".into()),
        (
            "npm_config_user_agent",
            "pnpm/10.34.7 npm/? node/v24.17.0 linux x64".into(),
        ),
        ("npm_config_verify_deps_before_run", "false".into()),
        ("pnpm_config_verify_deps_before_run", "false".into()),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}

#[test]
fn wrapper_environments_pass_and_reported_locations_are_inspected() -> TestResult {
    let fixture = Fixture::new()?;
    let env = wrapper_env(&fixture);
    assert_eq!(fixture.check(&env)?.registry(), PUBLIC_NPM_REGISTRY);
    assert!(fixture.check(&pnpm10_run_env(&fixture)).is_ok());
    for registry in ["https://registry.npmjs.org", "HTTPS://Registry.NPMJS.org/"] {
        let env = [("NPM_CONFIG_REGISTRY".into(), registry.into())];
        assert!(fixture.check(&env).is_ok(), "{registry}");
    }
    // Each reported location is followed to the file npm would read.
    let custom = fixture.directory.path().join("custom");
    for (key, value, config, owner) in [
        ("npm_config_userconfig", "npmrc", "npmrc", ConfigOwner::User),
        (
            "npm_config_globalconfig",
            "npmrc",
            "npmrc",
            ConfigOwner::System,
        ),
        (
            "npm_config_npm_globalconfig",
            "npmrc",
            "npmrc",
            ConfigOwner::System,
        ),
        ("npm_config_prefix", "", "etc/npmrc", ConfigOwner::Prefix),
        (
            "NPM_CONFIG_GLOBAL_PREFIX",
            "",
            "etc/npmrc",
            ConfigOwner::Prefix,
        ),
        (
            "npm_config_local_prefix",
            "",
            ".npmrc",
            ConfigOwner::Repository,
        ),
    ] {
        let mut env = env.clone();
        env.push((key.into(), custom.join(value).into_os_string()));
        assert!(fixture.check(&env).is_ok(), "{key}");
        let config = custom.join(config);
        fs::create_dir_all(config.parent().ok_or("parent")?)?;
        fs::write(&config, "registry=https://private.invalid/")?;
        assert_eq!(fixture.check(&env).err(), Some(Error::Configured(owner)));
        fs::remove_dir_all(&custom)?;
    }
    Ok(())
}

#[test]
fn wrapper_environments_with_source_settings_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    for (key, value) in [
        ("npm_config_registry", "https://private.invalid/"),
        ("npm_config_registry", "http://registry.npmjs.org/"),
        ("npm_config_registry", "https://registry.npmjs.org:443/"),
        ("npm_config_registry", "https://private@registry.npmjs.org/"),
        ("npm_config_registry", "https://registry.npmjs.org/private/"),
        ("npm_config_registry", "https://registry.npmjs.org//"),
        ("npm_config_registry", "https://registry.npmjs.org/?private"),
        ("npm_config_registry", "https://registry.npmjs.org."),
        ("npm_config_registry", " https://registry.npmjs.org/"),
        ("npm_config_registry", "https://registry.yarnpkg.com/"),
        ("npm_config__jsr_registry", "https://private.invalid/"),
        ("npm_config__jsr_registry", "https://registry.npmjs.org/"),
        ("npm_config_@jsr:registry", "https://npm.jsr.io/"),
        ("npm_config_@scope:registry", "https://private.invalid/"),
        ("npm_config_proxy", "http://proxy.invalid"),
        ("npm_config_https-proxy", "http://proxy.invalid"),
        ("npm_config_noproxy", "registry.npmjs.org"),
        ("npm_config_cafile", "/private/ca.pem"),
        ("npm_config_strict_ssl", "false"),
        ("npm_config__auth", "private-secret"),
        (
            "npm_config_//registry.npmjs.org/:_authToken",
            "private-secret",
        ),
        ("npm_config_store_dir", "/private/store"),
        ("npm_config_userconfig", "relative/.npmrc"),
        ("npm_config_prefix", ""),
    ] {
        let mut env = wrapper_env(&fixture);
        env.extend(pnpm10_run_env(&fixture));
        env.push((key.into(), value.into()));
        let error = fixture.check(&env).err();
        assert_eq!(error, Some(Error::Environment), "{key}");
        assert!(!format!("{error:?}").contains("private"));
    }
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
            "{\"setup\":{\"tools\":\"skip\",\"mirror\":\"private-secret\"}}",
            "{\"setup\":{\"javascript\":{\"registry\":\"private-secret\"}}}",
            "{\"setup\":{\"tools\":{\"url\":\"private-secret\"}}}",
            "{\"setup\":{\"javascript\":\"skip\"}}",
        ] {
            fs::write(&path, contents)?;
            assert_eq!(
                fixture.check(&[]).err(),
                Some(Error::Configured(ConfigOwner::RootPolicy))
            );
        }
        // Setup applies tool, dependency, and lifecycle policy, none of which
        // configures download sources.
        fs::write(
            &path,
            "{\"setup\":{\"tools\":\"managed\",\"dependencies\":\"skip\",\"lifecycleScripts\":\"\
             skip\",\"rust\":{},\"javascript\":{\"tools\":\"verify-system\"}}}",
        )?;
        assert!(fixture.check(&[]).is_ok());
        // Setup applies additional tools too, alone or with other policy.
        fs::write(
            &path,
            "{\"setup\":{\"additionalTools\":{},\"tools\":\"skip\"}}",
        )?;
        assert!(fixture.check(&[]).is_ok());
        fs::write(&path, "{\"setup\":{\"additionalTools\":{}}}")?;
        assert!(fixture.check(&[]).is_ok());
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

#[cfg(unix)]
#[test]
fn homebrew_bundled_prefix_is_allowed_only_while_its_global_config_is_inspected() -> TestResult {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new()?;
    // GitHub's macOS images: `brew install node@22 && brew link --force`.
    let brew = fixture.directory.path().join("homebrew");
    let keg = brew.join("Cellar/node@22/22.0.0");
    for dir in [
        brew.join("bin"),
        keg.join("bin"),
        brew.join("lib/node_modules"),
    ] {
        fs::create_dir_all(dir)?;
    }
    fs::write(keg.join("bin/node"), b"never execute this")?;
    symlink(keg.join("bin/node"), brew.join("bin/node"))?;
    fs::create_dir_all(keg.join(BUNDLED_NPMRC).parent().ok_or("fixture")?)?;
    symlink(
        keg.join("lib/node_modules/npm"),
        brew.join("lib/node_modules/npm"),
    )?;
    let check = |node: Option<&Path>| {
        inspect_inputs(
            &fixture.root,
            &fixture.home,
            node,
            &[],
            vec![brew.join("etc/npmrc")],
            Some(fixture.directory.path()),
        )
    };
    let homebrew = format!("prefix = {}\n", brew.display());
    for contents in [
        homebrew.clone(),
        format!("; comment\r\n\n\tprefix={}/\r\n", brew.display()),
    ] {
        fs::write(keg.join(BUNDLED_NPMRC), contents)?;
        // Native standalone sees the linked bundled file as System config; the
        // wrapper also reaches the keg's copy through the canonical Node path.
        assert!(check(None).is_ok());
        assert!(check(Some(&brew.join("bin/node"))).is_ok());
    }
    // The global config that prefix selects is still inspected and fails closed.
    fs::create_dir_all(brew.join("etc"))?;
    fs::write(brew.join("etc/npmrc"), b"")?;
    assert_eq!(
        check(None).err(),
        Some(Error::Configured(ConfigOwner::System))
    );
    fs::remove_file(brew.join("etc/npmrc"))?;
    let other = fixture.directory.path().join("other");
    for contents in [
        String::new(),
        format!("{homebrew}registry=https://registry.npmjs.org\n"),
        format!("{homebrew}{homebrew}"),
        format!("prefix = {}\n", other.display()),
        format!("prefix = {}/../homebrew\n", brew.display()),
        format!("prefix = \"{}\"\n", brew.display()),
        format!("prefix = {} ; comment\n", brew.display()),
        format!("prefix[] = {}\n", brew.display()),
        format!("PREFIX = {}\n", brew.display()),
        format!("[section]\n{homebrew}"),
        format!("\u{feff}{homebrew}"),
        format!("prefix = {}\u{85}\n", brew.display()),
        format!("{homebrew}{}", " ".repeat(4096)),
        "prefix = ${HOME}\n".into(),
        "prefix = ~/homebrew\n".into(),
        "prefix = homebrew\n".into(),
        "prefix\n".into(),
        "globalconfig = /etc/npmrc\n".into(),
    ] {
        fs::write(keg.join(BUNDLED_NPMRC), &contents)?;
        assert_eq!(
            check(None).err(),
            Some(Error::Configured(ConfigOwner::System)),
            "{contents:?}"
        );
    }
    // Other layers never get this allowance.
    fs::write(keg.join(BUNDLED_NPMRC), &homebrew)?;
    fs::write(fixture.home.join(".npmrc"), &homebrew)?;
    assert_eq!(
        check(None).err(),
        Some(Error::Configured(ConfigOwner::User))
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_global_npm_prefix_is_inspected_and_required() -> TestResult {
    let fixture = Fixture::new()?;
    let app_data = fixture.directory.path().join("AppData/Roaming");
    let env = [(OsString::from("AppData"), app_data.clone().into_os_string())];
    let system = system_configs(&env)?;
    assert_eq!(system, [app_data.join("npm").join("etc").join("npmrc")]);
    assert_eq!(system_configs(&[]).err(), Some(Error::Locations));
    let relative = [(OsString::from("APPDATA"), OsString::from("Roaming"))];
    assert_eq!(system_configs(&relative).err(), Some(Error::Locations));
    for path in [
        system[0].clone(),
        app_data.join("npm/node_modules/npm/npmrc"),
    ] {
        fs::create_dir_all(path.parent().ok_or("parent")?)?;
        fs::write(&path, "registry=https://mirror.invalid")?;
        let result = inspect_inputs(
            &fixture.root,
            &fixture.home,
            None,
            &[],
            system.clone(),
            Some(fixture.directory.path()),
        );
        assert_eq!(result.err(), Some(Error::Configured(ConfigOwner::System)));
        fs::remove_file(path)?;
    }
    Ok(())
}
