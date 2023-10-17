use std::collections::BTreeMap;

use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
use turborepo_env::{DetailedMap, EnvironmentVariableMap, EnvironmentVariablePairs};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
// Contains the environment variable inputs for the global hash
pub struct GlobalEnvConfiguration<'a> {
    pub env: &'a [String],
    pub pass_through_env: &'a [String],
}

// Contains the environment variables that impacted the global hash
#[derive(Debug, Serialize)]
pub struct GlobalEnvVarSummary<'a> {
    pub specified: GlobalEnvConfiguration<'a>,

    pub configured: EnvironmentVariablePairs,
    pub inferred: EnvironmentVariablePairs,
    #[serde(rename = "passthrough")]
    pub pass_through: EnvironmentVariablePairs,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHashSummary<'a> {
    pub root_key: &'static str,
    pub files: BTreeMap<RelativeUnixPathBuf, String>,
    pub hash_of_external_dependencies: &'a str,
    pub global_dot_env: &'a [RelativeUnixPathBuf],
    pub environment_variables: GlobalEnvVarSummary<'a>,
}

impl<'a> GlobalHashSummary<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        global_cache_key: &'static str,
        global_file_hash_map: BTreeMap<RelativeUnixPathBuf, String>,
        root_external_deps_hash: Option<&'a str>,
        global_env: &'a [String],
        global_pass_through_env: &'a [String],
        global_dot_env: &'a [RelativeUnixPathBuf],
        resolved_env_vars: DetailedMap,
        resolved_pass_through_env_vars: EnvironmentVariableMap,
    ) -> Self {
        Self {
            root_key: global_cache_key,
            files: global_file_hash_map,
            // This can be empty in single package mode
            hash_of_external_dependencies: root_external_deps_hash.unwrap_or_default(),

            environment_variables: GlobalEnvVarSummary {
                specified: GlobalEnvConfiguration {
                    env: global_env,
                    pass_through_env: global_pass_through_env,
                },
                configured: resolved_env_vars.by_source.explicit.to_secret_hashable(),
                inferred: resolved_env_vars.by_source.matching.to_secret_hashable(),
                pass_through: resolved_pass_through_env_vars.to_secret_hashable(),
            },

            global_dot_env: global_dot_env,
        }
    }
}
