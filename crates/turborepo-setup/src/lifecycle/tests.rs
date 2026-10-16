use Scripts::{Allowed, Blocked, Run, Skipped};

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Files<'a> = &'a [(&'a str, &'a str)];

fn npm(version: &str) -> Installer {
    Installer::Npm(Version::parse(version).unwrap())
}
fn pnpm(version: &str) -> Installer {
    Installer::Pnpm(Version::parse(version).unwrap())
}
fn yarn(version: &str) -> Installer {
    Installer::Yarn(Version::parse(version).unwrap())
}
fn bun() -> Installer {
    Installer::Bun(Version::new(1, 2, 0))
}

fn evaluate(files: Files, installer: Installer, request: Request) -> Result<Policy, Error> {
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in files {
        fs::write(dir.path().join(name), contents).unwrap();
    }
    Policy::evaluate(dir.path(), installer, request)
}

fn report(files: Files, installer: Installer, request: Request) -> Result<Vec<String>, Error> {
    evaluate(files, installer, request).map(|policy| policy.report())
}

fn scopes(files: Files, installer: Installer) -> Result<Scopes, Error> {
    let policy = evaluate(files, installer, Request::new(false, None))?;
    // Native policy never adds arguments, so committed settings stay in charge.
    assert!(policy.args().is_empty());
    Ok((policy.project(), policy.dependencies()))
}

#[test]
fn cli_skip_beats_repository_policy_which_beats_native_default() {
    let request = |intent, origin| Request { intent, origin };
    assert_eq!(
        Request::new(true, Some(Intent::Native)),
        request(Intent::Skip, Origin::Cli)
    );
    assert_eq!(
        Request::new(false, Some(Intent::Skip)),
        request(Intent::Skip, Origin::Repository)
    );
    assert_eq!(
        Request::new(false, None),
        request(Intent::Native, Origin::Default)
    );
}

#[test]
fn native_defaults_follow_the_locked_version() -> TestResult {
    for (installer, expected) in [
        (npm("11.6.0"), (Some(Run), Some(Run))),
        (npm("12.0.0"), (Some(Run), Some(Allowed))),
        (pnpm("9.15.0"), (Some(Run), Some(Run))),
        (pnpm("10.18.0"), (Some(Run), Some(Allowed))),
        (yarn("1.22.22"), (Some(Run), Some(Run))),
        (yarn("4.5.0"), (Some(Run), Some(Run))),
        (bun(), (Some(Run), Some(Allowed))),
        (Installer::Uv, (None, Some(Run))),
        (Installer::Cargo, (None, None)),
        (Installer::Go, (None, None)),
    ] {
        assert_eq!(scopes(&[], installer.clone())?, expected, "{installer}");
    }
    Ok(())
}

#[test]
fn committed_settings_stay_authoritative() -> TestResult {
    let off: Files = &[(".npmrc", "; comment\nignore-scripts = true\n")];
    for installer in [npm("10.9.2"), pnpm("10.18.0"), bun()] {
        assert_eq!(scopes(off, installer)?, (Some(Blocked), Some(Blocked)));
    }
    // Modern Yarn ignores .npmrc, and Yarn classic's fallback is not counted.
    for installer in [yarn("1.22.22"), yarn("4.5.0")] {
        assert_eq!(scopes(off, installer)?, (Some(Run), Some(Run)));
    }
    let ws = PNPM_WORKSPACE;
    let cases: [(Files, Installer, Scripts); 12] = [
        (
            &[(ws, "onlyBuiltDependencies: [esbuild]")],
            pnpm("9.15.0"),
            Allowed,
        ),
        (
            &[(
                "package.json",
                r#"{"pnpm":{"neverBuiltDependencies":["a"]}}"#,
            )],
            pnpm("10.18.0"),
            Run,
        ),
        (
            &[(ws, "dangerouslyAllowAllBuilds: true")],
            pnpm("10.18.0"),
            Run,
        ),
        (&[(ws, "ignoreDepScripts: true")], pnpm("9.15.0"), Blocked),
        // pnpm 11 ignores the older list keys in favor of allowBuilds.
        (
            &[(ws, "neverBuiltDependencies: [a]\nallowBuilds: {a: true}")],
            pnpm("11.0.0"),
            Allowed,
        ),
        (&[(ws, "ignoreScripts: true")], pnpm("11.0.0"), Blocked),
        (
            &[("package.json", r#"{"allowScripts":{"a":true}}"#)],
            npm("12.0.0"),
            Allowed,
        ),
        (
            &[(".npmrc", "dangerously-allow-all-scripts=true")],
            npm("12.0.0"),
            Run,
        ),
        (
            &[(".yarnrc", "--ignore-scripts true")],
            yarn("1.22.22"),
            Blocked,
        ),
        (
            &[(".yarnrc.yml", "enableScripts: false")],
            yarn("4.5.0"),
            Blocked,
        ),
        (
            &[("package.json", r#"{"trustedDependencies":[]}"#)],
            bun(),
            Blocked,
        ),
        (
            &[("bunfig.toml", "[install]\nignoreScripts = true")],
            bun(),
            Blocked,
        ),
    ];
    for (files, installer, expected) in cases {
        let found = scopes(files, installer.clone())?;
        assert_eq!(found.1, Some(expected), "{installer} {files:?}");
    }
    Ok(())
}

#[test]
fn skip_uses_each_managers_own_flag() -> TestResult {
    let broad: Files = &[(PNPM_WORKSPACE, "dangerouslyAllowAllBuilds: true")];
    let cases: [(Installer, &[&str], Option<Scripts>); 8] = [
        (npm("10.9.2"), &["--ignore-scripts"], Some(Skipped)),
        (pnpm("10.18.0"), &["--ignore-scripts"], Some(Skipped)),
        (yarn("1.22.22"), &["--ignore-scripts"], Some(Skipped)),
        (yarn("4.5.0"), &["--mode=skip-build"], Some(Skipped)),
        (bun(), &["--ignore-scripts"], Some(Skipped)),
        (Installer::Uv, &["--no-build"], Some(Skipped)),
        // Nothing runs, so there is nothing to suppress.
        (Installer::Cargo, &[], None),
        (Installer::Go, &[], None),
    ];
    for (installer, args, scripts) in cases {
        let policy = evaluate(broad, installer.clone(), Request::new(true, None))?;
        assert_eq!(policy.args(), args, "{installer}");
        assert_eq!(policy.dependencies(), scripts, "{installer}");
        assert!(!policy.runs_code(), "{installer}");
    }
    Ok(())
}

#[test]
fn unsupported_skip_fails_instead_of_claiming_suppression() -> TestResult {
    let error = evaluate(&[], yarn("2.4.3"), Request::new(true, None)).unwrap_err();
    assert!(matches!(error, Error::UnsupportedSkip { .. }));
    assert!(error.to_string().contains("Yarn 3 or later"), "{error}");
    let policy = evaluate(&[], yarn("2.4.3"), Request::new(false, None))?;
    assert!(policy.args().is_empty() && policy.runs_code());
    assert_eq!(
        Installer::javascript(Family::Aube, Version::new(1, 0, 0)),
        None
    );
    Ok(())
}

#[test]
fn unreadable_or_malformed_settings_fail_closed() {
    let cases: [(Files, Installer, &str); 5] = [
        (
            &[(PNPM_WORKSPACE, "onlyBuiltDependencies: [")],
            pnpm("10.18.0"),
            PNPM_WORKSPACE,
        ),
        (&[(PNPM_WORKSPACE, "- a")], pnpm("10.18.0"), PNPM_WORKSPACE),
        (&[("package.json", "{")], npm("12.0.0"), "package.json"),
        (
            &[("package.json", r#"{"trustedDependencies":"a"}"#)],
            bun(),
            "package.json",
        ),
        (&[("bunfig.toml", "[install")], bun(), "bunfig.toml"),
    ];
    for (files, installer, file) in cases {
        let error = evaluate(files, installer, Request::new(false, None)).unwrap_err();
        assert_eq!(error, Error::Settings(file));
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".npmrc")).unwrap();
    let error = Policy::evaluate(dir.path(), npm("10.9.2"), Request::new(false, None));
    assert_eq!(error.unwrap_err(), Error::Settings(".npmrc"));
}

#[test]
fn report_discloses_effective_behavior_before_execution() -> TestResult {
    let native = Request::new(false, None);
    let only: Files = &[(PNPM_WORKSPACE, "onlyBuiltDependencies: [esbuild]")];
    assert_eq!(
        report(only, pnpm("10.18.0"), native)?,
        [
            "pnpm 10.18.0: native pnpm policy; project scripts run; dependency scripts run only \
             for allowed packages (committed: onlyBuiltDependencies in pnpm-workspace.yaml)",
            DISCLOSURE,
        ]
    );
    let yarn_files: Files = &[
        (".yarnrc.yml", "enableScripts: false"),
        (
            "package.json",
            r#"{"dependenciesMeta":{"a":{"built":true}}}"#,
        ),
    ];
    assert_eq!(
        report(yarn_files, yarn("4.5.0"), native)?[0],
        "yarn 4.5.0: native yarn policy; project scripts run; dependency scripts do not run \
         (committed: enableScripts in .yarnrc.yml, dependenciesMeta.*.built in package.json)"
    );
    assert_eq!(
        report(&[], bun(), native)?[0],
        "bun 1.2.0: native bun policy; project scripts run; dependency scripts run only for \
         trusted packages"
    );
    assert_eq!(
        report(&[], Installer::Uv, native)?[0],
        "uv: native uv policy; source distributions without a compatible wheel run their build \
         backends"
    );
    assert_eq!(
        report(&[], Installer::Cargo, native)?,
        ["cargo: dependency setup runs no package code"]
    );
    let repository = Request::new(false, Some(Intent::Skip));
    assert_eq!(
        report(only, pnpm("10.18.0"), repository)?,
        [
            "pnpm 10.18.0: lifecycle scripts skipped (turbo.json setup policy); passing \
             --ignore-scripts to pnpm"
        ]
    );
    assert_eq!(
        report(&[], Installer::Uv, Request::new(true, None))?,
        [
            "uv: source builds refused (--ignore-scripts); passing --no-build to uv, so packages \
             without a compatible wheel fail"
        ]
    );
    Ok(())
}
