use std::collections::BTreeMap;

use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
use turborepo_env::EnvironmentVariablePairs;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
// Contains the environment variable inputs for the global hash
pub struct GlobalEnvConfiguration<'a> {
    pub env: &'a [String],
    pub pass_through_env: Option<&'a [String]>,
}

// Contains the environment variables that impacted the global hash
#[derive(Debug, Serialize)]
pub struct GlobalEnvVarSummary<'a> {
    pub specified: GlobalEnvConfiguration<'a>,

    pub configured: Option<EnvironmentVariablePairs>,
    pub inferred: Option<EnvironmentVariablePairs>,
    #[serde(rename = "passthrough")]
    pub pass_through: Option<EnvironmentVariablePairs>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHashSummary<'a> {
    pub root_key: &'static str,
    pub files: BTreeMap<RelativeUnixPathBuf, String>,
    pub hash_of_external_dependencies: &'a str,
    pub hash_of_internal_dependencies: &'a str,
    pub environment_variables: GlobalEnvVarSummary<'a>,
    pub engines: Option<BTreeMap<&'a str, &'a str>>,
}
