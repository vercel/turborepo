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
fn policy_loads_deterministically_and_round_trips() {
    let setup =
        r#"{"go":{"tools":"skip"},"dependencies":"skip","javascript":{"tools":"verify-system"}}"#;
    assert_eq!(parse(setup).unwrap(), parse(setup).unwrap());
    let raw = root(ALL_FLAGS, setup).unwrap();
    assert_eq!(
        serde_json::to_value(&raw).unwrap()["setup"],
        serde_json::json!({
            "dependencies": "skip",
            "javascript": {"tools": "verify-system"},
            "go": {"tools": "skip"}
        })
    );
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
#[test_case(r#"{"cargo":{"tools":"skip"}}"#; "non canonical toolchain")]
#[test_case(r#"{"typescript":{}}"#; "toolchain alias")]
#[test_case(r#"{"rust":"skip"}"#; "toolchain shorthand")]
#[test_case(r#"{"rust":{"version":"1.80.0"}}"#; "unknown toolchain key")]
#[test_case(r#""skip""#; "non object")]
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
