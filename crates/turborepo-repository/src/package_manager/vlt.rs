use std::{collections::BTreeMap, fs};

use serde::Deserialize;
use turbopath::AbsoluteSystemPath;

use crate::package_manager::{Error, PackageJsonWorkspaces, PackageManager, pnpm};

pub const LOCKFILE: &str = "vlt-lock.json";
/// vlt's project configuration. Besides `workspaces` it holds settings an
/// install depends on, such as registries, catalogs and graph modifiers.
pub const WORKSPACE_CONFIGURATION_PATH: &str = "vlt.json";

pub struct VltDetector<'a> {
    repo_root: &'a AbsoluteSystemPath,
    found: bool,
}

impl<'a> VltDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }
}

impl Iterator for VltDetector<'_> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        self.repo_root
            .join_component(LOCKFILE)
            .exists()
            .then_some(Ok(PackageManager::Vlt))
    }
}

#[derive(Debug, Deserialize)]
struct VltJson {
    workspaces: Option<Workspaces>,
}

/// The `workspaces` field of `vlt.json`: a glob, a list of globs, or globs
/// under named groups.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Workspaces {
    Globs(Globs),
    Groups(BTreeMap<String, Globs>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Globs {
    One(String),
    Many(Vec<String>),
}

impl From<Globs> for Vec<String> {
    fn from(globs: Globs) -> Self {
        match globs {
            Globs::One(glob) => vec![glob],
            Globs::Many(globs) => globs,
        }
    }
}

/// Workspace globs the way vlt resolves them: the `workspaces` field of
/// `vlt.json` wins whenever it is set, then comes the one of the root
/// `package.json`, then the `packages` of a `pnpm-workspace.yaml`.
///
/// Returns `None` if none of them declares any workspaces.
pub fn get_configured_workspace_globs(repo_root: &AbsoluteSystemPath) -> Option<Vec<String>> {
    let read = |file: &str| fs::read_to_string(repo_root.join_component(file)).ok();

    let vlt_json = read(WORKSPACE_CONFIGURATION_PATH)
        .and_then(|contents| serde_json::from_str::<VltJson>(&contents).ok());
    if let Some(workspaces) = vlt_json.and_then(|vlt_json| vlt_json.workspaces) {
        return Some(match workspaces {
            Workspaces::Globs(globs) => globs.into(),
            Workspaces::Groups(groups) => groups.into_values().flat_map(Vec::from).collect(),
        });
    }

    let package_json = read("package.json")
        .and_then(|contents| serde_json::from_str::<PackageJsonWorkspaces>(&contents).ok());
    if let Some(package_json) = package_json {
        return Some(package_json.workspaces.into());
    }

    pnpm::get_configured_workspace_globs(repo_root)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    const PNPM_WORKSPACE: &str = "packages:\n  - \"pnpm/*\"\n";

    fn globs(
        vlt_json: Option<serde_json::Value>,
        package_json: serde_json::Value,
        pnpm_workspace: Option<&str>,
    ) -> Option<Vec<String>> {
        let root = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(root.path()).unwrap();
        if let Some(vlt_json) = vlt_json {
            repo_root
                .join_component(WORKSPACE_CONFIGURATION_PATH)
                .create_with_contents(vlt_json.to_string())
                .unwrap();
        }
        repo_root
            .join_component("package.json")
            .create_with_contents(package_json.to_string())
            .unwrap();
        if let Some(pnpm_workspace) = pnpm_workspace {
            repo_root
                .join_component(pnpm::WORKSPACE_CONFIGURATION_PATH)
                .create_with_contents(pnpm_workspace)
                .unwrap();
        }
        get_configured_workspace_globs(&repo_root)
    }

    #[test_case(json!("packages/*"), &["packages/*"]; "glob")]
    #[test_case(json!(["packages/*", "!packages/skip"]), &["packages/*", "!packages/skip"]; "list")]
    #[test_case(json!({"apps": "apps/*", "libs": ["libs/*", "tools/*"]}), &["apps/*", "libs/*", "tools/*"]; "groups")]
    #[test_case(json!([]), &[]; "empty")]
    fn test_vlt_json_workspaces(workspaces: serde_json::Value, expected: &[&str]) {
        // set in vlt.json, so nothing else is read
        let globs = globs(
            Some(json!({"workspaces": workspaces})),
            json!({"workspaces": ["npm/*"]}),
            Some(PNPM_WORKSPACE),
        );
        assert_eq!(
            globs.as_deref(),
            Some(
                expected
                    .iter()
                    .map(|g| g.to_string())
                    .collect::<Vec<_>>()
                    .as_slice()
            )
        );
    }

    #[test_case(None; "no vlt.json")]
    #[test_case(Some(json!({"config": {}})); "vlt.json without workspaces")]
    fn test_fallbacks(vlt_json: Option<serde_json::Value>) {
        assert_eq!(
            globs(
                vlt_json.clone(),
                json!({"workspaces": ["npm/*"]}),
                Some(PNPM_WORKSPACE)
            ),
            Some(vec!["npm/*".to_string()])
        );
        assert_eq!(
            globs(
                vlt_json.clone(),
                json!({"workspaces": {"packages": ["yarn/*"]}}),
                None
            ),
            Some(vec!["yarn/*".to_string()])
        );
        assert_eq!(
            globs(vlt_json.clone(), json!({}), Some(PNPM_WORKSPACE)),
            Some(vec!["pnpm/*".to_string()])
        );
        assert_eq!(globs(vlt_json, json!({}), None), None);
    }

    #[test]
    fn test_detector() {
        let root = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(root.path()).unwrap();
        assert!(VltDetector::new(&repo_root).next().is_none());
        repo_root.join_component(LOCKFILE).create().unwrap();
        let mut detector = VltDetector::new(&repo_root);
        assert_eq!(detector.next().unwrap().unwrap(), PackageManager::Vlt);
        assert!(detector.next().is_none());
    }
}
