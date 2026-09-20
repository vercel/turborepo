use std::collections::HashMap;

use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_repository::package_graph::PackageName;
use turborepo_turbo_json::{LoaderError, NoOpUpdater, TurboJson, TurboJsonLoader, TurboJsonReader};

use crate::MicrofrontendsConfigs;

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
        package_directories: impl Iterator<Item = (PackageName, &'a AnchoredSystemPath)>,
    ) -> Self {
        Self::Standard(TurboJsonLoader::workspace(
            reader,
            root_turbo_json_path,
            package_directories,
        ))
    }

    /// Create a loader that will load turbo.json files throughout the workspace
    /// with microfrontends support.
    pub fn workspace_with_microfrontends<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        package_directories: impl Iterator<Item = (PackageName, &'a AnchoredSystemPath)>,
        micro_frontends_configs: MicrofrontendsConfigs,
    ) -> Self {
        Self::WithMfe(TurboJsonLoader::workspace_with_updater(
            reader,
            root_turbo_json_path,
            package_directories,
            micro_frontends_configs,
        ))
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s, with optional microfrontends support.
    pub fn workspace_no_turbo_json<'a>(
        reader: TurboJsonReader,
        package_directories: impl Iterator<Item = (PackageName, &'a AnchoredSystemPath)>,
        package_scripts: HashMap<PackageName, Vec<String>>,
        microfrontends_configs: Option<MicrofrontendsConfigs>,
    ) -> Self {
        if let Some(mfe) = microfrontends_configs {
            Self::WithMfe(TurboJsonLoader::workspace_no_turbo_json_with_updater(
                reader,
                package_directories,
                package_scripts,
                Some(mfe),
            ))
        } else {
            Self::Standard(TurboJsonLoader::workspace_no_turbo_json(
                reader,
                package_directories,
                package_scripts,
            ))
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        root_scripts: Vec<String>,
    ) -> Self {
        Self::Standard(TurboJsonLoader::single_package(
            reader,
            root_turbo_json,
            root_scripts,
        ))
    }

    /// Create a loader for task access tracing
    pub fn task_access(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        root_scripts: Vec<String>,
    ) -> Self {
        Self::Standard(TurboJsonLoader::task_access(
            reader,
            root_turbo_json,
            root_scripts,
        ))
    }

    /// Create a loader that will only return provided turbo.jsons and will
    /// never hit the file system.
    /// Primarily intended for testing
    pub fn noop(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        Self::Standard(TurboJsonLoader::noop(turbo_jsons))
    }

    /// Load a turbo.json for a given package
    pub fn load(&self, package: &PackageName) -> Result<&TurboJson, LoaderError> {
        match self {
            Self::Standard(loader) => loader.load(package),
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

    /// Pre-warm the cache by loading the given packages' turbo.json files in
    /// parallel. Errors are silently ignored — subsequent `load()` calls will
    /// report them.
    pub fn preload_packages(&self, packages: impl IntoIterator<Item = PackageName>) {
        match self {
            Self::Standard(loader) => loader.preload_packages(packages),
            Self::WithMfe(loader) => loader.preload_packages(packages),
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};
    use turborepo_task_id::TaskName;
    use turborepo_turbo_json::TASK_ACCESS_CONFIG_PATH;
    use turborepo_unescape::UnescapedString;

    use super::*;

    #[test_case(
        Some(r#"{ "tasks": {"//#build": {"env": ["SPECIAL_VAR"]}} }"#),
        Some(r#"{ "tasks": {"build": {"env": ["EXPLICIT_VAR"]}} }"#),
        Some("EXPLICIT_VAR"),
        None
    ; "both present")]
    #[test_case(
        None,
        Some(r#"{ "tasks": {"build": {"env": ["EXPLICIT_VAR"]}} }"#),
        Some("EXPLICIT_VAR"),
        None
    ; "no trace")]
    #[test_case(
        Some(r#"{ "tasks": {"//#build": {"env": ["SPECIAL_VAR"]}} }"#),
        None,
        Some("SPECIAL_VAR"),
        None
    ; "no turbo.json")]
    #[test_case(None, None, None, Some(false); "both missing")]
    fn test_task_access_loading(
        trace_contents: Option<&str>,
        turbo_json_content: Option<&str>,
        expected_env: Option<&str>,
        expected_cache: Option<bool>,
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

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader =
            UnifiedTurboJsonLoader::task_access(reader, root_turbo_json, vec!["build".into()]);
        let turbo_json = loader.load(&PackageName::Root).unwrap();
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        let actual_env = root_build.env.as_ref().map(|env| {
            env.iter()
                .map(|value| value.as_inner().to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(actual_env, expected_env.map(|env| vec![env.to_owned()]));
        assert_eq!(
            root_build.cache.as_ref().map(|cache| *cache.as_inner()),
            expected_cache
        );
    }

    #[test]
    fn test_no_turbo_json_with_mfe() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        let package_scripts = HashMap::from([
            (PackageName::Root, Vec::new()),
            (
                PackageName::from("web"),
                vec!["dev".to_owned(), "build".to_owned()],
            ),
            (
                PackageName::from("docs"),
                vec!["dev".to_owned(), "build".to_owned()],
            ),
        ]);
        let web_dir = AnchoredSystemPath::new("apps/web").unwrap();
        let docs_dir = AnchoredSystemPath::new("apps/docs").unwrap();

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
                (PackageName::from("web"), web_dir),
                (PackageName::from("docs"), docs_dir),
            ]
            .into_iter(),
            package_scripts,
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
                    if task_name == "dev" {
                        assert!(
                            def.with
                                .as_ref()
                                .unwrap()
                                .iter()
                                .any(|t| { t.as_inner() == &UnescapedString::from("web#proxy") })
                        );
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
                    assert!(
                        def.with
                            .as_ref()
                            .unwrap()
                            .iter()
                            .any(|t| { t.as_inner() == &UnescapedString::from("web#proxy") })
                    );
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }
    }
}
