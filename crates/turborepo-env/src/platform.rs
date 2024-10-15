use turborepo_ci::Vendor;
use turborepo_ui::{color, cprint, cprintln, ColorConfig, BOLD, GREY, UNDERLINE, YELLOW};

use crate::EnvironmentVariableMap;

pub struct PlatformEnv {
    env_keys: Vec<String>,
}

impl Default for PlatformEnv {
    fn default() -> Self {
        Self::new()
    }
}

const TURBO_PLATFORM_ENV_KEY: &str = "TURBO_PLATFORM_ENV";
const TURBO_PLATFORM_ENV_DISABLED_KEY: &str = "TURBO_PLATFORM_ENV_DISABLED";

impl PlatformEnv {
    pub fn new() -> Self {
        let env_keys = std::env::var(TURBO_PLATFORM_ENV_KEY)
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        Self { env_keys }
    }

    pub fn disabled() -> bool {
        let turbo_platform_env_disabled =
            std::env::var(TURBO_PLATFORM_ENV_DISABLED_KEY).unwrap_or_default();
        turbo_platform_env_disabled == "1" || turbo_platform_env_disabled == "true"
    }

    pub fn validate(&self, execution_env: &EnvironmentVariableMap) -> Vec<String> {
        if Self::disabled() {
            return vec![];
        }

        self.diff(execution_env)
    }

    pub fn diff(&self, execution_env: &EnvironmentVariableMap) -> Vec<String> {
        self.env_keys
            .iter()
            .filter(|key| !execution_env.contains_key(*key))
            .map(|s| s.to_string())
            .collect()
    }

    pub fn output_header(is_strict: bool, color_config: ColorConfig) {
        let ci = Vendor::get_constant().unwrap_or("unknown");

        let strict_message = if is_strict {
            "These variables WILL NOT be available to your application and may cause your build to \
             fail."
        } else {
            "These variables WILL NOT be considered in your cache key and could cause inadvertent \
             cache hits."
        };

        match ci {
            "VERCEL" => {
                cprintln!(
                    color_config,
                    BOLD,
                    "The following environment variables are set on your Vercel project, but \
                     missing from \"turbo.json\". {}",
                    strict_message
                );
            }
            _ => {
                cprintln!(
                    color_config,
                    BOLD,
                    "The following environment variables are missing from \"turbo.json\". {}",
                    strict_message
                );
            }
        }

        let docs = color!(
            color_config,
            UNDERLINE,
            "https://turbo.build/repo/docs/platform-environment-variables"
        );
        cprintln!(color_config, GREY, "Learn more at {docs}\n");
    }

    pub fn output_for_task(
        missing: Vec<String>,
        task_id_for_display: &str,
        color_config: ColorConfig,
    ) {
        cprintln!(color_config, YELLOW, "{}", task_id_for_display);
        for key in missing {
            cprint!(color_config, GREY, "  - ");
            cprint!(color_config, GREY, "{}\n", key);
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn set_env_var(key: &str, value: &str) {
        std::env::set_var(key, value);
    }

    fn clear_env_var(key: &str) {
        std::env::remove_var(key);
        assert!(std::env::var(key).is_err());
    }

    #[test]
    fn test_platform_env_new() {
        set_env_var(TURBO_PLATFORM_ENV_KEY, "VAR1,VAR2,VAR3");
        let platform_env = PlatformEnv::new();
        assert_eq!(platform_env.env_keys, vec!["VAR1", "VAR2", "VAR3"]);
        clear_env_var(TURBO_PLATFORM_ENV_KEY);
    }

    #[test]
    fn test_platform_env_new_empty() {
        set_env_var(TURBO_PLATFORM_ENV_KEY, "");
        let platform_env = PlatformEnv::new();
        assert!(platform_env.env_keys.is_empty());
        clear_env_var(TURBO_PLATFORM_ENV_KEY);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_disabled_true() {
        set_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY, "1");
        assert!(PlatformEnv::disabled());
        clear_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY);
    }

    #[test]
    fn test_disabled_false() {
        set_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY, "0");
        assert!(!PlatformEnv::disabled());
        clear_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY);
    }

    #[test]
    fn test_validate_disabled() {
        set_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY, "1");
        let platform_env = PlatformEnv::new();
        let execution_env = EnvironmentVariableMap(HashMap::new());
        let missing = platform_env.validate(&execution_env);
        assert!(missing.is_empty());
        clear_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY);
    }

    #[test]
    fn test_validate_missing_keys() {
        set_env_var(TURBO_PLATFORM_ENV_KEY, "VAR1,VAR2");
        clear_env_var(TURBO_PLATFORM_ENV_DISABLED_KEY);

        let platform_env = PlatformEnv::new();

        let mut execution_env = EnvironmentVariableMap(HashMap::new());
        execution_env.insert("VAR2".to_string(), "value".to_string());

        let missing = platform_env.validate(&execution_env);

        assert_eq!(missing, vec!["VAR1".to_string()]);

        clear_env_var(TURBO_PLATFORM_ENV_KEY);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_diff_all_keys_present() {
        set_env_var(TURBO_PLATFORM_ENV_KEY, "VAR1,VAR2");
        let platform_env = PlatformEnv::new();

        let mut execution_env = EnvironmentVariableMap(HashMap::new());
        execution_env.insert("VAR1".to_string(), "value1".to_string());
        execution_env.insert("VAR2".to_string(), "value2".to_string());

        let missing = platform_env.diff(&execution_env);
        assert!(missing.is_empty());

        clear_env_var(TURBO_PLATFORM_ENV_KEY);
    }
}
