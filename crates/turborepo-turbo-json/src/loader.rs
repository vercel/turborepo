//! TurboJson loading utilities
//!
//! This module provides utilities for reading turbo.json files from disk.
//! Higher-level loading strategies (workspace, MFE enrichment, etc.) are
//! implemented in turborepo-lib.

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{Error, FutureFlags, TurboJson};

/// Configuration file names
pub const CONFIG_FILE: &str = "turbo.json";
pub const CONFIG_FILE_JSONC: &str = "turbo.jsonc";

/// A helper structure configured with all settings related to reading a
/// `turbo.json` file from disk.
#[derive(Debug, Clone)]
pub struct TurboJsonReader {
    repo_root: AbsoluteSystemPathBuf,
    future_flags: FutureFlags,
}

impl TurboJsonReader {
    /// Create a new TurboJsonReader with the given repo root
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            future_flags: Default::default(),
        }
    }

    /// Set the future flags for this reader
    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.future_flags = future_flags;
        self
    }

    /// Read a turbo.json file from the given path
    ///
    /// # Arguments
    /// * `path` - The path to the turbo.json file
    /// * `is_root` - Whether this is a root turbo.json (affects schema
    ///   validation)
    ///
    /// # Returns
    /// * `Ok(Some(TurboJson))` - Successfully read and parsed the file
    /// * `Ok(None)` - File does not exist
    /// * `Err(Error)` - Error reading or parsing the file
    pub fn read(
        &self,
        path: &AbsoluteSystemPath,
        is_root: bool,
    ) -> Result<Option<TurboJson>, Error> {
        TurboJson::read(&self.repo_root, path, is_root, self.future_flags)
    }

    /// Get the repo root path
    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }

    /// Get the future flags
    pub fn future_flags(&self) -> FutureFlags {
        self.future_flags
    }
}

/// Represents where to look for a turbo.json file
#[derive(Debug, Clone)]
pub enum TurboJsonPath<'a> {
    /// Look for turbo.json/turbo.jsonc in this directory
    Dir(&'a AbsoluteSystemPath),
    /// Only use this specific file path (does not need to be named turbo.json)
    File(&'a AbsoluteSystemPath),
}

/// Load a turbo.json from a path, handling both turbo.json and turbo.jsonc
///
/// This function handles the logic of:
/// - Looking for both turbo.json and turbo.jsonc in a directory
/// - Erroring if both exist
/// - Returning the appropriate one if only one exists
pub fn load_from_path(
    reader: &TurboJsonReader,
    turbo_json_path: TurboJsonPath,
    is_root: bool,
) -> Result<TurboJson, Error> {
    let result = match turbo_json_path {
        TurboJsonPath::Dir(turbo_json_dir_path) => {
            let turbo_json_path = turbo_json_dir_path.join_component(CONFIG_FILE);
            let turbo_jsonc_path = turbo_json_dir_path.join_component(CONFIG_FILE_JSONC);

            // Load both turbo.json and turbo.jsonc
            let turbo_json = reader.read(&turbo_json_path, is_root);
            let turbo_jsonc = reader.read(&turbo_jsonc_path, is_root);

            select_turbo_json(turbo_json_dir_path, turbo_json, turbo_jsonc)
        }
        TurboJsonPath::File(turbo_json_path) => reader.read(turbo_json_path, is_root),
    };

    // Handle errors or success
    match result {
        // There was an error, and we don't have any chance of recovering
        Err(e) => Err(e),
        Ok(None) => Err(Error::NoTurboJSON),
        // We're not synthesizing anything and there was no error, we're done
        Ok(Some(turbo)) => Ok(turbo),
    }
}

/// Helper for selecting the correct turbo.json read result when both
/// turbo.json and turbo.jsonc might exist
fn select_turbo_json(
    turbo_json_dir_path: &AbsoluteSystemPath,
    turbo_json: Result<Option<TurboJson>, Error>,
    turbo_jsonc: Result<Option<TurboJson>, Error>,
) -> Result<Option<TurboJson>, Error> {
    tracing::debug!(
        "path: {}, turbo_json: {:?}, turbo_jsonc: {:?}",
        turbo_json_dir_path.as_str(),
        turbo_json.as_ref().map(|v| v.as_ref().map(|_| ())),
        turbo_jsonc.as_ref().map(|v| v.as_ref().map(|_| ()))
    );
    match (turbo_json, turbo_jsonc) {
        // If both paths contain valid turbo.json error
        (Ok(Some(_)), Ok(Some(_))) => Err(Error::MultipleTurboConfigs {
            directory: turbo_json_dir_path.to_string(),
        }),
        // If turbo.json is valid and turbo.jsonc is missing or invalid, use turbo.json
        (Ok(Some(turbo_json)), Ok(None)) | (Ok(Some(turbo_json)), Err(_)) => Ok(Some(turbo_json)),
        // If turbo.jsonc is valid and turbo.json is missing or invalid, use turbo.jsonc
        (Ok(None), Ok(Some(turbo_jsonc))) | (Err(_), Ok(Some(turbo_jsonc))) => {
            Ok(Some(turbo_jsonc))
        }
        // If neither are present, then choose nothing
        (Ok(None), Ok(None)) => Ok(None),
        // If only one has an error return the failure
        (Err(e), Ok(None)) | (Ok(None), Err(e)) => Err(e),
        // If both fail then just return error for `turbo.json`
        (Err(e), Err(_)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_load_from_path_with_both_files() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create both turbo.json and turbo.jsonc
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

        fs::write(&turbo_json_path, "{}").unwrap();
        fs::write(&turbo_jsonc_path, "{}").unwrap();

        // Should error when both files exist
        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::MultipleTurboConfigs { .. }
        ));
    }

    #[test]
    fn test_load_from_path_with_only_turbo_json() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.json
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        fs::write(&turbo_json_path, "{}").unwrap();

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_from_path_with_only_turbo_jsonc() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.jsonc
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);
        fs::write(&turbo_jsonc_path, "{}").unwrap();

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_from_path_no_file() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::NoTurboJSON));
    }
}
