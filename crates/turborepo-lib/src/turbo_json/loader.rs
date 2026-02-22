//! TurboJson loading - re-exports from turborepo-turbo-json and MFE integration
//!
//! This module provides the TurboJsonLoader integration with
//! MicrofrontendsConfigs and a unified loader type that can handle both MFE and
//! non-MFE cases.

use std::collections::HashMap;

use turbopath::AbsoluteSystemPathBuf;
use turborepo_engine::BuilderError;
use turborepo_repository::{
    package_graph::{PackageInfo, PackageName},
    package_json::PackageJson,
};
// Re-export TurboJsonLoader and related types from turborepo-turbo-json
pub use turborepo_turbo_json::{
    LoaderError, NoOpUpdater, TurboJsonLoader, TurboJsonReader, TurboJsonUpdater,
};

use super::TurboJson;
use crate::{config::Error, microfrontends::MicrofrontendsConfigs};

/// Implement TurboJsonUpdater for MicrofrontendsConfigs
impl TurboJsonUpdater for MicrofrontendsConfigs {
    type Error = Error;

    fn update_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: Result<TurboJson, LoaderError>,
    ) -> Result<TurboJson, Self::Error> {
        // Convert LoaderError to config::Error for compatibility
        let turbo_json = turbo_json.map_err(loader_error_to_config_error);
        MicrofrontendsConfigs::update_turbo_json(self, package_name, turbo_json)
    }
}

/// Convert LoaderError to config::Error
fn loader_error_to_config_error(err: LoaderError) -> Error {
    match err {
        LoaderError::TurboJson(e) => Error::TurboJsonError(e),
        LoaderError::InvalidTurboJsonLoad(pkg) => Error::InvalidTurboJsonLoad(pkg),
    }
}

/// A unified TurboJson loader that can handle both MFE and non-MFE cases.
///
/// This enum wraps the generic `TurboJsonLoader` to provide a single type
/// that can be used in contexts where the loader type needs to be determined
/// at runtime (e.g., based on whether MFE configs are present).
#[derive(Debug, Clone)]
pub enum UnifiedTurboJsonLoader {
    /// Loader without MFE support (uses NoOpUpdater)
    Standard(TurboJsonLoader<NoOpUpdater>),
    /// Loader with MFE support
    WithMfe(TurboJsonLoader<MicrofrontendsConfigs>),
}

impl UnifiedTurboJsonLoader {
    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        Self::Standard(TurboJsonLoader::workspace(
            reader,
            root_turbo_json_path,
            packages,
        ))
    }

    /// Create a loader that will load turbo.json files throughout the workspace
    /// with microfrontends support.
    pub fn workspace_with_microfrontends<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        micro_frontends_configs: MicrofrontendsConfigs,
    ) -> Self {
        Self::WithMfe(TurboJsonLoader::workspace_with_updater(
            reader,
            root_turbo_json_path,
            packages,
            micro_frontends_configs,
        ))
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s, with optional microfrontends support.
    pub fn workspace_no_turbo_json<'a>(
        reader: TurboJsonReader,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        microfrontends_configs: Option<MicrofrontendsConfigs>,
    ) -> Self {
        if let Some(mfe) = microfrontends_configs {
            Self::WithMfe(TurboJsonLoader::workspace_no_turbo_json_with_updater(
                reader,
                packages,
                Some(mfe),
            ))
        } else {
            Self::Standard(TurboJsonLoader::workspace_no_turbo_json(reader, packages))
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self::Standard(TurboJsonLoader::single_package(
            reader,
            root_turbo_json,
            package_json,
        ))
    }

    /// Create a loader for task access tracing
    pub fn task_access(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self::Standard(TurboJsonLoader::task_access(
            reader,
            root_turbo_json,
            package_json,
        ))
    }

    /// Create a loader that will only return provided turbo.jsons and will
    /// never hit the file system.
    /// Primarily intended for testing
    pub fn noop(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        Self::Standard(TurboJsonLoader::noop(turbo_jsons))
    }

    /// Load a turbo.json for a given package
    pub fn load(&self, package: &PackageName) -> Result<&TurboJson, Error> {
        match self {
            Self::Standard(loader) => loader.load(package).map_err(loader_error_to_config_error),
            Self::WithMfe(loader) => loader.load(package),
        }
    }

    /// Pre-warm the cache by loading all package turbo.json files in parallel.
    /// Errors are silently ignored — subsequent `load()` calls will report
    /// them.
    pub fn preload_all(&self) {
        match self {
            Self::Standard(loader) => loader.preload_all(),
            Self::WithMfe(loader) => loader.preload_all(),
        }
    }
}

// Implement the TurboJsonLoader trait from turborepo-engine for
// UnifiedTurboJsonLoader
impl turborepo_engine::TurboJsonLoader for UnifiedTurboJsonLoader {
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError> {
        UnifiedTurboJsonLoader::load(self, package).map_err(BuilderError::from)
    }
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashSet};

    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
    use turborepo_engine::TaskDefinitionFromProcessed;
    use turborepo_errors::Spanned;
    use turborepo_repository::package_json::PackageJson;
    use turborepo_task_id::TaskName;
    use turborepo_turbo_json::TASK_ACCESS_CONFIG_PATH;
    use turborepo_types::TaskDefinition;
    use turborepo_unescape::UnescapedString;

    use super::*;

    #[test_case(
        Some(r#"{ "tasks": {"//#build": {"env": ["SPECIAL_VAR"]}} }"#),
        Some(r#"{ "tasks": {"build": {"env": ["EXPLICIT_VAR"]}} }"#),
        TaskDefinition { env: vec!["EXPLICIT_VAR".to_string()], .. Default::default() }
    ; "both present")]
    #[test_case(
        None,
        Some(r#"{ "tasks": {"build": {"env": ["EXPLICIT_VAR"]}} }"#),
        TaskDefinition { env: vec!["EXPLICIT_VAR".to_string()], .. Default::default() }
    ; "no trace")]
    #[test_case(
        Some(r#"{ "tasks": {"//#build": {"env": ["SPECIAL_VAR"]}} }"#),
        None,
        TaskDefinition { env: vec!["SPECIAL_VAR".to_string()], .. Default::default() }
    ; "no turbo.json")]
    #[test_case(
        None,
        None,
        TaskDefinition { cache: false, .. Default::default() }
    ; "both missing")]
    fn test_task_access_loading(
        trace_contents: Option<&str>,
        turbo_json_content: Option<&str>,
        expected_root_build: TaskDefinition,
    ) {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component("turbo.json");

        if let Some(content) = turbo_json_content {
            root_turbo_json
                .create_with_contents(content.as_bytes())
                .unwrap();
        }
        if let Some(content) = trace_contents {
            let trace_path = repo_root.join_components(&TASK_ACCESS_CONFIG_PATH);
            trace_path.ensure_dir().unwrap();
            trace_path.create_with_contents(content.as_bytes()).unwrap();
        }

        let mut scripts = BTreeMap::new();
        scripts.insert("build".into(), Spanned::new("echo building".into()));
        let root_package_json = PackageJson {
            scripts,
            ..Default::default()
        };

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader =
            UnifiedTurboJsonLoader::task_access(reader, root_turbo_json, root_package_json);
        let turbo_json = loader.load(&PackageName::Root).unwrap();
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        assert_eq!(
            expected_root_build,
            TaskDefinition::from_raw(root_build.clone(), RelativeUnixPath::new(".").unwrap())
                .unwrap()
        );
    }

    #[test]
    fn test_no_turbo_json_with_mfe() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create stub package info with scripts
        let root_pkg_json = PackageJson::default();
        let root_info = turborepo_repository::package_graph::PackageInfo {
            package_json: root_pkg_json,
            ..turborepo_repository::package_graph::PackageInfo::default()
        };

        let web_pkg_json = PackageJson {
            scripts: BTreeMap::from([
                ("dev".to_string(), Spanned::new("echo dev".to_string())),
                ("build".to_string(), Spanned::new("echo build".to_string())),
            ]),
            ..PackageJson::default()
        };
        let web_info = turborepo_repository::package_graph::PackageInfo {
            package_json: web_pkg_json,
            ..turborepo_repository::package_graph::PackageInfo::default()
        };

        let docs_pkg_json = PackageJson {
            scripts: BTreeMap::from([
                ("dev".to_string(), Spanned::new("echo dev".to_string())),
                ("build".to_string(), Spanned::new("echo build".to_string())),
            ]),
            ..PackageJson::default()
        };
        let docs_info = turborepo_repository::package_graph::PackageInfo {
            package_json: docs_pkg_json,
            ..turborepo_repository::package_graph::PackageInfo::default()
        };

        let microfrontends_configs = MicrofrontendsConfigs::from_configs(
            HashSet::from_iter(["web", "docs"].iter().copied()),
            vec![
                (
                    "web",
                    turborepo_microfrontends::TurborepoMfeConfig::from_str(
                        r#"{"version": "1", "applications": {"web": {}, "docs": {"routing": [{"paths": ["/docs"]}]}}}"#,
                        "mfe.json",
                    )
                    .map(Some),
                ),
                (
                    "docs",
                    Err(turborepo_microfrontends::Error::ChildConfig {
                        reference: "web".into(),
                    }),
                ),
            ]
            .into_iter(),
            {
                let mut deps = std::collections::HashMap::new();
                deps.insert("web", true);
                deps
            },
        )
        .unwrap();

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = UnifiedTurboJsonLoader::workspace_no_turbo_json(
            reader,
            vec![
                (&PackageName::Root, &root_info),
                (&PackageName::from("web"), &web_info),
                (&PackageName::from("docs"), &docs_info),
            ]
            .into_iter(),
            microfrontends_configs,
        );

        {
            let web_json = loader.load(&PackageName::from("web")).unwrap();
            for task_name in ["dev", "build", "proxy"] {
                if let Some(def) = web_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                    // Make sure proxy is in there
                    if task_name == "dev" {
                        assert_eq!(
                            def.with.as_ref().unwrap().first().unwrap().as_inner(),
                            &UnescapedString::from("web#proxy")
                        )
                    }
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }

        {
            let docs_json = loader.load(&PackageName::from("docs")).unwrap();
            for task_name in ["dev"] {
                if let Some(def) = docs_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                    assert_eq!(
                        def.with.as_ref().unwrap().first().unwrap().as_inner(),
                        &UnescapedString::from("web#proxy")
                    )
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }
    }
}
