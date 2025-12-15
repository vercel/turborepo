use std::collections::HashMap;

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_fixed_map::FixedMap;
use turborepo_repository::{
    package_graph::{PackageInfo, PackageName},
    package_json::PackageJson,
};
use turborepo_task_id::TaskName;

use super::{Pipeline, RawTaskDefinition, TurboJson};
use crate::{
    cli::EnvMode,
    config::{Error, CONFIG_FILE, CONFIG_FILE_JSONC},
    microfrontends::MicrofrontendsConfigs,
    run::task_access::TASK_ACCESS_CONFIG_PATH,
    turbo_json::FutureFlags,
};

/// Structure for loading TurboJson structures.
/// Depending on the strategy used, TurboJson might not correspond to
/// `turbo.json` file.
#[derive(Debug, Clone)]
pub struct TurboJsonLoader {
    reader: TurboJsonReader,
    cache: FixedMap<PackageName, TurboJson>,
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
        micro_frontends_configs: Option<MicrofrontendsConfigs>,
    },
    WorkspaceNoTurboJson {
        // Map of package names to their scripts
        packages: HashMap<PackageName, Vec<String>>,
        microfrontends_configs: Option<MicrofrontendsConfigs>,
    },
    TaskAccess {
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    },
    Noop,
}

// A helper structure configured with all settings related to reading a
// `turbo.json` file from disk.
#[derive(Debug, Clone)]
pub struct TurboJsonReader {
    repo_root: AbsoluteSystemPathBuf,
    future_flags: FutureFlags,
}

impl TurboJsonLoader {
    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        let repo_root = reader.repo_root();
        let packages = package_turbo_json_dirs(repo_root, root_turbo_json_path, packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: None,
            },
        }
    }

    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace_with_microfrontends<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        micro_frontends_configs: MicrofrontendsConfigs,
    ) -> Self {
        let repo_root = reader.repo_root();
        let packages = package_turbo_json_dirs(repo_root, root_turbo_json_path, packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: Some(micro_frontends_configs),
            },
        }
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s.
    pub fn workspace_no_turbo_json<'a>(
        reader: TurboJsonReader,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        microfrontends_configs: Option<MicrofrontendsConfigs>,
    ) -> Self {
        let packages = workspace_package_scripts(packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::WorkspaceNoTurboJson {
                packages,
                microfrontends_configs,
            },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            reader,
            cache: FixedMap::new(Some(PackageName::Root).into_iter()),
            strategy: Strategy::SinglePackage {
                root_turbo_json,
                package_json,
            },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn task_access(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            reader,
            cache: FixedMap::new(Some(PackageName::Root).into_iter()),
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
        let cache = FixedMap::from_iter(
            turbo_jsons
                .into_iter()
                .map(|(key, value)| (key, Some(value))),
        );
        // This never gets read from so we populate it with root
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) { "C:\\" } else { "/" })
            .expect("wasn't able to create absolute system path")
            .to_owned();
        Self {
            reader: TurboJsonReader::new(repo_root),
            cache,
            strategy: Strategy::Noop,
        }
    }

    /// Load a turbo.json for a given package
    pub fn load<'a>(&'a self, package: &PackageName) -> Result<&'a TurboJson, Error> {
        if let Ok(Some(turbo_json)) = self.cache.get(package) {
            return Ok(turbo_json);
        }
        let turbo_json = self.uncached_load(package)?;
        self.cache
            .insert(package, turbo_json)
            .map_err(|_| Error::NoTurboJSON)
    }

    fn uncached_load(&self, package: &PackageName) -> Result<TurboJson, Error> {
        let reader = &self.reader;
        match &self.strategy {
            Strategy::SinglePackage {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(Error::InvalidTurboJsonLoad(package.clone()))
                } else {
                    load_from_root_package_json(reader, root_turbo_json, package_json)
                }
            }
            Strategy::Workspace {
                packages,
                micro_frontends_configs,
            } => {
                let turbo_json_path = packages.get(package).ok_or_else(|| Error::NoTurboJSON)?;
                let is_root = package == &PackageName::Root;
                let turbo_json = load_from_file(
                    reader,
                    if is_root {
                        LoadTurboJsonPath::File(turbo_json_path)
                    } else {
                        LoadTurboJsonPath::Dir(turbo_json_path)
                    },
                    is_root,
                );
                if let Some(mfe_configs) = micro_frontends_configs {
                    mfe_configs.update_turbo_json(package, turbo_json)
                } else {
                    turbo_json
                }
            }
            Strategy::WorkspaceNoTurboJson {
                packages,
                microfrontends_configs,
            } => {
                let script_names = packages.get(package).ok_or(Error::NoTurboJSON)?;
                if matches!(package, PackageName::Root) {
                    root_turbo_json_from_scripts(script_names)
                } else {
                    let turbo_json = workspace_turbo_json_from_scripts(script_names);
                    if let Some(mfe_configs) = microfrontends_configs {
                        mfe_configs.update_turbo_json(package, turbo_json)
                    } else {
                        turbo_json
                    }
                }
            }
            Strategy::TaskAccess {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(Error::InvalidTurboJsonLoad(package.clone()))
                } else {
                    load_task_access_trace_turbo_json(reader, root_turbo_json, package_json)
                }
            }
            Strategy::Noop => Err(Error::NoTurboJSON),
        }
    }
}

/// Map all packages in the package graph to their dirs that contain a
/// turbo.json
fn package_turbo_json_dirs<'a>(
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
            Some((pkg.clone(), repo_root.resolve(info.package_path())))
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

enum LoadTurboJsonPath<'a> {
    // Look for a turbo.json in this directory
    Dir(&'a AbsoluteSystemPath),
    // Only use this path as a source for turbo.json
    // Does not need to have filename of turbo.json
    File(&'a AbsoluteSystemPath),
}

fn load_from_file(
    reader: &TurboJsonReader,
    turbo_json_path: LoadTurboJsonPath,
    is_root: bool,
) -> Result<TurboJson, Error> {
    let result = match turbo_json_path {
        LoadTurboJsonPath::Dir(turbo_json_dir_path) => {
            let turbo_json_path = turbo_json_dir_path.join_component(CONFIG_FILE);
            let turbo_jsonc_path = turbo_json_dir_path.join_component(CONFIG_FILE_JSONC);

            // Load both turbo.json and turbo.jsonc
            let turbo_json = reader.read(&turbo_json_path, is_root);
            let turbo_jsonc = reader.read(&turbo_jsonc_path, is_root);

            select_turbo_json(turbo_json_dir_path, turbo_json, turbo_jsonc)
        }
        LoadTurboJsonPath::File(turbo_json_path) => reader.read(turbo_json_path, is_root),
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

fn load_from_root_package_json(
    reader: &TurboJsonReader,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, Error> {
    let mut turbo_json = match reader.read(turbo_json_path, true) {
        // we're synthesizing, but we have a starting point
        // Note: this will have to change to support task inference in a monorepo
        // for now, we're going to error on any "root" tasks and turn non-root tasks into root
        // tasks
        Ok(Some(mut turbo_json)) => {
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
        Ok(None) => TurboJson::default(),
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
                env_mode: Some(Spanned::new(EnvMode::Loose)),
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
                env_mode: Some(Spanned::new(EnvMode::Loose)),
                ..Default::default()
            }),
        );
    }
    Ok(turbo_json)
}

fn load_task_access_trace_turbo_json(
    reader: &TurboJsonReader,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, Error> {
    let trace_json_path = reader.repo_root().join_components(&TASK_ACCESS_CONFIG_PATH);
    let turbo_from_trace = reader.read(&trace_json_path, true);

    // check the zero config case (turbo trace file, but no turbo.json file)
    if let Ok(Some(turbo_from_trace)) = turbo_from_trace {
        if !turbo_json_path.exists() {
            debug!("Using turbo.json synthesized from trace file");
            return Ok(turbo_from_trace);
        }
    }
    load_from_root_package_json(reader, turbo_json_path, root_package_json)
}

// Helper for selecting the correct turbo.json read result
fn select_turbo_json(
    turbo_json_dir_path: &AbsoluteSystemPath,
    turbo_json: Result<Option<TurboJson>, Error>,
    turbo_jsonc: Result<Option<TurboJson>, Error>,
) -> Result<Option<TurboJson>, Error> {
    debug!(
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

impl TurboJsonReader {
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            future_flags: Default::default(),
        }
    }

    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.future_flags = future_flags;
        self
    }

    pub fn read(
        &self,
        path: &AbsoluteSystemPath,
        is_root: bool,
    ) -> Result<Option<TurboJson>, Error> {
        TurboJson::read(&self.repo_root, path, is_root, self.future_flags)
    }

    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::{BTreeMap, HashSet},
        fs,
    };

    use anyhow::Result;
    use insta::assert_snapshot;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::RelativeUnixPath;
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::{config::Error, task_graph::TaskDefinition};

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
        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(Some(PackageName::Root).into_iter()),
            strategy: Strategy::Workspace {
                packages: vec![(PackageName::Root, root_turbo_json.to_owned())]
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

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::single_package(reader, root_turbo_json, root_package_json);
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

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::task_access(reader, root_turbo_json, root_package_json);
        let turbo_json = loader.load(&PackageName::Root)?;
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        assert_eq!(
            expected_root_build,
            TaskDefinition::from_raw(root_build.clone(), RelativeUnixPath::new(".").unwrap())?
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
        let reader = TurboJsonReader::new(junk_path.to_owned());
        let single_loader = TurboJsonLoader::single_package(
            reader.clone(),
            junk_path.to_owned(),
            PackageJson::default(),
        );
        let task_access_loader =
            TurboJsonLoader::task_access(reader, junk_path.to_owned(), PackageJson::default());

        for loader in [single_loader, task_access_loader] {
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
        let packages = vec![(
            PackageName::from("a"),
            a_turbo_json.parent().unwrap().to_owned(),
        )]
        .into_iter()
        .collect();

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(vec![PackageName::Root, PackageName::from("a")].into_iter()),
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
        let packages = vec![(
            PackageName::from("a"),
            a_turbo_json.parent().unwrap().to_owned(),
        )]
        .into_iter()
        .collect();

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(vec![PackageName::Root, PackageName::from("a")].into_iter()),
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

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(vec![PackageName::Root, PackageName::from("pkg-a")].into_iter()),
            strategy: Strategy::WorkspaceNoTurboJson {
                packages,
                microfrontends_configs: None,
            },
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

    #[test]
    fn test_no_turbo_json_with_mfe() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let packages = vec![
            (PackageName::Root, vec![]),
            (
                PackageName::from("web"),
                vec!["dev".to_owned(), "build".to_owned()],
            ),
            (
                PackageName::from("docs"),
                vec!["dev".to_owned(), "build".to_owned()],
            ),
        ]
        .into_iter()
        .collect();

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
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(
                vec![
                    PackageName::Root,
                    PackageName::from("web"),
                    PackageName::from("docs"),
                ]
                .into_iter(),
            ),
            strategy: Strategy::WorkspaceNoTurboJson {
                packages,
                microfrontends_configs,
            },
        };

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

    #[test]
    fn test_load_from_file_with_both_files() -> Result<()> {
        let tmp_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path())?;
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create both turbo.json and turbo.jsonc
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

        turbo_json_path.create_with_contents("{}")?;
        turbo_jsonc_path.create_with_contents("{}")?;

        // Test load_from_file with turbo.json path
        let result = load_from_file(&reader, LoadTurboJsonPath::Dir(repo_root), true);

        // The function should return an error when both files exist
        assert!(result.is_err());
        let mut err = result.unwrap_err();
        // Override tmpdir so we can snapshot the error message
        if let Error::MultipleTurboConfigs { directory } = &mut err {
            *directory = "some-dir".to_owned()
        }
        assert_snapshot!(err, @r"
        Found both turbo.json and turbo.jsonc in the same directory: some-dir
        Remove either turbo.json or turbo.jsonc so there is only one.
        ");

        Ok(())
    }

    #[test]
    fn test_load_from_file_with_only_turbo_json() -> Result<()> {
        let tmp_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path())?;
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.json
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        turbo_json_path.create_with_contents("{}")?;

        // Test load_from_file
        let result = load_from_file(&reader, LoadTurboJsonPath::Dir(repo_root), true);

        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_load_from_file_with_only_turbo_jsonc() -> Result<()> {
        let tmp_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path())?;
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.jsonc
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);
        turbo_jsonc_path.create_with_contents("{}")?;

        // Test load_from_file
        let result = load_from_file(&reader, LoadTurboJsonPath::Dir(repo_root), true);

        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_context_aware_parsing() {
        // Test that the reader correctly determines root vs package contexts
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create a root turbo.json with root-only fields
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(
                r#"{
            "globalEnv": ["NODE_ENV"],
            "tasks": {"build": {}}
        }"#,
            )
            .unwrap();

        // Create a package turbo.json with extends
        let pkg_dir = repo_root.join_components(&["packages", "foo"]);
        pkg_dir.create_dir_all().unwrap();
        let pkg_turbo_json = pkg_dir.join_component("turbo.json");
        pkg_turbo_json
            .create_with_contents(
                r#"{
            "extends": ["//"],
            "tasks": {"test": {}}
        }"#,
            )
            .unwrap();

        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Reading root turbo.json should work with globalEnv
        let root_result = reader.read(&root_turbo_json, true);
        assert!(root_result.is_ok());
        let root_json = root_result.unwrap().unwrap();
        assert!(!root_json.global_env.is_empty());

        // Reading package turbo.json should work with extends
        let pkg_result = reader.read(&pkg_turbo_json, false);
        assert!(pkg_result.is_ok());
        let pkg_json = pkg_result.unwrap().unwrap();
        assert!(!pkg_json.extends.is_empty());

        // Now test invalid cases
        // Root turbo.json with extends should fail
        root_turbo_json
            .create_with_contents(
                r#"{
            "extends": ["//"],
            "tasks": {"build": {}}
        }"#,
            )
            .unwrap();
        let invalid_root = reader.read(&root_turbo_json, true);
        assert!(invalid_root.is_err());

        // Package turbo.json with globalEnv should fail
        pkg_turbo_json
            .create_with_contents(
                r#"{
            "globalEnv": ["NODE_ENV"],
            "tasks": {"test": {}}
        }"#,
            )
            .unwrap();
        let invalid_pkg = reader.read(&pkg_turbo_json, false);
        assert!(invalid_pkg.is_err());
    }

    #[test]
    fn test_invalid_workspace_turbo_json() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let a_turbo_json = repo_root.join_components(&["packages", "a", "turbo.json"]);
        a_turbo_json.ensure_dir().unwrap();
        let packages = vec![(
            PackageName::from("a"),
            a_turbo_json.parent().unwrap().to_owned(),
        )]
        .into_iter()
        .collect();

        a_turbo_json
            .create_with_contents(r#"{"tasks": {"build": {"lol": true}}}"#)
            .unwrap();

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader {
            reader,
            cache: FixedMap::new(vec![PackageName::Root, PackageName::from("a")].into_iter()),
            strategy: Strategy::Workspace {
                packages,
                micro_frontends_configs: None,
            },
        };
        let result = loader.load(&PackageName::from("a"));
        assert!(
            matches!(result.unwrap_err(), Error::TurboJsonParseError(_)),
            "expected parsing to fail due to unknown key"
        );
    }
}
