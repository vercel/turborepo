use turborepo_ci::Vendor;
use turborepo_ui::{ceprint, ceprintln, color, ColorConfig, BOLD, GREY, UNDERLINE, YELLOW};

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

        let docs_message = color!(
            color_config,
            UNDERLINE,
            "https://turbo.build/repo/docs/platform-environment-variables"
        );

        match ci {
            "VERCEL" => {
                ceprintln!(
                    color_config,
                    BOLD,
                    "Warning - the following environment variables are set on your Vercel \
                     project, but missing from \"turbo.json\". {strict_message} Learn more at \
                     {docs_message}\n"
                );
            }
            _ => {
                ceprintln!(
                    color_config,
                    BOLD,
                    "Warning - the following environment variables are missing from \
                     \"turbo.json\". {strict_message} Learn more at {docs_message}\n"
                );
            }
        }
    }

    pub fn output_for_task(
        missing: Vec<String>,
        task_id_for_display: &str,
        color_config: ColorConfig,
    ) {
        let ci = Vendor::get_constant().unwrap_or("unknown");
        let log_prefix = match ci {
            "VERCEL" => "[warn]",
            _ => "",
        };
        ceprintln!(
            color_config,
            YELLOW,
            "{} {}",
            log_prefix,
            task_id_for_display
        );
        for key in missing {
            ceprint!(color_config, GREY, "{}   - ", log_prefix);
            ceprint!(color_config, GREY, "{} \n", key);
        }
    }
}
