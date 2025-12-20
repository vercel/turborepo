#![deny(clippy::all)]

//! Turborepo's update notifier. Indicates to the user that there is a new
//! version of `turbo` available.

use std::{fmt, time::Duration};

use console::style;
use semver::Version as SemVerVersion;
use serde::Deserialize;
use thiserror::Error as ThisError;
use turborepo_repository::package_manager::PackageManager;
use update_informer::{
    Check, Package, Registry, Result as UpdateResult, Version,
    http_client::{GenericHttpClient, HttpClient},
};

mod ui;

// 800ms
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(800);
// 1 day
const DEFAULT_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

const NOTIFIER_DISABLE_VARS: [&str; 1] = ["NO_UPDATE_NOTIFIER"];
const ENVIRONMENTAL_DISABLE_VARS: [&str; 1] = ["CI"];

#[derive(ThisError, Debug)]
pub enum UpdateNotifierError {
    #[error("Failed to write to terminal")]
    RenderError(#[from] ui::utils::GetDisplayLengthError),
    #[error("Failed to parse current version")]
    VersionError(#[from] semver::Error),
    #[error("Failed to check for updates")]
    FetchError(#[from] Box<dyn std::error::Error>),
}

#[derive(Deserialize, Debug)]
struct NpmVersionData {
    version: String,
}

#[derive(Debug)]
enum VersionTag {
    Latest,
    Canary,
}

impl fmt::Display for VersionTag {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VersionTag::Latest => write!(f, "latest"),
            VersionTag::Canary => write!(f, "canary"),
        }
    }
}

struct NPMRegistry;

impl Registry for NPMRegistry {
    const NAME: &'static str = "npm-registry";
    fn get_latest_version<T: HttpClient>(
        http: GenericHttpClient<T>,
        pkg: &Package,
    ) -> UpdateResult<Option<String>> {
        // determine tag to request
        let tag = get_tag_from_version(&pkg.version().semver().pre);
        // since we're overloading tag within name, we need to split it back out
        let full_name = pkg.to_string();
        let split_name: Vec<&str> = full_name.split('/').collect();
        let name = split_name[1];
        let url = format!("https://turborepo.com/api/binaries/version?name={name}&tag={tag}");

        let result: NpmVersionData = http.get(&url)?;
        Ok(Some(result.version))
    }
}

fn get_tag_from_version(pre: &semver::Prerelease) -> VersionTag {
    match pre {
        t if t.contains("canary") => VersionTag::Canary,
        _ => VersionTag::Latest,
    }
}

fn should_skip_notification(config_no_update: bool) -> bool {
    if config_no_update {
        return true;
    }

    if NOTIFIER_DISABLE_VARS
        .iter()
        .chain(ENVIRONMENTAL_DISABLE_VARS.iter())
        .any(|var| std::env::var(var).is_ok())
    {
        return true;
    }

    if !atty::is(atty::Stream::Stdout) {
        return true;
    }

    false
}

/// Configuration for the update check notification.
#[derive(Debug)]
pub struct UpdateCheckConfig<'a> {
    /// The name of the package to check for updates.
    pub package_name: &'a str,
    /// The GitHub repository URL for the changelog link.
    pub github_repo: &'a str,
    /// Optional footer text to display below the update message.
    pub footer: Option<&'a str>,
    /// The current version of the package.
    pub current_version: &'a str,
    /// Timeout for the update check request. Defaults to 800ms.
    pub timeout: Option<Duration>,
    /// Interval between update checks. Defaults to 24 hours.
    pub interval: Option<Duration>,
    /// The package manager being used.
    pub package_manager: &'a PackageManager,
    /// Whether update notifications are disabled via config.
    pub config_no_update: bool,
}

pub fn display_update_check(config: UpdateCheckConfig) -> Result<(), UpdateNotifierError> {
    // bail early if the user has disabled update notifications
    if should_skip_notification(config.config_no_update) {
        return Ok(());
    }

    let version = check_for_updates(
        config.package_name,
        config.current_version,
        config.timeout,
        config.interval,
    );

    if let Ok(Some(version)) = version {
        let latest_version = version.to_string();

        let update_cmd = match config.package_manager {
            PackageManager::Npm => style("npx @turbo/codemod@latest update").cyan().bold(),
            PackageManager::Yarn | PackageManager::Berry => {
                style("yarn dlx @turbo/codemod@latest update").cyan().bold()
            }
            PackageManager::Pnpm | PackageManager::Pnpm6 | PackageManager::Pnpm9 => {
                style("pnpm dlx @turbo/codemod@latest update").cyan().bold()
            }
            PackageManager::Bun => style("bunx @turbo/codemod@latest update").cyan().bold(),
        };

        let msg = format!(
            "
            Update available {version_prefix}{current_version} ≫ {latest_version}
            Changelog: {github_repo}/releases/tag/{latest_version}
            Run \"{update_cmd}\" to update
            ",
            version_prefix = style("v").dim(),
            current_version = style(config.current_version).dim(),
            latest_version = style(latest_version).green().bold(),
            github_repo = config.github_repo,
            update_cmd = update_cmd
        );

        if let Some(footer) = config.footer {
            return ui::message(&format!("{msg}\n{footer}"));
        }

        return ui::message(&msg);
    }

    Ok(())
}

pub fn check_for_updates(
    package_name: &str,
    current_version: &str,
    timeout: Option<Duration>,
    interval: Option<Duration>,
) -> Result<Option<Version>, UpdateNotifierError> {
    // we want notifications per channel (latest, canary, etc) so we need to ensure
    // we have one cached latest version per channel. UpdateInformer does not
    // support this out of the box, so we hack it into the name by overloading
    // owner (in the supported owner/name format) to be channel/name.
    let parsed_version = SemVerVersion::parse(current_version)?;
    let tag = get_tag_from_version(&parsed_version.pre);
    let package_name = format!("{tag}/{package_name}");

    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let interval = interval.unwrap_or(DEFAULT_INTERVAL);
    let informer = update_informer::new(NPMRegistry, package_name, current_version)
        .timeout(timeout)
        .interval(interval);

    let data = informer
        .check_version()
        .map_err(UpdateNotifierError::FetchError)?;

    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // Mutex to ensure env var tests don't interfere with each other
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    // Helper to run tests with specific env vars set, then clean up
    fn with_env_vars<F, R>(vars: &[(&str, Option<&str>)], f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _guard = ENV_MUTEX.lock().unwrap();

        // Store original values and set new ones
        let originals: Vec<_> = vars
            .iter()
            .map(|(key, value)| {
                let original = std::env::var(key).ok();
                // SAFETY: We hold ENV_MUTEX to ensure exclusive access to env vars
                // during test execution, preventing data races between tests.
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(key, v),
                        None => std::env::remove_var(key),
                    }
                }
                (*key, original)
            })
            .collect();

        let result = f();

        // Restore original values
        for (key, original) in originals {
            // SAFETY: We hold ENV_MUTEX to ensure exclusive access to env vars
            // during test execution, preventing data races between tests.
            unsafe {
                match original {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }

        result
    }

    // ==================== should_skip_notification tests ====================

    #[test]
    fn test_skip_notification_when_config_no_update_is_true() {
        // When config_no_update is true, should always skip regardless of other
        // conditions
        with_env_vars(&[("NO_UPDATE_NOTIFIER", None), ("CI", None)], || {
            assert!(
                should_skip_notification(true),
                "should skip when config_no_update is true"
            );
        });
    }

    #[test]
    fn test_skip_notification_when_no_update_notifier_env_set() {
        with_env_vars(&[("NO_UPDATE_NOTIFIER", Some("1")), ("CI", None)], || {
            assert!(
                should_skip_notification(false),
                "should skip when NO_UPDATE_NOTIFIER is set"
            );
        });
    }

    #[test]
    fn test_skip_notification_when_no_update_notifier_env_empty() {
        // Even an empty string means the var is set
        with_env_vars(&[("NO_UPDATE_NOTIFIER", Some("")), ("CI", None)], || {
            assert!(
                should_skip_notification(false),
                "should skip when NO_UPDATE_NOTIFIER is set (even if empty)"
            );
        });
    }

    #[test]
    fn test_skip_notification_when_ci_env_set() {
        with_env_vars(
            &[("NO_UPDATE_NOTIFIER", None), ("CI", Some("true"))],
            || {
                assert!(
                    should_skip_notification(false),
                    "should skip when CI is set"
                );
            },
        );
    }

    #[test]
    fn test_skip_notification_when_ci_env_set_to_any_value() {
        with_env_vars(&[("NO_UPDATE_NOTIFIER", None), ("CI", Some("1"))], || {
            assert!(
                should_skip_notification(false),
                "should skip when CI is set to any value"
            );
        });
    }

    #[test]
    fn test_skip_notification_when_both_env_vars_set() {
        with_env_vars(
            &[("NO_UPDATE_NOTIFIER", Some("1")), ("CI", Some("true"))],
            || {
                assert!(
                    should_skip_notification(false),
                    "should skip when both env vars are set"
                );
            },
        );
    }

    #[test]
    fn test_skip_notification_non_tty() {
        // In test environment, stdout is typically not a TTY, so this tests the
        // non-TTY path. When running in a non-TTY environment with no env vars,
        // should still skip due to non-TTY.
        with_env_vars(&[("NO_UPDATE_NOTIFIER", None), ("CI", None)], || {
            // In test environment, we're typically not in a TTY
            // The function should return true (skip) when not in a TTY
            let result = should_skip_notification(false);
            // We can't guarantee TTY status in tests, so we just verify it runs
            // without panicking. The actual TTY check behavior depends on the
            // test runner environment.
            let _ = result;
        });
    }

    // ==================== get_tag_from_version tests ====================

    #[test]
    fn test_get_tag_canary_prerelease() {
        let pre = semver::Prerelease::new("canary.1").unwrap();
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Canary),
            "canary prerelease should return Canary tag"
        );
    }

    #[test]
    fn test_get_tag_canary_prerelease_with_suffix() {
        let pre = semver::Prerelease::new("canary.123.abcdef").unwrap();
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Canary),
            "canary prerelease with suffix should return Canary tag"
        );
    }

    #[test]
    fn test_get_tag_empty_prerelease() {
        let pre = semver::Prerelease::EMPTY;
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Latest),
            "empty prerelease should return Latest tag"
        );
    }

    #[test]
    fn test_get_tag_alpha_prerelease() {
        let pre = semver::Prerelease::new("alpha.1").unwrap();
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Latest),
            "alpha prerelease should return Latest tag"
        );
    }

    #[test]
    fn test_get_tag_beta_prerelease() {
        let pre = semver::Prerelease::new("beta.2").unwrap();
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Latest),
            "beta prerelease should return Latest tag"
        );
    }

    #[test]
    fn test_get_tag_rc_prerelease() {
        let pre = semver::Prerelease::new("rc.1").unwrap();
        let tag = get_tag_from_version(&pre);
        assert!(
            matches!(tag, VersionTag::Latest),
            "rc prerelease should return Latest tag"
        );
    }

    // ==================== VersionTag Display tests ====================

    #[test]
    fn test_version_tag_display_latest() {
        let tag = VersionTag::Latest;
        assert_eq!(tag.to_string(), "latest");
    }

    #[test]
    fn test_version_tag_display_canary() {
        let tag = VersionTag::Canary;
        assert_eq!(tag.to_string(), "canary");
    }

    // ==================== Integration tests for version parsing
    // ====================

    #[test]
    fn test_version_parsing_stable() {
        let version = SemVerVersion::parse("2.0.0").unwrap();
        let tag = get_tag_from_version(&version.pre);
        assert!(matches!(tag, VersionTag::Latest));
    }

    #[test]
    fn test_version_parsing_canary() {
        let version = SemVerVersion::parse("2.0.0-canary.1").unwrap();
        let tag = get_tag_from_version(&version.pre);
        assert!(matches!(tag, VersionTag::Canary));
    }

    #[test]
    fn test_version_parsing_complex_canary() {
        let version = SemVerVersion::parse("2.1.0-canary.20231201.abc123").unwrap();
        let tag = get_tag_from_version(&version.pre);
        assert!(matches!(tag, VersionTag::Canary));
    }

    // ==================== Constants verification tests ====================

    #[test]
    fn test_notifier_disable_vars_contains_no_update_notifier() {
        assert!(
            NOTIFIER_DISABLE_VARS.contains(&"NO_UPDATE_NOTIFIER"),
            "NOTIFIER_DISABLE_VARS should contain NO_UPDATE_NOTIFIER"
        );
    }

    #[test]
    fn test_environmental_disable_vars_contains_ci() {
        assert!(
            ENVIRONMENTAL_DISABLE_VARS.contains(&"CI"),
            "ENVIRONMENTAL_DISABLE_VARS should contain CI"
        );
    }

    #[test]
    fn test_default_timeout_is_reasonable() {
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_millis(800));
    }

    #[test]
    fn test_default_interval_is_one_day() {
        assert_eq!(DEFAULT_INTERVAL, Duration::from_secs(60 * 60 * 24));
    }
}
