//! Turborepo environment manifest support from Portless 0.15.1.

use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const LOADER_FILENAME: &str = "turbo-env-loader.cjs";
pub const MANIFEST_FILENAME: &str = "dev-manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestEntry {
    #[serde(rename = "PORT")]
    pub port: String,
    #[serde(rename = "HOST")]
    pub host: String,
    #[serde(rename = "PORTLESS_URL")]
    pub portless_url: String,
    #[serde(
        rename = "__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS",
        skip_serializing_if = "Option::is_none"
    )]
    pub vite_additional_server_allowed_hosts: Option<String>,
    #[serde(
        rename = "NODE_EXTRA_CA_CERTS",
        skip_serializing_if = "Option::is_none"
    )]
    pub node_extra_ca_certs: Option<String>,
}

pub type Manifest = BTreeMap<String, ManifestEntry>;

#[derive(Debug, Error)]
pub enum TurboError {
    #[error("could not determine the Portless user state directory")]
    MissingHomeDirectory,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("could not serialize the Turbo environment manifest: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn user_state_dir() -> Result<PathBuf, TurboError> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".portless"))
        .ok_or(TurboError::MissingHomeDirectory)
}

pub fn loader_path(base_dir: &Path) -> PathBuf {
    base_dir.join(LOADER_FILENAME)
}

pub fn manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(MANIFEST_FILENAME)
}

/// Generate the CommonJS preload script byte-for-byte like Portless 0.15.1.
pub fn loader_source(base_dir: &Path) -> Result<String, serde_json::Error> {
    let encoded_base = serde_json::to_string(&base_dir.to_string_lossy())?;
    Ok(format!(
        concat!(
            "\"use strict\";\n",
            "var fs = require(\"fs\");\n",
            "var path = require(\"path\");\n",
            "var manifestPath = path.join({encoded_base}, \"dev-manifest.json\");\n",
            "try {{\n",
            "  var raw = fs.readFileSync(manifestPath, \"utf-8\");\n",
            "  var manifest = JSON.parse(raw);\n",
            "  var cwd = process.cwd();\n",
            "  var entry = manifest[cwd];\n",
            "  if (entry && typeof entry === \"object\") {{\n",
            "    var keys = Object.keys(entry);\n",
            "    for (var i = 0; i < keys.length; i++) {{\n",
            "      process.env[keys[i]] = entry[keys[i]];\n",
            "    }}\n",
            "  }}\n",
            "}} catch (_) {{}}\n"
        ),
        encoded_base = encoded_base
    ))
}

fn ensure_directory(base_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(base_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(base_dir, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn write_mode_644(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}

/// Create or refresh the loader. Returns `true` only when the file changed.
pub fn ensure_env_loader(base_dir: &Path) -> Result<bool, TurboError> {
    ensure_directory(base_dir)?;
    let target = loader_path(base_dir);
    let source = loader_source(base_dir)?;
    if fs::read_to_string(&target).is_ok_and(|existing| existing == source) {
        return Ok(false);
    }
    write_mode_644(&target, source.as_bytes())?;
    Ok(true)
}

/// Write pretty JSON with the trailing newline emitted by `turbo.ts`.
pub fn write_manifest(entries: &Manifest, base_dir: &Path) -> Result<(), TurboError> {
    ensure_directory(base_dir)?;
    let mut contents = serde_json::to_string_pretty(entries)?;
    contents.push('\n');
    write_mode_644(&manifest_path(base_dir), contents.as_bytes())?;
    Ok(())
}

/// Remove the transient manifest. Missing files are treated as success.
pub fn remove_manifest(base_dir: &Path) -> io::Result<()> {
    match fs::remove_file(manifest_path(base_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Prepend the loader's `--require` flag while preserving `NODE_OPTIONS`.
pub fn build_node_options_with_existing(base_dir: &Path, existing: Option<&str>) -> String {
    let loader = loader_path(base_dir);
    let loader = loader.to_string_lossy();
    let require = if loader.contains(' ') {
        format!("--require \"{loader}\"")
    } else {
        format!("--require {loader}")
    };
    existing
        .filter(|value| !value.is_empty())
        .map_or_else(|| require.clone(), |value| format!("{require} {value}"))
}

pub fn build_node_options(base_dir: &Path) -> String {
    let existing = env::var("NODE_OPTIONS").ok();
    build_node_options_with_existing(base_dir, existing.as_deref())
}

/// Return true only when `turbo.json` exists and can be opened for reading.
pub fn has_turbo_config(workspace_root: &Path) -> bool {
    fs::File::open(workspace_root.join("turbo.json")).is_ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn entry() -> ManifestEntry {
        ManifestEntry {
            port: "3001".to_owned(),
            host: "127.0.0.1".to_owned(),
            portless_url: "https://app.project.local".to_owned(),
            vite_additional_server_allowed_hosts: Some(".localhost,.local".to_owned()),
            node_extra_ca_certs: Some("/home/user/.portless/ca.pem".to_owned()),
        }
    }

    #[test]
    fn detects_readable_turbo_config() {
        let directory = tempdir().unwrap();
        assert!(!has_turbo_config(directory.path()));
        fs::write(directory.path().join("turbo.json"), "{}").unwrap();
        assert!(has_turbo_config(directory.path()));
    }

    #[test]
    fn creates_updates_and_reuses_loader() {
        let directory = tempdir().unwrap();
        assert!(ensure_env_loader(directory.path()).unwrap());
        let target = loader_path(directory.path());
        let expected = loader_source(directory.path()).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), expected);
        assert!(!ensure_env_loader(directory.path()).unwrap());

        fs::write(&target, "old content").unwrap();
        assert!(ensure_env_loader(directory.path()).unwrap());
        assert_eq!(fs::read_to_string(target).unwrap(), expected);
    }

    #[test]
    fn loader_matches_commonjs_source_and_escapes_windows_paths_once() {
        let source = loader_source(Path::new(r"C:\Users\test\.portless")).unwrap();
        assert!(source.starts_with("\"use strict\";\nvar fs = require(\"fs\");"));
        assert!(source.contains("process.env[keys[i]] = entry[keys[i]];"));
        assert!(source.contains(r#"path.join("C:\\Users\\test\\.portless", "dev-manifest.json")"#));
        assert!(!source.contains(r"C:\\\\Users"));
    }

    #[test]
    fn writes_exact_manifest_shape_and_removes_it_idempotently() {
        let directory = tempdir().unwrap();
        let entries = Manifest::from([("/path/to/app".to_owned(), entry())]);
        write_manifest(&entries, directory.path()).unwrap();

        let raw = fs::read_to_string(manifest_path(directory.path())).unwrap();
        assert!(raw.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["/path/to/app"]["PORT"], "3001");
        assert_eq!(
            parsed["/path/to/app"]["NODE_EXTRA_CA_CERTS"],
            "/home/user/.portless/ca.pem"
        );
        assert!(parsed["/path/to/app"].get("port").is_none());

        remove_manifest(directory.path()).unwrap();
        remove_manifest(directory.path()).unwrap();
        assert!(!manifest_path(directory.path()).exists());
    }

    #[test]
    fn omits_optional_manifest_fields() {
        let directory = tempdir().unwrap();
        let mut value = entry();
        value.vite_additional_server_allowed_hosts = None;
        value.node_extra_ca_certs = None;
        write_manifest(
            &Manifest::from([("/app".to_owned(), value)]),
            directory.path(),
        )
        .unwrap();
        let raw = fs::read_to_string(manifest_path(directory.path())).unwrap();
        assert!(!raw.contains("NODE_EXTRA_CA_CERTS"));
        assert!(!raw.contains("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS"));
    }

    #[test]
    fn prepends_node_options_and_quotes_spaced_paths() {
        assert_eq!(
            build_node_options_with_existing(Path::new("/tmp/state"), None),
            "--require /tmp/state/turbo-env-loader.cjs"
        );
        assert_eq!(
            build_node_options_with_existing(
                Path::new("/tmp/state"),
                Some("--max-old-space-size=4096")
            ),
            "--require /tmp/state/turbo-env-loader.cjs --max-old-space-size=4096"
        );
        assert_eq!(
            build_node_options_with_existing(Path::new("/tmp/path with spaces"), None),
            "--require \"/tmp/path with spaces/turbo-env-loader.cjs\""
        );
    }

    #[cfg(unix)]
    #[test]
    fn writes_portless_file_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempdir().unwrap();
        ensure_env_loader(directory.path()).unwrap();
        write_manifest(&Manifest::new(), directory.path()).unwrap();
        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(loader_path(directory.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
        assert_eq!(
            fs::metadata(manifest_path(directory.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }
}
