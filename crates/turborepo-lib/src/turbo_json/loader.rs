use std::collections::HashMap;

use itertools::Itertools;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_micro_frontend::MICRO_FRONTENDS_PACKAGES;
use turborepo_repository::{
    package_graph::{PackageInfo, PackageName},
    package_json::PackageJson,
};

use super::{Pipeline, RawTaskDefinition, TurboJson, CONFIG_FILE};
use crate::{
    cli::EnvMode,
    config::Error,
    micro_frontends::MicroFrontendsConfigs,
    run::{task_access::TASK_ACCESS_CONFIG_PATH, task_id::TaskName},
};

/// Structure for loading TurboJson structures.
/// Depending on the strategy used, TurboJson might not correspond to
/// `turbo.json` file.
#[derive(Debug, Clone)]
pub struct TurboJsonLoader {
    repo_root: AbsoluteSystemPathBuf,
    cache: HashMap<PackageName, TurboJson>,
    strategy: Strategy,
}

#[derive(Debug, Clone)]
enum Strategy {
    SinglePackage {
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    },
    Workspace {
        // Map of package names to their package specific turbo.json
        packages: HashMap<PackageName, AbsoluteSystemPathBuf>,
        micro_frontends_configs: Option<MicroFrontendsConfigs>,
    },
    WorkspaceNoTurboJson {
        // Map of package names to their scripts
        packages: HashMap<PackageName, Vec<String>>,
    },
    TaskAccess {
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    },
    Noop,
}

impl TurboJsonLoader {
    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace<'a>(
        repo_root: AbsoluteSystemPathBuf,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        let packages = package_turbo_jsons(&repo_root, root_turbo_json_path, packages);
        Self {
            repo_root,
            cache: HashMap::new(),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: None,
            },
        }
    }

    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace_with_microfrontends<'a>(
        repo_root: AbsoluteSystemPathBuf,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        micro_frontends_configs: MicroFrontendsConfigs,
    ) -> Self {
        let packages = package_turbo_jsons(&repo_root, root_turbo_json_path, packages);
        Self {
            repo_root,
            cache: HashMap::new(),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: Some(micro_frontends_configs),
            },
        }
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s.
    pub fn workspace_no_turbo_json<'a>(
        repo_root: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        let packages = workspace_package_scripts(packages);
        Self {
            repo_root,
            cache: HashMap::new(),
            strategy: Strategy::WorkspaceNoTurboJson { packages },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        repo_root: AbsoluteSystemPathBuf,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            repo_root,
            cache: HashMap::new(),
            strategy: Strategy::SinglePackage {
                root_turbo_json,
                package_json,
            },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn task_access(
        repo_root: AbsoluteSystemPathBuf,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            repo_root,
            cache: HashMap::new(),
            strategy: Strategy::TaskAccess {
                root_turbo_json,
                package_json,
            },
        }
    }

    /// Create a loader that will only return provided turbo.jsons and will
    /// never hit the file system.
    /// Primarily intended for testing
    pub fn noop(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        Self {
            // This never gets read from so we populate it with
            repo_root: AbsoluteSystemPath::new(if cfg!(windows) { "C:\\" } else { "/" })
                .expect("wasn't able to create absolute system path")
                .to_owned(),
            cache: turbo_jsons,
            strategy: Strategy::Noop,
        }
    }

    /// Load a turbo.json for a given package
    pub fn load<'a>(&'a mut self, package: &PackageName) -> Result<&'a TurboJson, Error> {
        if !self.cache.contains_key(package) {
            let turbo_json = self.uncached_load(package)?;
            self.cache.insert(package.clone(), turbo_json);
        }
        Ok(self
            .cache
            .get(package)
            .expect("just inserted value for this key"))
    }

    fn uncached_load(&self, package: &PackageName) -> Result<TurboJson, Error> {
        match &self.strategy {
            Strategy::SinglePackage {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(Error::InvalidTurboJsonLoad(package.clone()))
                } else {
                    load_from_root_package_json(&self.repo_root, root_turbo_json, package_json)
                }
            }
            Strategy::Workspace {
                packages,
                micro_frontends_configs,
            } => {
                let path = packages.get(package).ok_or_else(|| Error::NoTurboJSON)?;
                let should_inject_proxy_task = micro_frontends_configs
                    .as_ref()
                    .map_or(false, |configs| configs.contains_package(package.as_str()));
                let turbo_json = load_from_file(&self.repo_root, path);
                if should_inject_proxy_task {
                    let mut turbo_json = turbo_json.or_else(|err| match err {
                        Error::NoTurboJSON => Ok(TurboJson::default()),
                        err => Err(err),
                    })?;
                    let mfe_package_name = packages
                        .keys()
                        .filter(|package| MICRO_FRONTENDS_PACKAGES.contains(&package.as_str()))
                        .map(|package| package.as_str())
                        // If multiple MFE packages are present in the graph, then be deterministic
                        // in which ones we select
                        .sorted()
                        .next();
                    turbo_json.with_proxy(mfe_package_name);
                    Ok(turbo_json)
                } else {
                    turbo_json
                }
            }
            Strategy::WorkspaceNoTurboJson { packages } => {
                let script_names = packages.get(package).ok_or(Error::NoTurboJSON)?;
                if matches!(package, PackageName::Root) {
                    root_turbo_json_from_scripts(script_names)
                } else {
                    workspace_turbo_json_from_scripts(script_names)
                }
            }
            Strategy::TaskAccess {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(Error::InvalidTurboJsonLoad(package.clone()))
                } else {
                    load_task_access_trace_turbo_json(
                        &self.repo_root,
                        root_turbo_json,
                        package_json,
                    )
                }
            }
            Strategy::Noop => Err(Error::NoTurboJSON),
        }
    }
}

/// Map all packages in the package graph to their turbo.json path
fn package_turbo_jsons<'a>(
    repo_root: &AbsoluteSystemPath,
    root_turbo_json_path: AbsoluteSystemPathBuf,
    packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
) -> HashMap<PackageName, AbsoluteSystemPathBuf> {
    let mut package_turbo_jsons = HashMap::new();
    package_turbo_jsons.insert(PackageName::Root, root_turbo_json_path);
    package_turbo_jsons.extend(packages.filter_map(|(pkg, info)| {
        if pkg == &PackageName::Root {
            None
        } else {
            Some((
                pkg.clone(),
                repo_root
                    .resolve(info.package_path())
                    .join_component(CONFIG_FILE),
            ))
        }
    }));
    package_turbo_jsons
}

/// Map all packages in the package graph to their scripts
fn workspace_package_scripts<'a>(
    packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
) -> HashMap<PackageName, Vec<String>> {
    packages
        .map(|(pkg, info)| {
            (
                pkg.clone(),
                info.package_json.scripts.keys().cloned().collect(),
            )
        })
        .collect()
}

fn load_from_file(
    repo_root: &AbsoluteSystemPath,
    turbo_json_path: &AbsoluteSystemPath,
) -> Result<TurboJson, Error> {
    match TurboJson::read(repo_root, turbo_json_path) {
        // If the file didn't exist, throw a custom error here instead of propagating
        Err(Error::Io(_)) => Err(Error::NoTurboJSON),
        // There was an error, and we don't have any chance of recovering
        // because we aren't synthesizing anything
        Err(e) => Err(e),
        // We're not synthesizing anything and there was no error, we're done
        Ok(turbo) => Ok(turbo),
    }
}

fn load_from_root_package_json(
    repo_root: &AbsoluteSystemPath,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, Error> {
    let mut turbo_json = match TurboJson::read(repo_root, turbo_json_path) {
        // we're synthesizing, but we have a starting point
        // Note: this will have to change to support task inference in a monorepo
        // for now, we're going to error on any "root" tasks and turn non-root tasks into root
        // tasks
        Ok(mut turbo_json) => {
            let mut pipeline = Pipeline::default();
            for (task_name, task_definition) in turbo_json.tasks {
                if task_name.is_package_task() {
                    let (span, text) = task_definition.span_and_text("turbo.json");

                    return Err(Error::PackageTaskInSinglePackageMode {
                        task_id: task_name.to_string(),
                        span,
                        text,
                    });
                }

                pipeline.insert(task_name.into_root_task(), task_definition);
            }

            turbo_json.tasks = pipeline;

            turbo_json
        }
        // turbo.json doesn't exist, but we're going try to synthesize something
        Err(Error::Io(_)) => TurboJson::default(),
        // some other happened, we can't recover
        Err(e) => {
            return Err(e);
        }
    };

    // TODO: Add location info from package.json
    for script_name in root_package_json.scripts.keys() {
        let task_name = TaskName::from(script_name.as_str());
        if !turbo_json.has_task(&task_name) {
            let task_name = task_name.into_root_task();
            // Explicitly set cache to Some(false) in this definition
            // so we can pretend it was set on purpose. That way it
            // won't get clobbered by the merge function.
            turbo_json.tasks.insert(
                task_name,
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                }),
            );
        }
    }

    Ok(turbo_json)
}

fn root_turbo_json_from_scripts(scripts: &[String]) -> Result<TurboJson, Error> {
    let mut turbo_json = TurboJson {
        ..Default::default()
    };
    for script in scripts {
        let task_name = TaskName::from(script.as_str()).into_root_task();
        turbo_json.tasks.insert(
            task_name,
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                env_mode: Some(EnvMode::Loose),
                ..Default::default()
            }),
        );
    }
    Ok(turbo_json)
}

fn workspace_turbo_json_from_scripts(scripts: &[String]) -> Result<TurboJson, Error> {
    let mut turbo_json = TurboJson {
        extends: Spanned::new(vec!["//".to_owned()]),
        ..Default::default()
    };
    for script in scripts {
        let task_name = TaskName::from(script.clone());
        turbo_json.tasks.insert(
            task_name,
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                env_mode: Some(EnvMode::Loose),
                ..Default::default()
            }),
        );
    }
    Ok(turbo_json)
}

fn load_task_access_trace_turbo_json(
    repo_root: &AbsoluteSystemPath,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, Error> {
    let trace_json_path = repo_root.join_components(&TASK_ACCESS_CONFIG_PATH);
    let turbo_from_trace = TurboJson::read(repo_root, &trace_json_path);

    // check the zero config case (turbo trace file, but no turbo.json file)
    if let Ok(turbo_from_trace) = turbo_from_trace {
        if !turbo_json_path.exists() {
            debug!("Using turbo.json synthesized from trace file");
            return Ok(turbo_from_trace);
        }
    }
    load_from_root_package_json(repo_root, turbo_json_path, root_package_json)
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeMap, fs};

    use anyhow::Result;
    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;
    use crate::{task_graph::TaskDefinition, turbo_json::CONFIG_FILE};

    #[test_case(r"{}", TurboJson::default() ; "empty")]
    #[test_case(r#"{ "globalDependencies": ["tsconfig.json", "jest.config.ts"] }"#,
        TurboJson {
            global_deps: vec!["jest.config.ts".to_string(), "tsconfig.json".to_string()],
            ..TurboJson::default()
        }
    ; "global dependencies (sorted)")]
    #[test_case(r#"{ "globalPassThroughEnv": ["GITHUB_TOKEN", "AWS_SECRET_KEY"] }"#,
        TurboJson {
            global_pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string(), "GITHUB_TOKEN".to_string()]),
            ..TurboJson::default()
        }
    )]
    #[test_case(r#"{ "//": "A comment"}"#, TurboJson::default() ; "faux comment")]
    #[test_case(r#"{ "//": "A comment", "//": "Another comment" }"#, TurboJson::default() ; "two faux comments")]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: TurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        let root_turbo_json = repo_root.join_component("turbo.json");
        fs::write(&root_turbo_json, turbo_json_content)?;
        let mut loader = TurboJsonLoader {
            repo_root: repo_root.to_owned(),
            cache: HashMap::new(),
            strategy: Strategy::Workspace {
                packages: vec![(PackageName::Root, root_turbo_json)]
                    .into_iter()
                    .collect(),
                micro_frontends_configs: None,
            },
        };

        let mut turbo_json = loader.load(&PackageName::Root)?.clone();

        turbo_json.text = None;
        turbo_json.path = None;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        None,
        PackageJson {
             scripts: [("build".to_string(), Spanned::new("echo build".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        TurboJson {
            tasks: Pipeline([(
                "//#build".into(),
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                })
              )].into_iter().collect()
            ),
            ..TurboJson::default()
        }
    )]
    #[test_case(
        Some(r#"{
            "tasks": {
                "build": {
                    "cache": true
                }
            }
        }"#),
        PackageJson {
             scripts: [("test".to_string(), Spanned::new("echo test".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        TurboJson {
            tasks: Pipeline([(
                "//#build".into(),
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(true).with_range(81..85)),
                    ..RawTaskDefinition::default()
                }).with_range(50..103)
            ),
            (
                "//#test".into(),
                Spanned::new(RawTaskDefinition {
                     cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                })
            )].into_iter().collect()),
            ..TurboJson::default()
        }
    )]
    fn test_get_root_turbo_with_synthesizing(
        turbo_json_content: Option<&str>,
        root_package_json: PackageJson,
        expected_turbo_json: TurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        let root_turbo_json = repo_root.join_component(CONFIG_FILE);

        if let Some(content) = turbo_json_content {
            fs::write(&root_turbo_json, content)?;
        }

        let mut loader = TurboJsonLoader::single_package(
            repo_root.to_owned(),
            root_turbo_json,
            root_package_json,
        );
        let mut turbo_json = loader.load(&PackageName::Root)?.clone();
        turbo_json.text = None;
        turbo_json.path = None;
        for (_, task_definition) in turbo_json.tasks.iter_mut() {
            task_definition.path = None;
            task_definition.text = None;
        }
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

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
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        let root_turbo_json = repo_root.join_component(CONFIG_FILE);

        if let Some(content) = turbo_json_content {
            root_turbo_json.create_with_contents(content.as_bytes())?;
        }
        if let Some(content) = trace_contents {
            let trace_path = repo_root.join_components(&TASK_ACCESS_CONFIG_PATH);
            trace_path.ensure_dir()?;
            trace_path.create_with_contents(content.as_bytes())?;
        }

        let mut scripts = BTreeMap::new();
        scripts.insert("build".into(), Spanned::new("echo building".into()));
        let root_package_json = PackageJson {
            scripts,
            ..Default::default()
        };

        let mut loader =
            TurboJsonLoader::task_access(repo_root.to_owned(), root_turbo_json, root_package_json);
        let turbo_json = loader.load(&PackageName::Root)?;
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        assert_eq!(
            expected_root_build,
            TaskDefinition::try_from(root_build.clone())?
        );

        Ok(())
    }

    #[test]
    fn test_single_package_loading_non_root() {
        let junk_path = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\never\\loaded"
        } else {
            "/never/loaded"
        })
        .unwrap();
        let non_root = PackageName::from("some-pkg");
        let single_loader = TurboJsonLoader::single_package(
            junk_path.to_owned(),
            junk_path.to_owned(),
            PackageJson::default(),
        );
        let task_access_loader = TurboJsonLoader::task_access(
            junk_path.to_owned(),
            junk_path.to_owned(),
            PackageJson::default(),
        );

        for mut loader in [single_loader, task_access_loader] {
            let result = loader.load(&non_root);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                matches!(err, Error::InvalidTurboJsonLoad(_)),
                "expected {err} to be no turbo json"
            );
        }
    }

    #[test]
    fn test_workspace_turbo_json_loading() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let a_turbo_json = repo_root.join_components(&["packages", "a", "turbo.json"]);
        a_turbo_json.ensure_dir().unwrap();
        let packages = vec![(PackageName::from("a"), a_turbo_json.clone())]
            .into_iter()
            .collect();

        let mut loader = TurboJsonLoader {
            repo_root: repo_root.to_owned(),
            cache: HashMap::new(),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: None,
            },
        };
        let result = loader.load(&PackageName::from("a"));
        assert!(
            matches!(result.unwrap_err(), Error::NoTurboJSON),
            "expected parsing to fail with missing turbo.json"
        );

        a_turbo_json
            .create_with_contents(r#"{"tasks": {"build": {}}}"#)
            .unwrap();

        let turbo_json = loader.load(&PackageName::from("a")).unwrap();
        assert_eq!(turbo_json.tasks.len(), 1);
    }

    #[test]
    fn test_turbo_json_caching() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let a_turbo_json = repo_root.join_components(&["packages", "a", "turbo.json"]);
        a_turbo_json.ensure_dir().unwrap();
        let packages = vec![(PackageName::from("a"), a_turbo_json.clone())]
            .into_iter()
            .collect();

        let mut loader = TurboJsonLoader {
            repo_root: repo_root.to_owned(),
            cache: HashMap::new(),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: None,
            },
        };
        a_turbo_json
            .create_with_contents(r#"{"tasks": {"build": {}}}"#)
            .unwrap();

        let turbo_json = loader.load(&PackageName::from("a")).unwrap();
        assert_eq!(turbo_json.tasks.len(), 1);
        a_turbo_json.remove().unwrap();
        assert!(loader.load(&PackageName::from("a")).is_ok());
    }

    #[test]
    fn test_no_turbo_json() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let packages = vec![
            (
                PackageName::Root,
                vec!["build".to_owned(), "lint".to_owned(), "test".to_owned()],
            ),
            (
                PackageName::from("pkg-a"),
                vec!["build".to_owned(), "lint".to_owned(), "special".to_owned()],
            ),
        ]
        .into_iter()
        .collect();

        let mut loader = TurboJsonLoader {
            repo_root: repo_root.to_owned(),
            cache: HashMap::new(),
            strategy: Strategy::WorkspaceNoTurboJson { packages },
        };

        {
            let root_json = loader.load(&PackageName::Root).unwrap();
            for task_name in ["//#build", "//#lint", "//#test"] {
                if let Some(def) = root_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }

        {
            let pkg_a_json = loader.load(&PackageName::from("pkg-a")).unwrap();
            for task_name in ["build", "lint", "special"] {
                if let Some(def) = pkg_a_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }
        // Should get no turbo.json error if package wasn't declared
        let goose_err = loader.load(&PackageName::from("goose")).unwrap_err();
        assert!(matches!(goose_err, Error::NoTurboJSON));
    }
}
