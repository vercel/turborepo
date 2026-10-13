//! Native managed-run handshake. Schema readability is not execution support.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const QUERY_FLAG: &str = "--__internal-managed-run-capabilities";
pub const ABI_VERSION: u32 = 1;
/// Includes the terminating newline. Consumers must bound reads before parsing.
pub const MAX_RESPONSE_BYTES: usize = 4096;
const MAX_SCHEMAS: usize = 64;

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
    /// Sorted, unique lock schemas with the COMPLETE managed-run contract:
    /// activation before probes, drift validation, strict/loose task env and
    /// portable toolchain hashing. Accepting a lock schema alone is
    /// insufficient.
    managed_run_schemas: Vec<u32>,
}

impl Capabilities {
    /// The current CLI has no complete managed-run implementation. Keep this
    /// empty until all guarantees above are delivered and verified; prelaunch
    /// schema 0 parsing must not accidentally advertise execution support.
    pub fn unsupported(cli_version: &str) -> Self {
        Self(Report {
            abi_version: ABI_VERSION,
            cli_version: cli_version.to_owned(),
            managed_run_schemas: Vec::new(),
        })
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

    fn validate(&self) -> Result<(), Error> {
        let report = &self.0;
        if report.abi_version != ABI_VERSION {
            return Err(Error::Abi(report.abi_version));
        }
        if report.cli_version.is_empty()
            || report.cli_version.len() > 128
            || !report.cli_version.bytes().all(|b| b.is_ascii_graphic())
            || report.managed_run_schemas.len() > MAX_SCHEMAS
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
            });
            assert_eq!(report.encode(), Err(Error::Metadata));
            let bytes = serde_json::to_vec(&report.0).unwrap();
            assert_eq!(Capabilities::parse(&bytes), Err(Error::Metadata));
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
