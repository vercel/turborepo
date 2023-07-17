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

    #[allow(dead_code)]
    fn get_name() -> Option<&'static str> {
        Self::get_info().map(|v| v.name)
    }

    pub fn get_constant() -> Option<&'static str> {
        Self::get_info().map(|v| v.constant)
    }
}

#[cfg(test)]
mod tests {
    use tracing::info;

    use super::*;
    use crate::Vendor;

    fn get_vendor(name: &str) -> Vendor {
        for v in VENDORS.iter() {
            if v.name == name {
                return v.clone();
            }
        }

        unreachable!("vendor not found")
    }

    struct TestCase {
        name: String,
        set_env: Vec<String>,
        want: Option<Vendor>,
    }

    #[test]
    fn test_info() {
        // This is purposefully *not* using test_case
        // because we don't want to run these tests in parallel
        // due to race conditions with environment variables.
        let tests = vec![
            TestCase {
                name: "AppVeyor".to_string(),
                set_env: vec!["APPVEYOR".to_string()],
                want: Some(get_vendor("AppVeyor")),
            },
            TestCase {
                name: "Vercel".to_string(),
                set_env: vec!["VERCEL".to_string(), "NOW_BUILDER".to_string()],
                want: Some(get_vendor("Vercel")),
            },
            TestCase {
                name: "Render".to_string(),
                set_env: vec!["RENDER".to_string()],
                want: Some(get_vendor("Render")),
            },
            TestCase {
                name: "Netlify".to_string(),
                set_env: vec!["NETLIFY".to_string()],
                want: Some(get_vendor("Netlify CI")),
            },
            TestCase {
                name: "Jenkins".to_string(),
                set_env: vec!["BUILD_ID".to_string(), "JENKINS_URL".to_string()],
                want: Some(get_vendor("Jenkins")),
            },
            TestCase {
                name: "Jenkins - failing".to_string(),
                set_env: vec!["BUILD_ID".to_string()],
                want: None,
            },
            TestCase {
                name: "GitHub Actions".to_string(),
                set_env: vec!["GITHUB_ACTIONS".to_string()],
                want: Some(get_vendor("GitHub Actions")),
            },
            TestCase {
                name: "Codeship".to_string(),
                set_env: vec!["CI_NAME=codeship".to_string()],
                want: Some(get_vendor("Codeship")),
            },
        ];

        for TestCase {
            name,
            set_env,
            want,
        } in tests
        {
            info!("test case: {}", name);

            let live_ci = if Vendor::get_name() == Some("GitHub Actions") {
                let live_ci = std::env::var("GITHUB_ACTIONS").unwrap_or_default();
                env::remove_var("GITHUB_ACTIONS");
                Some(live_ci)
            } else {
                None
            };

            for env in set_env.iter() {
                let mut env_parts = env.split('=');
                let key = env_parts.next().unwrap();
                let val = env_parts.next().unwrap_or("some value");
                env::set_var(key, val);
            }

            assert_eq!(Vendor::get_info(), want);

            if Vendor::get_name() == Some("GitHub Actions") {
                if let Some(live_ci) = live_ci {
                    env::set_var("GITHUB_ACTIONS", live_ci);
                } else {
                    env::remove_var("GITHUB_ACTIONS");
                }
            }

            for env in set_env {
                let mut env_parts = env.split('=');
                let key = env_parts.next().unwrap();
                env::remove_var(key);
            }
        }
    }
}
