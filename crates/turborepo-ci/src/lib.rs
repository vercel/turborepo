//! CI/CD vendor detection and vendor-specific behavior
//! Detects CI vendors and provides:
//! - Env var containing current commit SHA
//! - Env var containing current branch
//! - Env var containing current user
//! - Any vendor specific behavior for producing well formatted logs

mod vendor_behavior;
mod vendors;

use std::{env, sync::OnceLock};

use crate::vendors::get_vendors;
pub use crate::{
    vendor_behavior::{GroupPrefixFn, VendorBehavior},
    vendors::Vendor,
};

static IS_CI: OnceLock<bool> = OnceLock::new();
static VENDOR: OnceLock<Option<&'static Vendor>> = OnceLock::new();

const CI_ENV_VARS: &[&str] = [
    "BUILD_ID",
    "BUILD_NUMBER",
    "CI",
    "CI_APP_ID",
    "CI_BUILD_ID",
    "CI_BUILD_NUMBER",
    "CI_NAME",
    "CONTINUOUS_INTEGRATION",
    "RUN_ID",
    "TEAMCITY_VERSION",
]
.as_slice();

pub fn is_ci() -> bool {
    *IS_CI.get_or_init(|| {
        // We purposefully don't do `is_err()` because the Go version
        // returns false for both an unset env variable
        // and an env variable set to the empty string.
        CI_ENV_VARS
            .iter()
            .any(|env_var| !env::var(env_var).unwrap_or_default().is_empty())
    })
}

impl Vendor {
    // Returns info about a CI vendor
    pub fn infer() -> Option<&'static Vendor> {
        *VENDOR.get_or_init(Self::infer_inner)
    }

    /// Gets user from CI environment variables
    pub fn get_user() -> Option<String> {
        let vendor = Vendor::infer();

        vendor
            .and_then(|v| v.username_env_var)
            .and_then(|v| env::var(v).ok())
    }

    fn infer_inner() -> Option<&'static Vendor> {
        for env in get_vendors() {
            if let Some(eval_env) = &env.eval_env {
                for (name, expected_value) in eval_env {
                    if matches!(env::var(name), Ok(env_value) if *expected_value == env_value) {
                        return Some(env);
                    }
                }
            } else if !env.env.any.is_empty() {
                for env_var in &env.env.any {
                    if matches!(env::var(env_var), Ok(v) if !v.is_empty()) {
                        return Some(env);
                    }
                }
            } else if !env.env.all.is_empty() {
                let all = env
                    .env
                    .all
                    .iter()
                    .all(|env_var| !env::var(env_var).unwrap_or_default().is_empty());

                if all {
                    return Some(env);
                }
            }
        }

        None
    }

    pub fn get_name() -> Option<&'static str> {
        Self::infer().map(|v| v.name)
    }

    pub fn is(name: &str) -> bool {
        Self::infer().is_some_and(|v| v.name == name)
    }

    pub fn get_constant() -> Option<&'static str> {
        Self::infer().map(|v| v.constant)
    }
}

#[cfg(test)]
mod tests {
    use tracing::info;

    use super::*;

    fn get_vendor(name: &str) -> Vendor {
        for v in get_vendors() {
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
                unsafe { env::remove_var("GITHUB_ACTIONS") };
                Some(live_ci)
            } else {
                None
            };

            for env in set_env.iter() {
                let mut env_parts = env.split('=');
                let key = env_parts.next().unwrap();
                let val = env_parts.next().unwrap_or("some value");
                unsafe { env::set_var(key, val) };
            }

            assert_eq!(
                Vendor::infer_inner().map(|v| v.name),
                want.as_ref().map(|v| v.name)
            );

            if Vendor::get_name() == Some("GitHub Actions") {
                if let Some(live_ci) = live_ci {
                    unsafe { env::set_var("GITHUB_ACTIONS", live_ci) };
                } else {
                    unsafe { env::remove_var("GITHUB_ACTIONS") };
                }
            }

            for env in set_env {
                let mut env_parts = env.split('=');
                let key = env_parts.next().unwrap();
                unsafe { env::remove_var(key) };
            }
        }
    }

    #[test]
    fn test_gitlab_ci_group_name_sanitization() {
        use chrono::DateTime;

        let gitlab_vendor = get_vendor("GitLab CI");
        let behavior = gitlab_vendor.behavior.as_ref().unwrap();

        // Test with a package name containing @ and /
        let group_name = "@organisation/package:build".to_string();
        let start_time = DateTime::from_timestamp(1234567890, 0).unwrap();
        let end_time = DateTime::from_timestamp(1234567900, 0).unwrap();

        let start_fn = (behavior.group_prefix)(group_name.clone());
        let end_fn = (behavior.group_suffix)(group_name.clone());

        let start_output = start_fn(start_time);
        let end_output = end_fn(end_time);

        // The section identifier should be sanitized (@ -> at, / -> -)
        assert!(start_output.contains("section_start:1234567890:at-organisation-package:build"));
        assert!(end_output.contains("section_end:1234567900:at-organisation-package:build"));

        // The description should contain the original group name
        assert!(start_output.contains("@organisation/package:build"));

        // Test with a simple package name (should work unchanged)
        let simple_group_name = "simple-package:build".to_string();
        let simple_start_fn = (behavior.group_prefix)(simple_group_name.clone());
        let simple_start_output = simple_start_fn(start_time);

        assert!(simple_start_output.contains("section_start:1234567890:simple-package:build"));
        assert!(simple_start_output.contains("simple-package:build"));
    }
}
