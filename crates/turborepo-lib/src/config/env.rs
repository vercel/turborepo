use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
};

use super::{ConfigurationOptions, Error};
use crate::{cli::EnvMode, turbo_json::UIMode};

pub fn get_env_var_config(
    environment: &HashMap<OsString, OsString>,
) -> Result<ConfigurationOptions, Error> {
    let mut turbo_mapping = HashMap::new();
    turbo_mapping.insert(OsString::from("turbo_api"), "api_url");
    turbo_mapping.insert(OsString::from("turbo_login"), "login_url");
    turbo_mapping.insert(OsString::from("turbo_team"), "team_slug");
    turbo_mapping.insert(OsString::from("turbo_teamid"), "team_id");
    turbo_mapping.insert(OsString::from("turbo_token"), "token");
    turbo_mapping.insert(OsString::from("turbo_remote_cache_timeout"), "timeout");
    turbo_mapping.insert(
        OsString::from("turbo_remote_cache_upload_timeout"),
        "upload_timeout",
    );
    turbo_mapping.insert(OsString::from("turbo_ui"), "ui");
    turbo_mapping.insert(
        OsString::from("turbo_dangerously_disable_package_manager_check"),
        "allow_no_package_manager",
    );
    turbo_mapping.insert(OsString::from("turbo_daemon"), "daemon");
    turbo_mapping.insert(OsString::from("turbo_env_mode"), "env_mode");
    turbo_mapping.insert(OsString::from("turbo_cache_dir"), "cache_dir");
    turbo_mapping.insert(OsString::from("turbo_preflight"), "preflight");
    turbo_mapping.insert(OsString::from("turbo_scm_base"), "scm_base");
    turbo_mapping.insert(OsString::from("turbo_scm_head"), "scm_head");

    // We do not enable new config sources:
    // turbo_mapping.insert(String::from("turbo_signature"), "signature"); // new
    // turbo_mapping.insert(String::from("turbo_remote_cache_enabled"), "enabled");

    let mut output_map = HashMap::new();

    turbo_mapping.into_iter().try_for_each(
        |(mapping_key, mapped_property)| -> Result<(), Error> {
            if let Some(value) = environment.get(&mapping_key) {
                let converted = value.to_str().ok_or_else(|| {
                    Error::Encoding(
                        // CORRECTNESS: the mapping_key is hardcoded above.
                        mapping_key.to_ascii_uppercase().into_string().unwrap(),
                    )
                })?;
                output_map.insert(mapped_property, converted.to_owned());
                Ok(())
            } else {
                Ok(())
            }
        },
    )?;

    // Process signature
    let signature = if let Some(signature) = output_map.get("signature") {
        match signature.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(Error::InvalidSignature),
        }
    } else {
        None
    };

    // Process preflight
    let preflight = if let Some(preflight) = output_map.get("preflight") {
        match preflight.as_str() {
            "0" | "false" => Some(false),
            "1" | "true" => Some(true),
            "" => None,
            _ => return Err(Error::InvalidPreflight),
        }
    } else {
        None
    };

    // Process enabled
    let enabled = if let Some(enabled) = output_map.get("enabled") {
        match enabled.as_str() {
            "0" => Some(false),
            "1" => Some(true),
            _ => return Err(Error::InvalidRemoteCacheEnabled),
        }
    } else {
        None
    };

    // Process timeout
    let timeout = if let Some(timeout) = output_map.get("timeout") {
        Some(
            timeout
                .parse::<u64>()
                .map_err(Error::InvalidRemoteCacheTimeout)?,
        )
    } else {
        None
    };

    let upload_timeout = if let Some(upload_timeout) = output_map.get("upload_timeout") {
        Some(
            upload_timeout
                .parse::<u64>()
                .map_err(Error::InvalidUploadTimeout)?,
        )
    } else {
        None
    };

    // Process experimentalUI
    let ui = output_map
        .get("ui")
        .map(|s| s.as_str())
        .and_then(truth_env_var)
        .map(|ui| if ui { UIMode::Tui } else { UIMode::Stream });

    let allow_no_package_manager = output_map
        .get("allow_no_package_manager")
        .map(|s| s.as_str())
        .and_then(truth_env_var);

    // Process daemon
    let daemon = output_map.get("daemon").and_then(|val| match val.as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    });

    let env_mode = output_map
        .get("env_mode")
        .map(|s| s.as_str())
        .and_then(|s| match s {
            "strict" => Some(EnvMode::Strict),
            "loose" => Some(EnvMode::Loose),
            _ => None,
        });

    let cache_dir = output_map.get("cache_dir").map(|s| s.clone().into());

    // We currently don't pick up a Spaces ID via env var, we likely won't
    // continue using the Spaces name, we can add an env var when we have the
    // name we want to stick with.
    let spaces_id = None;

    let output = ConfigurationOptions {
        api_url: output_map.get("api_url").cloned(),
        login_url: output_map.get("login_url").cloned(),
        team_slug: output_map.get("team_slug").cloned(),
        team_id: output_map.get("team_id").cloned(),
        token: output_map.get("token").cloned(),
        scm_base: output_map.get("scm_base").cloned(),
        scm_head: output_map.get("scm_head").cloned(),

        // Processed booleans
        signature,
        preflight,
        enabled,
        ui,
        allow_no_package_manager,
        daemon,

        // Processed numbers
        timeout,
        upload_timeout,
        spaces_id,
        env_mode,
        cache_dir,
    };

    Ok(output)
}

pub fn get_override_env_var_config(
    environment: &HashMap<OsString, OsString>,
) -> Result<ConfigurationOptions, Error> {
    let mut vercel_artifacts_mapping = HashMap::new();
    vercel_artifacts_mapping.insert(OsString::from("vercel_artifacts_token"), "token");
    vercel_artifacts_mapping.insert(OsString::from("vercel_artifacts_owner"), "team_id");

    let mut output_map = HashMap::new();

    // Process the VERCEL_ARTIFACTS_* next.
    vercel_artifacts_mapping.into_iter().try_for_each(
        |(mapping_key, mapped_property)| -> Result<(), Error> {
            if let Some(value) = environment.get(&mapping_key) {
                let converted = value.to_str().ok_or_else(|| {
                    Error::Encoding(
                        // CORRECTNESS: the mapping_key is hardcoded above.
                        mapping_key.to_ascii_uppercase().into_string().unwrap(),
                    )
                })?;
                output_map.insert(mapped_property, converted.to_owned());
                Ok(())
            } else {
                Ok(())
            }
        },
    )?;

    let ui = environment
        .get(OsStr::new("ci"))
        .or_else(|| environment.get(OsStr::new("no_color")))
        .and_then(|value| {
            // If either of these are truthy, then we disable the TUI
            if value == "true" || value == "1" {
                Some(UIMode::Stream)
            } else {
                None
            }
        });

    let output = ConfigurationOptions {
        api_url: None,
        login_url: None,
        team_slug: None,
        team_id: output_map.get("team_id").cloned(),
        token: output_map.get("token").cloned(),
        scm_base: None,
        scm_head: None,

        signature: None,
        preflight: None,
        enabled: None,
        ui,
        daemon: None,
        timeout: None,
        upload_timeout: None,
        spaces_id: None,
        allow_no_package_manager: None,
        env_mode: None,
        cache_dir: None,
    };

    Ok(output)
}

fn truth_env_var(s: &str) -> Option<bool> {
    match s {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}
