/// Configuration source layers, ordered by precedence from lowest to highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigurationSource {
    TurboJson,
    GlobalConfig,
    GlobalAuth,
    LocalConfig,
    OverrideEnvironment,
    Environment,
    Override,
}

impl ConfigurationSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TurboJson => "turbo_json",
            Self::GlobalConfig => "global_config",
            Self::GlobalAuth => "global_auth",
            Self::LocalConfig => "local_config",
            Self::OverrideEnvironment => "override_environment",
            Self::Environment => "environment",
            Self::Override => "override",
        }
    }
}

/// Canonical precedence contract for configuration source merging.
///
/// Ordered from lowest to highest precedence.
pub const CONFIGURATION_PRECEDENCE: &[ConfigurationSource] = &[
    ConfigurationSource::TurboJson,
    ConfigurationSource::GlobalConfig,
    ConfigurationSource::GlobalAuth,
    ConfigurationSource::LocalConfig,
    ConfigurationSource::OverrideEnvironment,
    ConfigurationSource::Environment,
    ConfigurationSource::Override,
];

#[cfg(test)]
mod tests {
    use super::{ConfigurationSource, CONFIGURATION_PRECEDENCE};

    #[test]
    fn test_configuration_precedence_order_is_exact() {
        assert_eq!(
            CONFIGURATION_PRECEDENCE,
            &[
                ConfigurationSource::TurboJson,
                ConfigurationSource::GlobalConfig,
                ConfigurationSource::GlobalAuth,
                ConfigurationSource::LocalConfig,
                ConfigurationSource::OverrideEnvironment,
                ConfigurationSource::Environment,
                ConfigurationSource::Override,
            ]
        );
    }

    #[test]
    fn test_configuration_source_names_are_stable() {
        let names: Vec<_> = CONFIGURATION_PRECEDENCE
            .iter()
            .map(|source| source.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "turbo_json",
                "global_config",
                "global_auth",
                "local_config",
                "override_environment",
                "environment",
                "override",
            ]
        );
    }
}
