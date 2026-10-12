use DependencyPolicy::{Locked, Skip as SkipDependencies};
use ToolPolicy::{Managed, Skip, VerifySystem};
use miette::Diagnostic;
use test_case::test_case;

use super::*;
use crate::{RawPackageTurboJson, RawRootTurboJson, RawTurboJson, TurboJson};

const ALL_FLAGS: &str = r#"{"experimentalSetup":true,"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true}"#;

fn root(flags: &str, setup: &str) -> Result<RawTurboJson, Error> {
    let json = format!(r#"{{"futureFlags":{flags},"setup":{setup}}}"#);
    RawRootTurboJson::parse(&json, "turbo.json")?.try_into()
}

fn parse(setup: &str) -> Result<SetupPolicy, Error> {
    let raw = root(ALL_FLAGS, setup)?;
    let flags = raw.future_flags.clone().unwrap().into_inner();
    SetupPolicy::from_raw(raw.setup.as_ref(), &flags)
}

type Effective = (Option<ToolPolicy>, Option<DependencyPolicy>);

fn effective<const N: usize>(policy: &SetupPolicy, ids: [ToolchainId; N]) -> [Effective; N] {
    ids.map(|id| {
        let policy = policy.toolchain(&id);
        (policy.tools, policy.dependencies)
    })
}

#[test]
fn absent_or_empty_policy_leaves_every_value_inferred() {
    let absent = SetupPolicy::from_raw(None, &FutureFlags::default()).unwrap();
    for policy in [
        absent,
        parse("{}").unwrap(),
        parse(r#"{"rust":{}}"#).unwrap(),
    ] {
        assert!(!policy.is_configured());
        for id in [
            ToolchainId::JAVASCRIPT,
            ToolchainId::GO,
            ToolchainId::new("java"),
        ] {
            assert_eq!(policy.toolchain(&id), ToolchainSetupPolicy::default());
        }
    }
}

#[test]
fn toolchain_values_override_root_values() {
    let policy = parse(
        r#"{"tools":"managed","dependencies":"locked",
            "javascript":{"tools":"verify-system"},
            "rust":{"tools":"skip"},
            "python":{"dependencies":"skip"},
            "go":{"tools":"skip","dependencies":"skip"}}"#,
    )
    .unwrap();
    assert!(policy.is_configured());
    let ids = [
        ToolchainId::JAVASCRIPT,
        ToolchainId::RUST,
        ToolchainId::PYTHON,
        ToolchainId::GO,
    ];
    // Skipped tools skip dependencies instead of inheriting the root value.
    assert_eq!(
        effective(&policy, ids),
        [
            (Some(VerifySystem), Some(Locked)),
            (Some(Skip), Some(SkipDependencies)),
            (Some(Managed), Some(SkipDependencies)),
            (Some(Skip), Some(SkipDependencies)),
        ]
    );

    let tools_only = parse(r#"{"dependencies":"skip"}"#).unwrap();
    assert_eq!(
        effective(&tools_only, [ToolchainId::RUST]),
        [(None, Some(SkipDependencies))]
    );
    let only_javascript = parse(r#"{"tools":"skip","javascript":{"tools":"managed"}}"#).unwrap();
    assert_eq!(
        effective(
            &only_javascript,
            [ToolchainId::JAVASCRIPT, ToolchainId::new("java")]
        ),
        [(Some(Managed), None), (Some(Skip), Some(SkipDependencies))]
    );
}

#[test]
fn lifecycle_scripts_resolve_per_toolchain() {
    let policy =
        parse(r#"{"lifecycleScripts":"skip","python":{"lifecycleScripts":"native"}}"#).unwrap();
    assert!(policy.is_configured());
    let scripts = |id| policy.toolchain(&id).lifecycle_scripts;
    assert_eq!(
        scripts(ToolchainId::JAVASCRIPT),
        Some(LifecycleScripts::Skip)
    );
    assert_eq!(scripts(ToolchainId::PYTHON), Some(LifecycleScripts::Native));
    let unset = parse(r#"{"javascript":{"tools":"managed"}}"#).unwrap();
    assert_eq!(
        unset.toolchain(&ToolchainId::JAVASCRIPT).lifecycle_scripts,
        None
    );
}

#[test]
fn policy_loads_deterministically_and_round_trips() {
    let setup = r#"{"go":{"tools":"skip"},"dependencies":"skip","javascript":{"tools":"verify-system"},
        "mirrors":{"npmRegistry":"https://npm.example/registry"}}"#;
    assert_eq!(parse(setup).unwrap(), parse(setup).unwrap());
    let raw = root(ALL_FLAGS, setup).unwrap();
    assert_eq!(
        serde_json::to_value(&raw).unwrap()["setup"],
        serde_json::json!({
            "dependencies": "skip",
            "javascript": {"tools": "verify-system"},
            "go": {"tools": "skip"},
            "mirrors": {"npmRegistry": "https://npm.example/registry"}
        })
    );
}

#[test]
fn mirrors_keep_their_bases_and_count_as_policy() {
    let policy = parse(
        r#"{"mirrors":{"node":"https://mirror.example/nodejs/dist/","npmRegistry":"https://mirror.example:8443/npm",
            "bun":"https://mirror.example/github","uv":"https://mirror.example/astral","python":"https://mirror.example/astral"}}"#,
    )
    .unwrap();
    assert!(policy.is_configured());
    assert_eq!(
        policy.mirrors(),
        &SetupMirrors {
            node: Some("https://mirror.example/nodejs/dist/".into()),
            npm_registry: Some("https://mirror.example:8443/npm".into()),
            bun: Some("https://mirror.example/github".into()),
            uv: Some("https://mirror.example/astral".into()),
            python: Some("https://mirror.example/astral".into()),
        }
    );
    let empty = parse(r#"{"mirrors":{}}"#).unwrap();
    assert_eq!(empty.mirrors(), &SetupMirrors::default());
    assert!(!empty.is_configured());
}

#[test_case("node", "https://user:s3cret@mirror.example/dist"; "userinfo")]
#[test_case("node", "https://s3cret@mirror.example/dist"; "username")]
#[test_case("npmRegistry", "https://@mirror.example/s3cret"; "empty userinfo")]
#[test_case("npmRegistry", "https://mirror.example/npm?token=s3cret"; "query")]
#[test_case("node", "https://mirror.example/dist#s3cret"; "fragment")]
#[test_case("node", "http://mirror.example/s3cret"; "plain http")]
#[test_case("npmRegistry", "mirror.example/s3cret"; "relative")]
#[test_case("node", ""; "empty")]
#[test_case("bun", "http://mirror.example/s3cret"; "plain http bun")]
#[test_case("python", "https://s3cret@mirror.example/astral"; "python userinfo")]
fn invalid_mirrors_point_at_masked_values(key: &str, value: &str) {
    // The other mirror is valid but still never rendered.
    let other = if key == "node" { "npmRegistry" } else { "node" };
    let setup =
        format!(r#"{{"mirrors":{{"{key}":"{value}","{other}":"https://ok.example/s3cret"}}}}"#);
    let error = parse(&setup).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains(&format!("setup.mirrors.{key}")),
        "{message}"
    );
    let Error::SetupInvalidMirror {
        span: Some(span),
        text,
        ..
    } = error
    else {
        panic!("expected a mirror diagnostic: {message}");
    };
    let source = text.inner();
    let labeled = &source[span.offset()..span.offset() + span.len()];
    assert!(labeled.bytes().all(|b| b == b'*'), "{source}");
    assert!(source.contains(r#""mirrors""#) && !source.contains("s3cret"));
    assert!(!message.contains("s3cret") && !message.contains("example"));
}

#[test_case(r#"{"tools":"skip","dependencies":"locked"}"#; "root")]
#[test_case(r#"{"rust":{"tools":"skip","dependencies":"locked"}}"#; "toolchain")]
#[test_case(r#"{"tools":"skip","go":{"dependencies":"locked"}}"#; "inherited skip")]
fn locked_dependencies_require_tools(setup: &str) {
    let error = parse(setup).unwrap_err();
    assert!(
        matches!(
            error,
            Error::SetupLockedDependenciesWithSkippedTools { span: Some(_), .. }
        ),
        "{error:?}"
    );
}

#[test_case(r#"{"tools":"system"}"#; "unknown tool policy")]
#[test_case(r#"{"dependencies":"frozen"}"#; "unknown dependency policy")]
#[test_case(r#"{"lifecycleScripts":"allow"}"#; "unknown lifecycle policy")]
#[test_case(r#"{"cargo":{"tools":"skip"}}"#; "non canonical toolchain")]
#[test_case(r#"{"typescript":{}}"#; "toolchain alias")]
#[test_case(r#"{"rust":"skip"}"#; "toolchain shorthand")]
#[test_case(r#"{"rust":{"version":"1.80.0"}}"#; "unknown toolchain key")]
#[test_case(r#""skip""#; "non object")]
#[test_case(r#"{"mirrors":{"yarn":"https://mirror.example"}}"#; "unknown mirror")]
#[test_case(r#"{"mirrors":{"node":1}}"#; "non string mirror")]
#[test_case(r#"{"mirrors":"https://mirror.example"}"#; "mirror shorthand")]
fn invalid_values_identify_their_source(setup: &str) {
    let Err(Error::Parse(error)) = parse(setup) else {
        panic!("expected a parse error for {setup}");
    };
    assert!(!error.diagnostics.is_empty());
    for diagnostic in &error.diagnostics {
        assert!(diagnostic.labels().is_some(), "{diagnostic:?}");
    }
}

#[test_case("{}"; "flags omitted")]
#[test_case(r#"{"experimentalSetup":false}"#; "flag disabled")]
fn setup_requires_the_future_flag(flags: &str) {
    let error = root(flags, r#"{"tools":"skip"}"#).unwrap_err();
    assert!(
        matches!(error, Error::SetupRequiresFlag { span: Some(_), .. }),
        "{error:?}"
    );
}

#[test_case("rust", "experimentalCargoWorkspaces")]
#[test_case("python", "experimentalPythonWorkspaces")]
#[test_case("go", "experimentalGoWorkspaces")]
fn gated_toolchains_require_their_workspace_flag(key: &str, flag: &str) {
    let setup = format!(r#"{{"{key}":{{}}}}"#);
    let error = root(r#"{"experimentalSetup":true}"#, &setup).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::SetupToolchainRequiresFlag { key: k, flag: f, span: Some(_), .. }
                if k == key && *f == flag
        ),
        "{error:?}"
    );
}

#[test]
fn setup_is_root_only() {
    let json = r#"{"extends":["//"],"setup":{"tools":"skip"}}"#;
    let raw = RawPackageTurboJson::parse(json, "packages/web/turbo.json").unwrap();
    let error = TurboJson::try_from(RawTurboJson::from(raw)).unwrap_err();
    assert!(
        matches!(error, Error::SetupInPackage { span: Some(_), .. }),
        "{error:?}"
    );
}
