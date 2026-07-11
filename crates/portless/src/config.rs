//! Loading and resolution of `portless.json` and `package.json` configuration.

use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const CONFIG_FILENAME: &str = "portless.json";
const TOP_LEVEL_KEYS: &[&str] = &["name", "script", "appPort", "proxy", "apps", "turbo"];
const APP_KEYS: &[&str] = &["name", "script", "appPort", "proxy"];

/// Configuration fields applicable to one application.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub name: Option<String>,
    pub script: Option<String>,
    pub app_port: Option<u16>,
    pub proxy: Option<bool>,
}

impl AppConfig {
    /// Merge a closer configuration over this one, field by field.
    #[must_use]
    pub fn merged_with(&self, closer: &Self) -> Self {
        Self {
            name: closer.name.clone().or_else(|| self.name.clone()),
            script: closer.script.clone().or_else(|| self.script.clone()),
            app_port: closer.app_port.or(self.app_port),
            proxy: closer.proxy.or(self.proxy),
        }
    }
}

/// Root Portless configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortlessConfig {
    pub name: Option<String>,
    pub script: Option<String>,
    pub app_port: Option<u16>,
    pub proxy: Option<bool>,
    pub apps: Option<BTreeMap<String, AppConfig>>,
    pub turbo: Option<bool>,
}

/// A loaded configuration and the directory against which app paths are
/// resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfig {
    pub config: PortlessConfig,
    pub config_dir: PathBuf,
}

/// A malformed config or an I/O failure that TypeScript would propagate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigValidationError {
    message: String,
}

impl ConfigValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigValidationError {}

/// Load `portless.json`, falling back to the current directory's
/// `package.json`.
pub fn load_config(cwd: impl AsRef<Path>) -> Result<Option<LoadedConfig>, ConfigValidationError> {
    let cwd = cwd.as_ref();
    let config_path = cwd.join(CONFIG_FILENAME);
    match fs::read_to_string(&config_path) {
        Ok(raw) => {
            let value: Value = serde_json::from_str(&raw).map_err(|_| {
                ConfigValidationError::new(format!("Invalid JSON in {}", config_path.display()))
            })?;
            let config = parse_config(value, &config_path.display().to_string())?;
            Ok(Some(LoadedConfig {
                config,
                config_dir: cwd.to_path_buf(),
            }))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => load_config_from_package_json(cwd),
        Err(error) => Err(ConfigValidationError::new(error.to_string())),
    }
}

/// Load configuration from the process's current directory.
pub fn load_config_from_current_dir() -> Result<Option<LoadedConfig>, ConfigValidationError> {
    let cwd =
        std::env::current_dir().map_err(|error| ConfigValidationError::new(error.to_string()))?;
    load_config(cwd)
}

/// Load only the `portless` value from a package's `package.json`.
pub fn load_package_portless_config(
    dir: impl AsRef<Path>,
) -> Result<Option<AppConfig>, ConfigValidationError> {
    let path = dir.as_ref().join("package.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    let Ok(package): Result<Value, _> = serde_json::from_str(&raw) else {
        return Ok(None);
    };
    let Some(raw_config) = package
        .as_object()
        .and_then(|object| object.get("portless"))
    else {
        return Ok(None);
    };
    let Some(config) = normalize_portless_value(raw_config) else {
        return Ok(None);
    };
    let Some(object) = config.as_object() else {
        return Ok(None);
    };
    validate_app_config(object, "portless", &path.display().to_string())?;
    deserialize_app(config.clone()).map(Some)
}

/// Resolve an app entry by matching the package path and then its ancestors.
#[must_use]
pub fn resolve_app_config(
    config: &PortlessConfig,
    config_dir: impl AsRef<Path>,
    package_dir: impl AsRef<Path>,
) -> AppConfig {
    let Some(apps) = &config.apps else {
        return AppConfig {
            name: config.name.clone(),
            script: config.script.clone(),
            app_port: config.app_port,
            proxy: config.proxy,
        };
    };

    let Ok(relative) = package_dir.as_ref().strip_prefix(config_dir.as_ref()) else {
        return AppConfig::default();
    };
    if relative.as_os_str().is_empty() {
        return AppConfig::default();
    }

    let mut candidate = normalize_path(relative);
    if candidate.starts_with("..") {
        return AppConfig::default();
    }
    while !candidate.is_empty() {
        if let Some(app) = apps.get(&candidate) {
            return app.clone();
        }
        let parent = Path::new(&candidate)
            .parent()
            .map(normalize_path)
            .unwrap_or_default();
        if parent == "." || parent == candidate {
            break;
        }
        candidate = parent;
    }
    AppConfig::default()
}

/// Resolve and split a script's command text.
#[must_use]
pub fn resolve_script(script_name: &str, package_dir: impl AsRef<Path>) -> Option<Vec<String>> {
    script_value(script_name, package_dir.as_ref())
        .filter(|script| !script.trim().is_empty())
        .map(|script| split_command(&script))
}

/// Check whether a package defines a string-valued script.
#[must_use]
pub fn has_script(script_name: &str, dir: impl AsRef<Path>) -> bool {
    script_value(script_name, dir.as_ref()).is_some()
}

/// Supported JavaScript package managers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "npm" => Some(Self::Npm),
            "pnpm" => Some(Self::Pnpm),
            "yarn" => Some(Self::Yarn),
            "bun" => Some(Self::Bun),
            _ => None,
        }
    }
}

impl fmt::Display for PackageManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Detect the nearest package manager, preferring `packageManager` over
/// lockfiles.
#[must_use]
pub fn detect_package_manager(cwd: impl AsRef<Path>) -> PackageManager {
    const LOCK_FILES: &[(&str, PackageManager)] = &[
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("bun.lockb", PackageManager::Bun),
        ("bun.lock", PackageManager::Bun),
        ("package-lock.json", PackageManager::Npm),
    ];

    for dir in cwd.as_ref().ancestors() {
        if let Ok(raw) = fs::read_to_string(dir.join("package.json"))
            && let Ok(package) = serde_json::from_str::<Value>(&raw)
        {
            let manager = package
                .get("packageManager")
                .and_then(Value::as_str)
                .and_then(|value| value.split('@').next())
                .and_then(PackageManager::from_name);
            if let Some(manager) = manager {
                return manager;
            }
        }

        for (file, manager) in LOCK_FILES {
            if dir.join(file).exists() {
                return *manager;
            }
        }
    }
    PackageManager::Npm
}

/// Return a package-manager delegated command for a named script.
#[must_use]
pub fn resolve_script_command(
    script_name: &str,
    package_dir: impl AsRef<Path>,
) -> Option<Vec<String>> {
    let package_dir = package_dir.as_ref();
    has_script(script_name, package_dir).then(|| {
        vec![
            detect_package_manager(package_dir).to_string(),
            "run".to_owned(),
            script_name.to_owned(),
        ]
    })
}

/// Split command text on whitespace while respecting quotes and backslash
/// escapes.
#[must_use]
pub fn split_command(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && !in_single {
            escaped = true;
        } else if character == '\'' && !in_double {
            in_single = !in_single;
        } else if character == '"' && !in_single {
            in_double = !in_double;
        } else if character.is_whitespace() && !in_single && !in_double {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Whether command arguments appear to launch a server rather than a build
/// watcher.
#[must_use]
pub fn is_server_command<S: AsRef<str>>(args: &[S]) -> bool {
    const BUILD_ONLY: &[&str] = &[
        "tsup",
        "tsc",
        "esbuild",
        "rollup",
        "babel",
        "swc",
        "unbuild",
        "pkgroll",
        "ncc",
        "microbundle",
    ];
    let Some(first) = args.first() else {
        return false;
    };
    let binary = Path::new(first.as_ref())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(first.as_ref());
    !BUILD_ONLY.contains(&binary)
}

fn load_config_from_package_json(
    dir: &Path,
) -> Result<Option<LoadedConfig>, ConfigValidationError> {
    let path = dir.join("package.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigValidationError::new(error.to_string())),
    };
    let Ok(package): Result<Value, _> = serde_json::from_str(&raw) else {
        return Ok(None);
    };
    let Some(value) = package
        .as_object()
        .and_then(|object| object.get("portless"))
    else {
        return Ok(None);
    };
    let Some(value) = normalize_portless_value(value) else {
        return Ok(None);
    };
    let label = format!("{} \"portless\"", path.display());
    let config = parse_config(value, &label)?;
    Ok(Some(LoadedConfig {
        config,
        config_dir: dir.to_path_buf(),
    }))
}

fn normalize_portless_value(value: &Value) -> Option<Value> {
    if value.is_null() {
        return None;
    }
    if let Some(name) = value.as_str() {
        let name = name.trim();
        return (!name.is_empty()).then(|| {
            let mut object = Map::new();
            object.insert("name".to_owned(), Value::String(name.to_owned()));
            Value::Object(object)
        });
    }
    Some(value.clone())
}

fn parse_config(
    mut value: Value,
    config_path: &str,
) -> Result<PortlessConfig, ConfigValidationError> {
    let object = value.as_object().ok_or_else(|| {
        ConfigValidationError::new(format!("{config_path} must be a JSON object."))
    })?;
    validate_common_fields(object, "", config_path)?;
    validate_optional_bool(object, "turbo", "", config_path)?;
    if let Some(apps) = object.get("apps") {
        let entries = apps.as_object().ok_or_else(|| {
            ConfigValidationError::new(format!("\"apps\" in {config_path} must be an object."))
        })?;
        for (key, value) in entries {
            let app = value.as_object().ok_or_else(|| {
                ConfigValidationError::new(format!(
                    "\"apps.{key}\" in {config_path} must be an object."
                ))
            })?;
            validate_app_config(app, &format!("apps.{key}"), config_path)?;
        }
    }
    warn_unknown_keys(object, TOP_LEVEL_KEYS, config_path, None);
    normalize_app_ports(&mut value);
    serde_json::from_value(value).map_err(|error| ConfigValidationError::new(error.to_string()))
}

fn deserialize_app(mut value: Value) -> Result<AppConfig, ConfigValidationError> {
    normalize_app_port(&mut value);
    serde_json::from_value(value).map_err(|error| ConfigValidationError::new(error.to_string()))
}

fn validate_app_config(
    object: &Map<String, Value>,
    prefix: &str,
    config_path: &str,
) -> Result<(), ConfigValidationError> {
    validate_common_fields(object, prefix, config_path)?;
    warn_unknown_keys(object, APP_KEYS, config_path, Some(prefix));
    Ok(())
}

fn validate_common_fields(
    object: &Map<String, Value>,
    prefix: &str,
    config_path: &str,
) -> Result<(), ConfigValidationError> {
    validate_non_empty_string(object, "name", prefix, config_path)?;
    validate_non_empty_string(object, "script", prefix, config_path)?;
    if let Some(value) = object.get("appPort") {
        let valid = value
            .as_f64()
            .is_some_and(|port| port.fract() == 0.0 && (1.0..=f64::from(u16::MAX)).contains(&port));
        if !valid {
            return Err(ConfigValidationError::new(format!(
                "\"{}appPort\" in {config_path} must be an integer between 1 and 65535.",
                field_prefix(prefix)
            )));
        }
    }
    validate_optional_bool(object, "proxy", prefix, config_path)
}

fn normalize_app_ports(value: &mut Value) {
    normalize_app_port(value);
    if let Some(apps) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("apps"))
        .and_then(Value::as_object_mut)
    {
        for app in apps.values_mut() {
            normalize_app_port(app);
        }
    }
}

fn normalize_app_port(value: &mut Value) {
    let Some(port) = value
        .as_object()
        .and_then(|object| object.get("appPort"))
        .and_then(Value::as_f64)
    else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "appPort".to_owned(),
            Value::Number(serde_json::Number::from(port as u64)),
        );
    }
}

fn validate_non_empty_string(
    object: &Map<String, Value>,
    key: &str,
    prefix: &str,
    config_path: &str,
) -> Result<(), ConfigValidationError> {
    if let Some(value) = object.get(key) {
        let valid = value
            .as_str()
            .is_some_and(|string| !string.trim().is_empty());
        if !valid {
            return Err(ConfigValidationError::new(format!(
                "\"{}{key}\" in {config_path} must be a non-empty string.",
                field_prefix(prefix)
            )));
        }
    }
    Ok(())
}

fn validate_optional_bool(
    object: &Map<String, Value>,
    key: &str,
    prefix: &str,
    config_path: &str,
) -> Result<(), ConfigValidationError> {
    if object.get(key).is_some_and(|value| !value.is_boolean()) {
        return Err(ConfigValidationError::new(format!(
            "\"{}{key}\" in {config_path} must be a boolean.",
            field_prefix(prefix)
        )));
    }
    Ok(())
}

fn field_prefix(prefix: &str) -> String {
    if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}.")
    }
}

fn warn_unknown_keys(
    object: &Map<String, Value>,
    known: &[&str],
    config_path: &str,
    prefix: Option<&str>,
) {
    for key in object.keys().filter(|key| !known.contains(&key.as_str())) {
        let label = prefix.map_or_else(|| key.to_owned(), |prefix| format!("{prefix}.{key}"));
        eprintln!(
            "Warning: Unknown key \"{label}\" in {config_path}. Known keys: {}",
            known.join(", ")
        );
    }
}

fn script_value(script_name: &str, package_dir: &Path) -> Option<String> {
    let package: Value =
        serde_json::from_str(&fs::read_to_string(package_dir.join("package.json")).ok()?).ok()?;
    package
        .get("scripts")?
        .get(script_name)?
        .as_str()
        .map(str::to_owned)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn split_command_matches_shell_like_upstream_subset() {
        assert_eq!(
            split_command(r#"KEY='val with spaces' echo \"hello\" path\ with\ spaces"#),
            [
                "KEY=val with spaces",
                "echo",
                "\"hello\"",
                "path with spaces"
            ]
        );
    }

    #[test]
    fn portless_json_precedes_package_json_and_validates() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("package.json"),
            r#"{"portless":{"name":"package"}}"#,
        )
        .expect("package");
        fs::write(temp.path().join("portless.json"), r#"{"name":"file"}"#).expect("config");
        assert_eq!(
            load_config(temp.path())
                .expect("valid")
                .expect("loaded")
                .config
                .name
                .as_deref(),
            Some("file")
        );

        fs::write(temp.path().join("portless.json"), r#"{"appPort":0}"#).expect("invalid");
        assert!(load_config(temp.path()).is_err());

        fs::write(temp.path().join("portless.json"), r#"{"appPort":3000.0}"#)
            .expect("integer-valued number");
        assert_eq!(
            load_config(temp.path())
                .expect("valid number")
                .expect("loaded")
                .config
                .app_port,
            Some(3000)
        );
    }

    #[test]
    fn null_package_config_is_ignored() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"app","portless":null}"#,
        )
        .expect("package");
        assert_eq!(load_config(temp.path()).expect("valid package"), None);
        assert_eq!(
            load_package_portless_config(temp.path()).expect("valid package"),
            None
        );
    }

    #[test]
    fn package_config_merges_over_root_app_entry() {
        let root = AppConfig {
            name: Some("root".to_owned()),
            script: Some("dev".to_owned()),
            app_port: Some(3000),
            proxy: None,
        };
        let package = AppConfig {
            name: Some("package".to_owned()),
            proxy: Some(false),
            ..AppConfig::default()
        };
        assert_eq!(
            root.merged_with(&package),
            AppConfig {
                name: Some("package".to_owned()),
                script: Some("dev".to_owned()),
                app_port: Some(3000),
                proxy: Some(false)
            }
        );
    }

    #[test]
    fn detects_manager_and_resolves_delegated_script() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(temp.path().join("pnpm-lock.yaml"), "").expect("lock");
        fs::write(
            temp.path().join("package.json"),
            r#"{"scripts":{"dev":"next dev"}}"#,
        )
        .expect("package");
        assert_eq!(
            resolve_script_command("dev", temp.path()),
            Some(vec!["pnpm".to_owned(), "run".to_owned(), "dev".to_owned()])
        );
        assert!(!is_server_command(&["tsc", "--watch"]));
        assert!(is_server_command(&["next", "dev"]));
    }
}
