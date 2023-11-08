use std::collections::BTreeMap;

use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
use turborepo_env::EnvironmentVariablePairs;

use crate::run::{global_hash::GlobalHashableInputs, summary::Error};

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
    pub global_dot_env: Option<&'a [RelativeUnixPathBuf]>,
    pub environment_variables: GlobalEnvVarSummary<'a>,
}

impl<'a> TryFrom<GlobalHashableInputs<'a>> for GlobalHashSummary<'a> {
    type Error = Error;
    #[allow(clippy::too_many_arguments)]
    fn try_from(global_hashable_inputs: GlobalHashableInputs<'a>) -> Result<Self, Self::Error> {
        let GlobalHashableInputs {
            global_cache_key,
            global_file_hash_map,
            root_external_dependencies_hash,
            env,
            resolved_env_vars,
            pass_through_env,
            dot_env,
            env_at_execution_start,
            ..
        } = global_hashable_inputs;

        let pass_through = pass_through_env
            .map(
                |pass_through_env| -> Result<EnvironmentVariablePairs, Error> {
                    Ok(env_at_execution_start
                        .from_wildcards(pass_through_env)
                        .map_err(Error::Env)?
                        .to_secret_hashable())
                },
            )
            .transpose()?;

        Ok(Self {
            root_key: global_cache_key,
            files: global_file_hash_map.into_iter().collect(),
            // This can be empty in single package mode
            hash_of_external_dependencies: root_external_dependencies_hash.unwrap_or_default(),
            environment_variables: GlobalEnvVarSummary {
                specified: GlobalEnvConfiguration {
                    env,
                    pass_through_env,
                },
                configured: resolved_env_vars
                    .as_ref()
                    .map(|vars| vars.by_source.explicit.to_secret_hashable()),
                inferred: resolved_env_vars
                    .as_ref()
                    .map(|vars| vars.by_source.matching.to_secret_hashable()),
                pass_through,
            },

            global_dot_env: dot_env,
        })
    }
}
