//! JavaScript root recognition, independent of graph contributor construction.

use std::io::ErrorKind;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    package_json::{self, PackageJson},
    package_manager::{self, PackageManager},
    workspaces::WorkspaceGlobs,
};

/// Optional JavaScript facts, never a prerequisite for identifying a
/// repository.
#[derive(Debug)]
pub struct JavaScriptRoot {
    pub package_json: PackageJson,
    pub package_manager: Result<PackageManager, package_manager::Error>,
}

/// A construction-scoped root observation; probing never enumerates
/// packages or constructs a graph contributor.
#[derive(Debug)]
pub struct JavaScriptWorkspace {
    root: AbsoluteSystemPathBuf,
    metadata: JavaScriptRoot,
    globs: Option<WorkspaceGlobs>,
}

impl JavaScriptWorkspace {
    pub fn root(&self) -> &AbsoluteSystemPath {
        &self.root
    }

    pub fn is_workspace(&self) -> bool {
        self.globs.is_some()
    }

    /// Preserve JavaScript's declared glob membership, including
    /// exclusions. Invalid/outside-root membership queries do not
    /// claim a package.
    pub fn owns(&self, target: &AbsoluteSystemPath) -> bool {
        self.root.contains(target)
            && self.globs.as_ref().is_some_and(|globs| {
                globs
                    .target_is_workspace(&self.root, target)
                    .unwrap_or(false)
            })
    }

    /// Whether this manifest supplies JavaScript graph behavior, rather than
    /// incidental repository metadata such as a name, version or license.
    /// Native-only repositories must not acquire a package-manager requirement
    /// merely because they publish metadata in package.json. Explicit manager,
    /// workspace, dependency and task declarations still receive normal JS
    /// validation, including invalid package-manager diagnostics.
    pub fn participates_in_graph(&self) -> bool {
        let package = &self.metadata.package_json;
        self.metadata.package_manager.is_ok()
            || package.package_manager.is_some()
            || package.dev_engines.is_some()
            || !package.scripts.is_empty()
            || package.all_dependencies().next().is_some()
            || package.other.contains_key("workspaces")
            || package.pnpm.is_some()
            || package.resolutions.is_some()
            || package.patched_dependencies.is_some()
            || self.root.join_component("pnpm-workspace.yaml").exists()
            || self.root.join_component("aube-workspace.yaml").exists()
    }

    pub fn into_root(self) -> JavaScriptRoot {
        self.metadata
    }
}

pub struct JavaScriptBootstrap;

impl JavaScriptBootstrap {
    /// A missing manifest makes no root claim; malformed/unreadable
    /// manifests remain errors. Package-manager and glob failures
    /// retain the legacy standalone-package fallback, with the
    /// manager error kept in metadata.
    pub fn probe(
        dir: &AbsoluteSystemPath,
    ) -> Result<Option<JavaScriptWorkspace>, package_json::Error> {
        match Self::observe(dir) {
            Ok(workspace) => Ok(Some(workspace)),
            Err(package_json::Error::Io(io)) if io.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Observe once, retaining the original missing-manifest error for graph
    /// callers.
    pub(super) fn observe(
        dir: &AbsoluteSystemPath,
    ) -> Result<JavaScriptWorkspace, package_json::Error> {
        let package_json = PackageJson::load(&dir.join_component("package.json"))?;
        let package_manager = PackageManager::read_or_detect_package_manager(&package_json, dir);
        let globs = package_manager
            .as_ref()
            .ok()
            .and_then(|manager| manager.get_workspace_globs(dir).ok());
        Ok(JavaScriptWorkspace {
            root: dir.to_owned(),
            metadata: JavaScriptRoot {
                package_json,
                package_manager,
            },
            globs,
        })
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::bootstrap::{Registry, RepositoryBootstrap};

    #[test_case(r#"{"name":"metadata","private":true,"version":"1.0.0"}"#, false; "metadata_only")]
    #[test_case(r#"{"scripts":{"build":"echo build"}}"#, true; "scripts")]
    #[test_case(r#"{"dependencies":{"library":"1.0.0"}}"#, true; "dependencies")]
    #[test_case(r#"{"devDependencies":{"turbo":"2.0.0"}}"#, true; "dev_dependencies")]
    #[test_case(r#"{"workspaces":[]}"#, true; "empty_workspace_declaration")]
    #[test_case(r#"{"packageManager":"invalid"}"#, true; "invalid_explicit_manager")]
    #[test_case(r#"{"devEngines":{"packageManager":{"name":"invalid"}}}"#, true; "invalid_dev_engines")]
    fn native_root_only_omits_incidental_javascript(contents: &str, participates: bool) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        root.join_component("package.json")
            .create_with_contents(contents)
            .unwrap();
        root.join_component("Cargo.toml")
            .create_with_contents("[workspace]\nmembers = []\n")
            .unwrap();
        let bootstrap =
            RepositoryBootstrap::new(Registry::new([crate::toolchain::ToolchainId::RUST]));
        let observation = bootstrap.observe(root).unwrap();
        assert!(observation.is_repository());
        let (javascript, plan) = observation.into_graph_parts().unwrap();
        assert_eq!(javascript.is_some(), participates);
        assert_eq!(plan.native_workspaces().len(), 1);

        // Omitting metadata is permitted only when another ecosystem recognizes
        // the root; JS-only repositories retain their legacy validation path.
        let (javascript, _) = RepositoryBootstrap::default()
            .observe(root)
            .unwrap()
            .into_graph_parts()
            .unwrap();
        assert!(javascript.is_some());
    }
}
