/// Canonical identifier for each configuration option tracked by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionId {
    ApiUrl,
    LoginUrl,
    TeamSlug,
    TeamId,
    Token,
    Signature,
    Preflight,
    Timeout,
    UploadTimeout,
    Enabled,
    Ui,
    AllowNoPackageManager,
    Daemon,
    EnvMode,
    ScmBase,
    ScmHead,
    CacheDir,
    RootTurboJsonPath,
    Force,
    LogOrder,
    Cache,
    RemoteOnly,
    RemoteCacheReadOnly,
    RunSummary,
    AllowNoTurboJson,
    TuiScrollbackLength,
    Concurrency,
    NoUpdateNotifier,
    SsoLoginCallbackPort,
    FutureFlags,
}

impl OptionId {
    pub const ALL: [Self; 30] = [
        Self::ApiUrl,
        Self::LoginUrl,
        Self::TeamSlug,
        Self::TeamId,
        Self::Token,
        Self::Signature,
        Self::Preflight,
        Self::Timeout,
        Self::UploadTimeout,
        Self::Enabled,
        Self::Ui,
        Self::AllowNoPackageManager,
        Self::Daemon,
        Self::EnvMode,
        Self::ScmBase,
        Self::ScmHead,
        Self::CacheDir,
        Self::RootTurboJsonPath,
        Self::Force,
        Self::LogOrder,
        Self::Cache,
        Self::RemoteOnly,
        Self::RemoteCacheReadOnly,
        Self::RunSummary,
        Self::AllowNoTurboJson,
        Self::TuiScrollbackLength,
        Self::Concurrency,
        Self::NoUpdateNotifier,
        Self::SsoLoginCallbackPort,
        Self::FutureFlags,
    ];
}

/// Metadata for a canonical option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionSpec {
    /// Stable option identifier.
    pub id: OptionId,
    /// Field on `ConfigurationOptions`.
    pub configuration_field: &'static str,
    /// Serialized keys used in config files.
    pub config_keys: &'static [&'static str],
    /// Environment variable keys (lowercased because env parsing lowercases).
    pub env_vars: &'static [&'static str],
    /// CLI long flag names.
    pub cli_flags: &'static [&'static str],
}

/// Optional escape hatch for explicitly unregistered `ConfigurationOptions`
/// fields. Keep empty unless we intentionally omit a field from the registry.
pub const EXCLUDED_CONFIGURATION_FIELDS: &[&str] = &[];

pub const OPTION_REGISTRY: [OptionSpec; 30] = [
    OptionSpec {
        id: OptionId::ApiUrl,
        configuration_field: "api_url",
        config_keys: &["apiUrl", "apiurl", "ApiUrl", "APIURL"],
        env_vars: &["turbo_api"],
        cli_flags: &["--api"],
    },
    OptionSpec {
        id: OptionId::LoginUrl,
        configuration_field: "login_url",
        config_keys: &["loginUrl", "loginurl", "LoginUrl", "LOGINURL"],
        env_vars: &["turbo_login"],
        cli_flags: &["--login"],
    },
    OptionSpec {
        id: OptionId::TeamSlug,
        configuration_field: "team_slug",
        config_keys: &["teamSlug", "teamslug", "TeamSlug", "TEAMSLUG"],
        env_vars: &["turbo_team"],
        cli_flags: &["--team"],
    },
    OptionSpec {
        id: OptionId::TeamId,
        configuration_field: "team_id",
        config_keys: &["teamId", "teamid", "TeamId", "TEAMID"],
        env_vars: &["turbo_teamid", "vercel_artifacts_owner"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::Token,
        configuration_field: "token",
        config_keys: &["token"],
        env_vars: &["turbo_token", "vercel_artifacts_token"],
        cli_flags: &["--token"],
    },
    OptionSpec {
        id: OptionId::Signature,
        configuration_field: "signature",
        config_keys: &["signature"],
        env_vars: &[],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::Preflight,
        configuration_field: "preflight",
        config_keys: &["preflight"],
        env_vars: &["turbo_preflight"],
        cli_flags: &["--preflight"],
    },
    OptionSpec {
        id: OptionId::Timeout,
        configuration_field: "timeout",
        config_keys: &["timeout"],
        env_vars: &["turbo_remote_cache_timeout"],
        cli_flags: &["--remote-cache-timeout"],
    },
    OptionSpec {
        id: OptionId::UploadTimeout,
        configuration_field: "upload_timeout",
        config_keys: &["uploadTimeout"],
        env_vars: &["turbo_remote_cache_upload_timeout"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::Enabled,
        configuration_field: "enabled",
        config_keys: &["enabled"],
        env_vars: &[],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::Ui,
        configuration_field: "ui",
        config_keys: &["ui"],
        env_vars: &["turbo_ui", "ci", "no_color"],
        cli_flags: &["--ui"],
    },
    OptionSpec {
        id: OptionId::AllowNoPackageManager,
        configuration_field: "allow_no_package_manager",
        config_keys: &["dangerouslyDisablePackageManagerCheck"],
        env_vars: &["turbo_dangerously_disable_package_manager_check"],
        cli_flags: &["--dangerously-disable-package-manager-check"],
    },
    OptionSpec {
        id: OptionId::Daemon,
        configuration_field: "daemon",
        config_keys: &["daemon"],
        env_vars: &["turbo_daemon"],
        cli_flags: &["--daemon", "--no-daemon"],
    },
    OptionSpec {
        id: OptionId::EnvMode,
        configuration_field: "env_mode",
        config_keys: &["envMode"],
        env_vars: &["turbo_env_mode"],
        cli_flags: &["--env-mode"],
    },
    OptionSpec {
        id: OptionId::ScmBase,
        configuration_field: "scm_base",
        config_keys: &["scmBase"],
        env_vars: &["turbo_scm_base"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::ScmHead,
        configuration_field: "scm_head",
        config_keys: &["scmHead"],
        env_vars: &["turbo_scm_head"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::CacheDir,
        configuration_field: "cache_dir",
        config_keys: &["cacheDir"],
        env_vars: &["turbo_cache_dir"],
        cli_flags: &["--cache-dir"],
    },
    OptionSpec {
        id: OptionId::RootTurboJsonPath,
        configuration_field: "root_turbo_json_path",
        config_keys: &[],
        env_vars: &["turbo_root_turbo_json"],
        cli_flags: &["--root-turbo-json"],
    },
    OptionSpec {
        id: OptionId::Force,
        configuration_field: "force",
        config_keys: &["force"],
        env_vars: &["turbo_force"],
        cli_flags: &["--force"],
    },
    OptionSpec {
        id: OptionId::LogOrder,
        configuration_field: "log_order",
        config_keys: &["logOrder"],
        env_vars: &["turbo_log_order"],
        cli_flags: &["--log-order"],
    },
    OptionSpec {
        id: OptionId::Cache,
        configuration_field: "cache",
        config_keys: &[],
        env_vars: &["turbo_cache"],
        cli_flags: &["--cache", "--no-cache"],
    },
    OptionSpec {
        id: OptionId::RemoteOnly,
        configuration_field: "remote_only",
        config_keys: &["remoteOnly"],
        env_vars: &["turbo_remote_only"],
        cli_flags: &["--remote-only"],
    },
    OptionSpec {
        id: OptionId::RemoteCacheReadOnly,
        configuration_field: "remote_cache_read_only",
        config_keys: &["remoteCacheReadOnly"],
        env_vars: &["turbo_remote_cache_read_only"],
        cli_flags: &["--remote-cache-read-only"],
    },
    OptionSpec {
        id: OptionId::RunSummary,
        configuration_field: "run_summary",
        config_keys: &["runSummary"],
        env_vars: &["turbo_run_summary"],
        cli_flags: &["--summarize"],
    },
    OptionSpec {
        id: OptionId::AllowNoTurboJson,
        configuration_field: "allow_no_turbo_json",
        config_keys: &["allowNoTurboJson"],
        env_vars: &["turbo_allow_no_turbo_json"],
        cli_flags: &["--experimental-allow-no-turbo-json"],
    },
    OptionSpec {
        id: OptionId::TuiScrollbackLength,
        configuration_field: "tui_scrollback_length",
        config_keys: &["tuiScrollbackLength"],
        env_vars: &["turbo_tui_scrollback_length"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::Concurrency,
        configuration_field: "concurrency",
        config_keys: &["concurrency"],
        env_vars: &["turbo_concurrency"],
        cli_flags: &["--concurrency"],
    },
    OptionSpec {
        id: OptionId::NoUpdateNotifier,
        configuration_field: "no_update_notifier",
        config_keys: &["noUpdateNotifier"],
        env_vars: &["turbo_no_update_notifier"],
        cli_flags: &["--no-update-notifier"],
    },
    OptionSpec {
        id: OptionId::SsoLoginCallbackPort,
        configuration_field: "sso_login_callback_port",
        config_keys: &["ssoLoginCallbackPort"],
        env_vars: &["turbo_sso_login_callback_port"],
        cli_flags: &[],
    },
    OptionSpec {
        id: OptionId::FutureFlags,
        configuration_field: "future_flags",
        config_keys: &[],
        env_vars: &[],
        cli_flags: &[],
    },
];

pub fn option_registry() -> &'static [OptionSpec] {
    &OPTION_REGISTRY
}

pub fn option_spec(id: OptionId) -> Option<&'static OptionSpec> {
    OPTION_REGISTRY.iter().find(|spec| spec.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use struct_iterable::Iterable;

    use super::{option_registry, EXCLUDED_CONFIGURATION_FIELDS};
    use crate::registry::{OptionId, OPTION_REGISTRY};

    #[test]
    fn test_registry_is_unique_by_id_and_field() {
        let mut ids = HashSet::new();
        let mut fields = HashSet::new();

        for spec in option_registry() {
            assert!(ids.insert(spec.id), "duplicate option id: {:?}", spec.id);
            assert!(
                fields.insert(spec.configuration_field),
                "duplicate configuration field: {}",
                spec.configuration_field
            );
        }
    }

    #[test]
    fn test_registry_covers_all_configuration_fields_or_explicit_exclusions() {
        let all_configuration_fields: BTreeSet<&'static str> =
            crate::ConfigurationOptions::default()
                .iter()
                .map(|(name, _)| name)
                .collect();
        let excluded_fields: BTreeSet<&'static str> =
            EXCLUDED_CONFIGURATION_FIELDS.iter().copied().collect();
        let registered_fields: BTreeSet<&'static str> = OPTION_REGISTRY
            .iter()
            .map(|spec| spec.configuration_field)
            .collect();

        assert!(
            excluded_fields.is_subset(&all_configuration_fields),
            "excluded fields must exist on ConfigurationOptions"
        );

        let expected_fields: BTreeSet<&'static str> = all_configuration_fields
            .difference(&excluded_fields)
            .copied()
            .collect();

        assert_eq!(
            registered_fields, expected_fields,
            "registry must include every non-excluded ConfigurationOptions field"
        );
    }

    #[test]
    fn test_registry_has_spec_for_each_option_id() {
        let ids_from_registry: BTreeSet<OptionId> =
            OPTION_REGISTRY.iter().map(|spec| spec.id).collect();
        let ids_from_enum: BTreeSet<OptionId> = OptionId::ALL.into_iter().collect();

        assert_eq!(
            ids_from_registry, ids_from_enum,
            "every OptionId variant must have exactly one registry spec"
        );
    }
}
