#![deny(clippy::all)]

mod vendors;

use std::{env, sync::OnceLock};

use crate::vendors::get_vendors;
pub use crate::vendors::Vendor;

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
    /// We return an empty String instead of None because
    /// the Spaces API expects some sort of string in the user field.
    pub fn get_user() -> String {
        let vendor = Vendor::infer();

        vendor
            .and_then(|v| v.username_env_var)
            .and_then(|v| env::var(v).ok())
            .unwrap_or_default()
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

    pub fn get_constant() -> Option<&'static str> {
        Self::infer().map(|v| v.constant)
    }
}

pub fn github_header_footer(package: Option<&str>, task: &str) -> (String, String) {
    let header = if let Some(package) = package {
        format!("::group::{package}:{task}\n")
    } else {
        format!("::group::{task}\n")
    };
    (header, "::endgroup::\n".to_string())
}

#[cfg(test)]
mod tests {
    use tracing::info;

    use super::*;
    use crate::Vendor;

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

            assert_eq!(Vendor::infer_inner(), want.as_ref());

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
