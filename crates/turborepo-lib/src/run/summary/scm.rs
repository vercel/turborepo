use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_scm::SCM;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum SCMType {
    Git,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SCMState {
    #[serde(rename = "type")]
    ty: SCMType,
    pub(crate) sha: Option<String>,
    pub(crate) branch: Option<String>,
}

impl SCMState {
    pub fn get(env_vars: &EnvironmentVariableMap, scm: &SCM, dir: &AbsoluteSystemPath) -> Self {
        let mut state = SCMState {
            ty: SCMType::Git,
            sha: None,
            branch: None,
        };

        if turborepo_ci::is_ci() {
            if let Some(vendor) = Vendor::infer() {
                if let Some(sha_env_var) = vendor.sha_env_var {
                    state.sha = env_vars.get(sha_env_var).cloned()
                }

                if let Some(branch_env_var) = vendor.branch_env_var {
                    state.branch = env_vars.get(branch_env_var).cloned()
                }
            }
        }

        // Fall back to using `git`
        if state.branch.is_none() && state.sha.is_none() {
            if state.branch.is_none() {
                state.branch = scm.get_current_branch(dir).ok();
            }
            if state.sha.is_none() {
                state.sha = scm.get_current_sha(dir).ok();
            }
        }

        state
    }
}
