#![deny(clippy::all)]

mod vendors;

use std::env;

use lazy_static::lazy_static;

pub use crate::vendors::Vendor;
use crate::vendors::VENDORS;

lazy_static! {
    pub static ref IS_CI: bool = {
        // We purposefully don't do `is_err()` because the Go version
        // returns false for both an unset env variable
        // and an env variable set to the empty string.
        let build_id = env::var("BUILD_ID").unwrap_or_default();
        let build_number = env::var("BUILD_NUMBER").unwrap_or_default();
        let ci = env::var("CI").unwrap_or_default();
        let ci_app_id = env::var("CI_APP_ID").unwrap_or_default();
        let ci_build_id = env::var("CI_BUILD_ID").unwrap_or_default();
        let ci_build_number = env::var("CI_BUILD_NUMBER").unwrap_or_default();
        let ci_name = env::var("CI_NAME").unwrap_or_default();
        let continuous_integration = env::var("CONTINUOUS_INTEGRATION").unwrap_or_default();
        let run_id = env::var("RUN_ID").unwrap_or_default();
        let teamcity_version = env::var("TEAMCITY_VERSION").unwrap_or_default();

        !build_id.is_empty()
            || !build_number.is_empty()
            || !ci.is_empty()
            || !ci_app_id.is_empty()
            || !ci_build_id.is_empty()
            || !ci_build_number.is_empty()
            || !ci_name.is_empty()
            || !continuous_integration.is_empty()
            || !run_id.is_empty()
            || !teamcity_version.is_empty()
    };
}

impl Vendor {
    // Returns info about a CI vendor
    pub fn get_info() -> Option<Vendor> {
        for env in VENDORS.iter() {
            if let Some(eval_env) = &env.eval_env {
                for (name, expected_value) in eval_env {
                    if matches!(env::var(name), Ok(env_value) if *expected_value == env_value) {
                        return Some(env.clone());
                    }
                }
            } else if !env.env.any.is_empty() {
                for env_var in &env.env.any {
                    if matches!(env::var(env_var), Ok(v) if !v.is_empty()) {
                        return Some(env.clone());
                    }
                }
            } else if !env.env.all.is_empty() {
                let mut all = true;
                for env_var in &env.env.all {
                    if env::var(env_var).unwrap_or_default().is_empty() {
                        all = false;
                        break;
                    }
                }

                if all {
                    return Some(env.clone());
                }
            }
        }

        None
    }

    pub fn get_constant() -> Option<&'static str> {
        Self::get_info().map(|v| v.constant)
    }
}
