//! Native managed-run handshake. Schema readability is not execution support.

use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use turborepo_platform::{Architecture, OperatingSystem, Platform};

pub const QUERY_FLAG: &str = "--__internal-managed-run-capabilities";
pub const ABI_VERSION: u32 = 1;
/// Includes the terminating newline. Consumers must bound reads before parsing.
pub const MAX_RESPONSE_BYTES: usize = 4096;
const MAX_SCHEMAS: usize = 64;
const MAX_FEATURES: usize = 64;

/// The prelaunch `turbo.lock` schema this build activates.
pub const EXPERIMENTAL_LOCK_SCHEMA: u32 = 0;
/// What this build activates for that schema: each lock adapter ID, locked
/// `verify-system` tools checked on PATH, and `clone-local` selections that
/// `turbo setup --no-lock` records without a turbo.lock. Sorted. A name's
/// meaning is frozen once a build advertises it: a lock or activation change
/// that an older peer would misread needs a new name, so that peer is refused
/// instead.
pub const FEATURES: &[&str] = &[
    "artifact",
    "bun",
    "clone-local",
    "go",
    "node",
    "npm",
    "pnpm",
    "python",
    "rust",
    "rustup",
    "uv",
    "verify-system",
    "yarn",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("managed-run capability response exceeds its byte limit")]
    TooLarge,
    #[error("invalid managed-run capability JSON")]
    Json,
    #[error("unsupported managed-run capability ABI {0}")]
    Abi(u32),
    #[error("invalid managed-run capability metadata")]
    Metadata,
}

/// Validated response; raw deserialization cannot bypass the ABI/metadata
/// checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities(Report);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Report {
    abi_version: u32,
    /// Diagnostic only: never use the CLI version as a compatibility floor.
    cli_version: String,
    /// Sorted, unique lock schemas with the COMPLETE public managed-run
    /// contract. Empty until launch freezes a lock schema; prelaunch support
    /// is the feature-scoped `experimental_setup` below.
    managed_run_schemas: Vec<u32>,
    /// Older readers reject this extension rather than treating it as
    /// complete schema support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    experimental_setup: Option<Experimental>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Experimental {
    lock_schema: u32,
    /// The build's OS and architecture. Peers share one host and classify its
    /// C library at runtime, so equal targets select the same artifacts.
    target: String,
    /// Sorted and unique. Unknown names are newer features, never errors.
    features: Vec<String>,
}

/// What a managed selection needs from the CLI that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    schema: u32,
    features: BTreeSet<String>,
}

impl Requirement {
    pub fn new<S: Into<String>>(schema: u32, features: impl IntoIterator<Item = S>) -> Self {
        Self {
            schema,
            features: features.into_iter().map(Into::into).collect(),
        }
    }

    pub fn schema(&self) -> u32 {
        self.schema
    }

    pub fn features(&self) -> impl Iterator<Item = &str> {
        self.features.iter().map(String::as_str)
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let features = self.features().collect::<Vec<_>>().join(", ");
        if self.features.contains("clone-local") {
            write!(
                f,
                "this clone's turbo setup --no-lock tools (schema {}: {features})",
                self.schema
            )
        } else {
            write!(f, "turbo.lock schema {} ({features})", self.schema)
        }
    }
}

/// Builds whose host the managed-run context accepts. Linux builds cover glibc
/// and musl hosts alike: both peers classify the host's C library at runtime,
/// and activation admits only verify-system tools on musl. Released Linux
/// binaries are static musl builds, so this never depends on the build's own
/// C library. Windows arm64 is unqualified.
fn target() -> Option<&'static str> {
    use Architecture::{Arm64, X64};
    use OperatingSystem::{Linux, Macos, Windows};
    let platform = Platform::current();
    match (platform.os(), platform.arch()) {
        (Macos, Arm64) => Some("macos-arm64"),
        (Macos, X64) => Some("macos-x64"),
        (Linux, Arm64) => Some("linux-arm64"),
        (Linux, X64) => Some("linux-x64"),
        (Windows, X64) => Some("windows-x64"),
        _ => None,
    }
}

fn feature(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Capabilities {
    /// No managed execution support, regardless of readable schemas/version.
    pub fn unsupported(cli_version: &str) -> Self {
        Self(Report {
            abi_version: ABI_VERSION,
            cli_version: cli_version.to_owned(),
            managed_run_schemas: Vec::new(),
            experimental_setup: None,
        })
    }

    /// This build's report: every feature it activates on a claimed target.
    pub fn current(cli_version: &str) -> Self {
        let mut report = Self::unsupported(cli_version);
        report.0.experimental_setup = target().map(|target| Experimental {
            lock_schema: EXPERIMENTAL_LOCK_SCHEMA,
            target: target.to_owned(),
            features: FEATURES.iter().map(|&name| name.to_owned()).collect(),
        });
        report
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(Error::TooLarge);
        }
        let report = Self(serde_json::from_slice(bytes).map_err(|_| Error::Json)?);
        report.validate()?;
        Ok(report)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(&self.0).map_err(|_| Error::Json)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(bytes)
    }

    pub fn cli_version(&self) -> &str {
        &self.0.cli_version
    }

    pub fn supports_schema(&self, schema: u32) -> bool {
        self.0.managed_run_schemas.binary_search(&schema).is_ok()
    }

    /// Why this CLI cannot run `requirement` on this host; `None` when it
    /// can. Complete schema support, or the prelaunch scope for the same
    /// schema and target with every required feature. Never a version floor.
    pub fn shortfall(&self, requirement: &Requirement) -> Option<String> {
        if self.supports_schema(requirement.schema) {
            return None;
        }
        let version = self.cli_version();
        let Some(scope) = &self.0.experimental_setup else {
            return Some(format!("turbo {version} does not support managed runs"));
        };
        if scope.lock_schema != requirement.schema {
            return Some(format!(
                "turbo {version} supports managed runs for turbo.lock schema {}, not {}",
                scope.lock_schema, requirement.schema
            ));
        }
        let host = target();
        if host != Some(scope.target.as_str()) {
            return Some(format!(
                "turbo {version} is built for {}, but this host's turbo is built for {}",
                scope.target,
                host.unwrap_or("an unsupported platform")
            ));
        }
        let missing: Vec<_> = requirement
            .features()
            .filter(|name| {
                scope
                    .features
                    .binary_search_by(|f| f.as_str().cmp(name))
                    .is_err()
            })
            .collect();
        (!missing.is_empty()).then(|| {
            format!(
                "turbo {version} lacks managed-run support for {}",
                missing.join(", ")
            )
        })
    }

    fn validate(&self) -> Result<(), Error> {
        let report = &self.0;
        if report.abi_version != ABI_VERSION {
            return Err(Error::Abi(report.abi_version));
        }
        let sorted = |values: &[String]| values.windows(2).all(|pair| pair[0] < pair[1]);
        if report.cli_version.is_empty()
            || report.cli_version.len() > 128
            || !report.cli_version.bytes().all(|b| b.is_ascii_graphic())
            || report.managed_run_schemas.len() > MAX_SCHEMAS
            || report.experimental_setup.as_ref().is_some_and(|scope| {
                scope.target.is_empty()
                    || scope.target.len() > 64
                    || !scope.target.bytes().all(|b| b.is_ascii_graphic())
                    || scope.features.len() > MAX_FEATURES
                    || !scope.features.iter().all(|name| feature(name))
                    || !sorted(&scope.features)
            })
            || report
                .managed_run_schemas
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::Metadata);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn current_report_is_versioned_and_claims_no_schema_support() {
        let report = Capabilities::unsupported("2.11.8-canary.0");
        let bytes = report.encode().unwrap();
        assert_eq!(
            bytes,
            b"{\"abiVersion\":1,\"cliVersion\":\"2.11.8-canary.0\",\"managedRunSchemas\":[]}\n"
        );
        assert_eq!(Capabilities::parse(&bytes).unwrap(), report);
        for schema in [0, 1, u32::MAX] {
            assert!(!report.supports_schema(schema));
        }
    }

    fn peer(target: &str, features: &str) -> Capabilities {
        let scope = format!(r#"{{"lockSchema":0,"target":"{target}","features":[{features}]}}"#);
        let json = format!(
            r#"{{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"experimentalSetup":{scope}}}"#
        );
        Capabilities::parse(json.as_bytes()).unwrap()
    }

    #[test]
    fn experimental_scope_needs_the_schema_target_and_every_required_feature() {
        let current = Capabilities::current("current");
        let bytes = current.encode().unwrap();
        assert_eq!(Capabilities::parse(&bytes).unwrap(), current);
        assert!(!current.supports_schema(0));
        assert!(FEATURES.windows(2).all(|pair| pair[0] < pair[1]));
        let every = Requirement::new(EXPERIMENTAL_LOCK_SCHEMA, FEATURES.iter().copied());
        assert_eq!(current.shortfall(&every).is_none(), target().is_some());
        assert!(current.shortfall(&Requirement::new(1, ["node"])).is_some());
        let Some(host) = target() else { return };
        // Newer peers may advertise features this reader does not know.
        let peer_here = peer(host, r#""future-adapter","node","pnpm""#);
        assert_eq!(
            peer_here.shortfall(&Requirement::new(0, ["node", "pnpm"])),
            None
        );
        let clone_local = Requirement::new(0, ["clone-local", "go", "node"]);
        assert_eq!(
            peer_here.shortfall(&clone_local).as_deref(),
            Some("turbo v lacks managed-run support for clone-local, go")
        );
        let next = peer_here.shortfall(&Requirement::new(1, ["node"])).unwrap();
        assert!(next.contains("schema 0, not 1"), "{next}");
        // A peer built for another target never satisfies this host's turbo.
        for other in [
            "macos-arm64",
            "macos-x64",
            "linux-x64",
            "linux-x64-gnu",
            "windows-x64",
        ] {
            let shortfall = peer(other, r#""node""#).shortfall(&Requirement::new(0, ["node"]));
            assert_eq!(shortfall.is_none(), other == host, "{other}");
        }
        assert_eq!(
            clone_local.to_string(),
            "this clone's turbo setup --no-lock tools (schema 0: clone-local, go, node)"
        );
        assert_eq!(
            Requirement::new(0, ["pnpm", "node"]).to_string(),
            "turbo.lock schema 0 (node, pnpm)"
        );
    }

    #[test]
    fn compatibility_is_explicit_schema_membership_not_a_cli_version_floor() {
        // Synthetic peer responses, not capabilities of this partially shipped CLI.
        let peer = Capabilities::parse(
            br#"{"abiVersion":1,"cliVersion":"old-build","managedRunSchemas":[0,2]}"#,
        )
        .unwrap();
        assert!(peer.supports_schema(0));
        assert!(peer.supports_schema(2));
        assert!(!peer.supports_schema(1));
        assert_eq!(peer.cli_version(), "old-build");
        assert!(!Capabilities::unsupported("9999.0.0").supports_schema(0));
    }

    #[test]
    fn malformed_missing_duplicate_and_unknown_fields_fail_closed() {
        for bytes in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"extra":true}"#,
            r#"{"abiVersion":1,"abiVersion":1,"cliVersion":"v","managedRunSchemas":[]}"#,
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"managedRunSchemas":[0]}"#,
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[-1]}"#,
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[4294967296]}"#,
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"experimentalSetup":{}}"#,
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"experimentalSetup":{"lockSchema":0,"target":"t","features":[],"complete":true}}"#,
            // The earlier Node/pnpm-only scope is not this contract.
            r#"{"abiVersion":1,"cliVersion":"v","managedRunSchemas":[],"experimentalNodePnpm":{"contractVersion":1,"lockSchema":0,"platform":"macos-arm64"}}"#,
        ] {
            assert_eq!(Capabilities::parse(bytes.as_bytes()), Err(Error::Json));
        }
    }

    #[test]
    fn unknown_abi_and_invalid_metadata_fail_closed() {
        assert_eq!(
            Capabilities::parse(br#"{"abiVersion":2,"cliVersion":"v","managedRunSchemas":[]}"#),
            Err(Error::Abi(2))
        );
        for (version, schemas) in [
            ("".to_owned(), vec![]),
            ("v\nsecret".to_owned(), vec![]),
            ("v".repeat(129), vec![]),
            ("v".to_owned(), vec![1, 1]),
            ("v".to_owned(), vec![2, 1]),
            ("v".to_owned(), (0..65).collect()),
        ] {
            let report = Capabilities(Report {
                abi_version: ABI_VERSION,
                cli_version: version,
                managed_run_schemas: schemas,
                experimental_setup: None,
            });
            assert_eq!(report.encode(), Err(Error::Metadata));
            let bytes = serde_json::to_vec(&report.0).unwrap();
            assert_eq!(Capabilities::parse(&bytes), Err(Error::Metadata));
        }
        let many: Vec<_> = (10..75).map(|i| format!("f{i}")).collect();
        for (target, features) in [
            ("linux x64", vec!["node"]),
            ("linux-x64", vec!["pnpm", "node"]),
            ("linux-x64", vec!["node", "node"]),
            ("linux-x64", vec!["Node"]),
            ("linux-x64", vec![""]),
        ]
        .into_iter()
        .map(|(t, f)| (t, f.into_iter().map(String::from).collect()))
        .chain([("linux-x64", many)])
        {
            let mut report = Capabilities::unsupported("v");
            report.0.experimental_setup = Some(Experimental {
                lock_schema: 0,
                target: target.into(),
                features,
            });
            assert_eq!(report.encode(), Err(Error::Metadata));
        }
    }

    #[test]
    fn byte_limit_is_checked_before_json_and_includes_trailing_whitespace() {
        let mut bytes = Capabilities::unsupported("v").encode().unwrap();
        bytes.resize(MAX_RESPONSE_BYTES, b' ');
        assert!(Capabilities::parse(&bytes).is_ok());
        bytes.push(b' ');
        assert_eq!(Capabilities::parse(&bytes), Err(Error::TooLarge));
        assert_eq!(
            Capabilities::parse(&vec![b'x'; MAX_RESPONSE_BYTES + 1]),
            Err(Error::TooLarge)
        );
    }
}
