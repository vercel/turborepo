use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use tracing::warn;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath, RelativeUnixPathBuf};
use turborepo_microfrontends::{Error, TurborepoMfeConfig as MfeConfig, MICROFRONTENDS_PACKAGE};
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::{TaskId, TaskName};

use crate::{config, turbo_json::TurboJson};

#[derive(Debug, Clone)]
pub struct MicrofrontendsConfigs {
    configs: HashMap<String, ConfigInfo>,
    mfe_package: Option<&'static str>,
    has_mfe_dependency: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ConfigInfo {
    // A map from tasks declared in the configuration to the application that they belong to
    tasks: HashMap<TaskId<'static>, String>,
    ports: HashMap<TaskId<'static>, u16>,
    version: &'static str,
    path: Option<RelativeUnixPathBuf>,
    // Whether to use the Turborepo proxy (true) or create a proxy task (false)
    use_turborepo_proxy: bool,
}

impl MicrofrontendsConfigs {
    /// Constructs a collection of configurations from disk
    pub fn from_disk(
        repo_root: &AbsoluteSystemPath,
        package_graph: &PackageGraph,
    ) -> Result<Option<Self>, Error> {
        tracing::debug!("MicrofrontendsConfigs::from_disk - loading configurations");

        struct PackageMetadata<'a> {
            names: HashSet<&'a str>,
            has_mfe_dep: HashMap<&'a str, bool>,
            configs: Vec<(&'a str, Result<Option<MfeConfig>, Error>)>,
        }

        let metadata = package_graph.packages().fold(
            PackageMetadata {
                names: HashSet::new(),
                has_mfe_dep: HashMap::new(),
                configs: Vec::new(),
            },
            |mut acc, (name, info)| {
                let name_str = name.as_str();
                acc.names.insert(name_str);
                let has_dep = info
                    .package_json
                    .all_dependencies()
                    .any(|(dep, _)| dep.as_str() == MICROFRONTENDS_PACKAGE);
                tracing::debug!(
                    "from_disk - package: {}, has @vercel/microfrontends dep: {}",
                    name_str,
                    has_dep
                );
                acc.has_mfe_dep.insert(name_str, has_dep);

                let config_result =
                    MfeConfig::load_from_dir_with_mfe_dep(repo_root, info.package_path(), has_dep);
                if let Ok(Some(ref _config)) = config_result {
                    tracing::debug!(
                        "from_disk - found config in package: {}, path: {:?}",
                        name_str,
                        info.package_path()
                    );
                } else if let Err(ref e) = config_result {
                    tracing::debug!(
                        "from_disk - error loading config from package {}: {}",
                        name_str,
                        e
                    );
                }
                acc.configs.push((name_str, config_result));
                acc
            },
        );

        tracing::debug!(
            "from_disk - loaded {} package configs",
            metadata.configs.len()
        );

        Self::from_configs(
            metadata.names,
            metadata.configs.into_iter(),
            metadata.has_mfe_dep,
        )
    }

    /// Constructs a collection of configurations from a list of configurations
    pub fn from_configs<'a>(
        package_names: HashSet<&str>,
        configs: impl Iterator<Item = (&'a str, Result<Option<MfeConfig>, Error>)>,
        package_has_mfe_dependency: HashMap<&str, bool>,
    ) -> Result<Option<Self>, Error> {
        tracing::debug!("from_configs - processing configurations");
        let PackageGraphResult {
            configs,
            missing_default_apps,
            missing_applications,
            unsupported_version: _,
            mfe_package,
            has_mfe_dependency,
        } = PackageGraphResult::new(package_names, configs, package_has_mfe_dependency)?;

        tracing::debug!(
            "from_configs - result: {} configs, mfe_package={:?}, has_mfe_dependency={}",
            configs.len(),
            mfe_package,
            has_mfe_dependency
        );

        if !missing_default_apps.is_empty() {
            warn!(
                "Missing default applications: {}",
                missing_default_apps.join(", ")
            );
        }

        if !missing_applications.is_empty() {
            warn!(
                "Unable to find packages referenced in 'microfrontends.json' in workspace. Local \
                 proxy will not route to the following applications if they are running locally: \
                 {}",
                missing_applications.join(", ")
            );
        }

        if configs.is_empty() {
            tracing::debug!("from_configs - no configs found, returning None");
        } else {
            tracing::debug!(
                "from_configs - returning MicrofrontendsConfigs with packages: {:?}",
                configs.keys().collect::<Vec<_>>()
            );
        }

        Ok((!configs.is_empty()).then_some(Self {
            configs,
            mfe_package,
            has_mfe_dependency,
        }))
    }

    pub fn configs(&self) -> impl Iterator<Item = (&String, &HashMap<TaskId<'static>, String>)> {
        self.configs.iter().map(|(pkg, info)| (pkg, &info.tasks))
    }

    pub fn get(&self, package_name: &str) -> Option<&HashMap<TaskId<'static>, String>> {
        let info = self.configs.get(package_name)?;
        Some(&info.tasks)
    }

    pub fn task_has_mfe_proxy(&self, task_id: &TaskId) -> bool {
        self.configs
            .values()
            .any(|info| info.tasks.contains_key(task_id))
    }

    pub fn config_filename(&self, package_name: &str) -> Option<&RelativeUnixPath> {
        let info = self.configs.get(package_name)?;
        let path = info.path.as_ref()?;
        Some(path)
    }

    pub fn dev_task_port(&self, task_id: &TaskId) -> Option<u16> {
        self.configs
            .values()
            .find_map(|config| config.ports.get(task_id).copied())
    }

    pub fn should_use_turborepo_proxy(&self) -> bool {
        self.configs
            .values()
            .all(|config| config.use_turborepo_proxy)
    }

    pub fn has_dev_task<'a>(&self, task_ids: impl Iterator<Item = &'a TaskId<'static>>) -> bool {
        task_ids.into_iter().any(|task_id| task_id.task() == "dev")
    }

    pub fn task_uses_turborepo_proxy(&self, task_id: &TaskId) -> bool {
        self.configs
            .values()
            .any(|config| config.tasks.contains_key(task_id) && config.use_turborepo_proxy)
    }

    pub fn update_turbo_json(
        &self,
        package_name: &PackageName,
        mut turbo_json: Result<TurboJson, config::Error>,
    ) -> Result<TurboJson, config::Error> {
        // We add all of the microfrontend configurations as global dependencies so any
        // changes will invalidate all tasks.
        // TODO: Move this to only apply to packages that are part of the
        // microfrontends. This will require us operating on task definitions
        // instead of `turbo.json`s which currently isn't feasible.
        if matches!(package_name, PackageName::Root) {
            if let Ok(turbo_json) = &mut turbo_json {
                turbo_json
                    .global_deps
                    .append(&mut self.configuration_file_paths());
            }
        }
        // If package either
        // - contains the proxy task
        // - a member of one of the microfrontends
        // then we need to modify its task definitions
        if let Some(FindResult {
            dev,
            proxy,
            use_turborepo_proxy,
            ..
        }) = self.package_turbo_json_update(package_name)
        {
            // If using Turborepo's built-in proxy, don't add proxy task to task graph
            if use_turborepo_proxy {
                return turbo_json;
            }

            // We need to modify turbo.json, use default one if there isn't one present
            let mut turbo_json = turbo_json.or_else(|err| match err {
                config::Error::NoTurboJSON => Ok(TurboJson::default()),
                err => Err(err),
            })?;

            // If the current package contains the proxy task, then add that definition
            if proxy.package() == package_name.as_str() {
                turbo_json.with_proxy(self.mfe_package);
            }

            if let Some(dev) = dev {
                // If this package has a dev task that's part of the MFE, then we make sure the
                // proxy gets included in the task graph.
                turbo_json.with_task(
                    TaskName::from(dev.task()).into_owned(),
                    &proxy.as_task_name(),
                );
            }

            Ok(turbo_json)
        } else {
            turbo_json
        }
    }

    fn package_turbo_json_update<'a>(
        &'a self,
        package_name: &PackageName,
    ) -> Option<FindResult<'a>> {
        tracing::debug!(
            "package_turbo_json_update - checking package: {}",
            package_name.as_str()
        );
        tracing::debug!(
            "package_turbo_json_update - available configs: {:?}",
            self.configs.keys().collect::<Vec<_>>()
        );

        let results = self.configs.iter().filter_map(|(config, info)| {
            let dev_task = info.tasks.iter().find_map(|(task, _app_name)| {
                (task.package() == package_name.as_str()).then(|| {
                    tracing::debug!(
                        "package_turbo_json_update - MATCH found dev task {} for package {}",
                        task,
                        package_name.as_str()
                    );
                    FindResult {
                        dev: Some(task.as_borrowed()),
                        proxy: TaskId::new(config, "proxy"),
                        version: info.version,
                        use_turborepo_proxy: info.use_turborepo_proxy,
                    }
                })
            });

            let proxy_owner = (config.as_str() == package_name.as_str()).then(|| {
                tracing::debug!(
                    "package_turbo_json_update - package {} owns the proxy config",
                    package_name.as_str()
                );
                FindResult {
                    dev: None,
                    proxy: TaskId::new(config, "proxy"),
                    version: info.version,
                    use_turborepo_proxy: info.use_turborepo_proxy,
                }
            });

            dev_task.or(proxy_owner)
        });
        // We invert the standard comparing order so higher versions are prioritized
        results.sorted_by(|a, b| b.version.cmp(a.version)).next()
    }

    // Returns a list of repo relative paths to all MFE configurations
    fn configuration_file_paths(&self) -> Vec<String> {
        self.configs
            .values()
            .filter_map(|info| {
                let path = info.path.as_ref()?;
                Some(path.as_str().to_string())
            })
            .collect()
    }
}

// Internal struct used to capture the results of checking the package graph
struct PackageGraphResult {
    configs: HashMap<String, ConfigInfo>,
    missing_default_apps: Vec<String>,
    missing_applications: Vec<String>,
    unsupported_version: Vec<(String, String)>,
    mfe_package: Option<&'static str>,
    has_mfe_dependency: bool,
}

impl PackageGraphResult {
    fn new<'a>(
        packages_in_graph: HashSet<&str>,
        packages: impl Iterator<Item = (&'a str, Result<Option<MfeConfig>, Error>)>,
        package_has_mfe_dependency: HashMap<&str, bool>,
    ) -> Result<Self, Error> {
        let mut configs = HashMap::new();
        let mut referenced_default_apps = HashSet::new();
        let mut referenced_packages = HashSet::new();
        let mut unsupported_version = Vec::new();
        let mut mfe_package = None;
        let mut has_mfe_dependency = false;
        // We sort packages to ensure deterministic behavior
        let sorted_packages = packages.sorted_by(|(a, _), (b, _)| a.cmp(b));
        for (package_name, config) in sorted_packages {
            // Check if this package is the @vercel/microfrontends package itself (workspace
            // package)
            if package_name == MICROFRONTENDS_PACKAGE {
                mfe_package = Some(MICROFRONTENDS_PACKAGE);
            }

            // Check if any package depends on @vercel/microfrontends
            if package_has_mfe_dependency
                .get(package_name)
                .copied()
                .unwrap_or(false)
            {
                has_mfe_dependency = true;
            }

            let Some(config) = config.or_else(|err| {
                match &err {
                    turborepo_microfrontends::Error::UnsupportedVersion(_) => {
                        unsupported_version.push((package_name.to_string(), err.to_string()));
                        Ok(None)
                    }
                    turborepo_microfrontends::Error::ChildConfig { reference } => {
                        referenced_default_apps.insert(reference.clone());
                        Ok(None)
                    }
                    turborepo_microfrontends::Error::JsonParse(msg)
                        if msg.contains("Found an unknown key") =>
                    {
                        // Only allow unknown keys if this package has @vercel/microfrontends
                        // dependency
                        let has_mfe_dep = package_has_mfe_dependency
                            .get(package_name)
                            .copied()
                            .unwrap_or(false);
                        if has_mfe_dep {
                            // Package uses @vercel/microfrontends, so Vercel-specific fields are
                            // allowed
                            Ok(None)
                        } else {
                            // Package doesn't use @vercel/microfrontends, reject Vercel-specific
                            // fields
                            Err(err)
                        }
                    }
                    _ => Err(err),
                }
            })?
            else {
                continue;
            };
            let mut info = ConfigInfo::new(&config);
            if let Some(path) = config.path() {
                info.path = Some(path.to_unix());
            }
            // Use Turborepo proxy if:
            // - No @vercel/microfrontends package in workspace AND
            // - No package depends on @vercel/microfrontends
            let pkg_has_mfe_dep = package_has_mfe_dependency
                .get(package_name)
                .copied()
                .unwrap_or(false);
            info.use_turborepo_proxy = mfe_package.is_none() && !pkg_has_mfe_dep;
            referenced_packages.insert(package_name.to_string());
            referenced_packages.extend(info.tasks.keys().map(|task| task.package().to_string()));

            // Validate that the config file is in the correct package
            if let Some((root_app, root_package)) = config.root_route_app() {
                if root_package != package_name {
                    return Err(turborepo_microfrontends::Error::ConfigInWrongPackage {
                        found_package: package_name.to_string(),
                        root_app: root_app.to_string(),
                        root_package: root_package.to_string(),
                    });
                }
            }

            configs.insert(package_name.to_string(), info);
        }
        let default_apps_found = configs.keys().cloned().collect();
        let mut missing_default_apps = referenced_default_apps
            .difference(&default_apps_found)
            .cloned()
            .collect::<Vec<_>>();
        missing_default_apps.sort();
        let mut missing_applications = referenced_packages
            .into_iter()
            .filter(|package| !packages_in_graph.contains(package.as_str()))
            .collect::<Vec<_>>();
        missing_applications.sort();
        Ok(Self {
            configs,
            missing_default_apps,
            missing_applications,
            unsupported_version,
            mfe_package,
            has_mfe_dependency,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FindResult<'a> {
    dev: Option<TaskId<'a>>,
    proxy: TaskId<'a>,
    version: &'static str,
    use_turborepo_proxy: bool,
}

impl ConfigInfo {
    fn new(config: &MfeConfig) -> Self {
        let mut ports = HashMap::new();
        let mut tasks = HashMap::new();
        tracing::debug!("ConfigInfo::new - creating config info");
        for dev_task in config.development_tasks() {
            let task_name = dev_task.task.unwrap_or("dev");
            let task = TaskId::new(dev_task.package, task_name).into_owned();

            if let Some(port) = config.port(dev_task.application_name) {
                ports.insert(task.clone(), port);
                tracing::debug!("ConfigInfo::new - added port {} for task {}", port, task);
            }

            tasks.insert(task.clone(), dev_task.application_name.to_owned());
        }
        let version = config.version();

        Self {
            tasks,
            version,
            ports,
            path: None,
            use_turborepo_proxy: false,
        }
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use turborepo_microfrontends::MICROFRONTENDS_PACKAGE;

    use super::*;

    struct PackageUpdateTest {
        package_name: &'static str,
        version: &'static str,
        result: Option<TestFindResult>,
    }

    struct TestFindResult {
        dev: Option<&'static str>,
        proxy: &'static str,
    }

    impl PackageUpdateTest {
        pub const fn new(package_name: &'static str) -> Self {
            Self {
                package_name,
                version: "1",
                result: None,
            }
        }

        pub const fn v1(mut self) -> Self {
            self.version = "1";
            self
        }

        pub const fn dev(mut self, dev: &'static str, proxy: &'static str) -> Self {
            self.result = Some(TestFindResult {
                dev: Some(dev),
                proxy,
            });
            self
        }

        pub const fn proxy_only(mut self, proxy: &'static str) -> Self {
            self.result = Some(TestFindResult { dev: None, proxy });
            self
        }

        pub fn package_name(&self) -> PackageName {
            PackageName::from(self.package_name)
        }

        pub fn expected(&self) -> Option<FindResult<'_>> {
            match self.result {
                Some(TestFindResult {
                    dev: Some(dev),
                    proxy,
                }) => Some(FindResult {
                    dev: Some(Self::str_to_task(dev)),
                    proxy: Self::str_to_task(proxy),
                    version: self.version,
                    use_turborepo_proxy: false,
                }),
                Some(TestFindResult { dev: None, proxy }) => Some(FindResult {
                    dev: None,
                    proxy: Self::str_to_task(proxy),
                    version: self.version,
                    use_turborepo_proxy: false,
                }),
                None => None,
            }
        }

        fn str_to_task(s: &str) -> TaskId<'static> {
            turborepo_task_id::TaskName::from(s)
                .task_id()
                .unwrap()
                .into_owned()
        }
    }

    #[test]
    fn test_mfe_package_is_found() {
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![(MICROFRONTENDS_PACKAGE, Ok(None))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.mfe_package, Some(MICROFRONTENDS_PACKAGE));
    }

    #[test]
    fn test_no_mfe_package() {
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![("foo", Ok(None)), ("bar", Ok(None))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.mfe_package, None);
    }

    #[test]
    fn test_use_turborepo_proxy_disabled_when_vercel_microfrontends_present() {
        // Create a microfrontends config
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                    "docs": {
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        // When @vercel/microfrontends package is present, use_turborepo_proxy should be
        // false
        let result_with_mfe_package = PackageGraphResult::new(
            HashSet::default(),
            vec![
                (MICROFRONTENDS_PACKAGE, Ok(None)),
                ("web", Ok(Some(config.clone()))),
            ]
            .into_iter(),
            HashMap::new(),
        )
        .unwrap();

        assert_eq!(
            result_with_mfe_package.mfe_package,
            Some(MICROFRONTENDS_PACKAGE)
        );
        assert!(
            result_with_mfe_package
                .configs
                .values()
                .all(|config| !config.use_turborepo_proxy),
            "use_turborepo_proxy should be false when @vercel/microfrontends is present"
        );

        // When @vercel/microfrontends package is NOT present, use_turborepo_proxy
        // should be true
        let result_without_mfe_package = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();

        assert_eq!(result_without_mfe_package.mfe_package, None);
        assert!(
            result_without_mfe_package
                .configs
                .values()
                .all(|config| config.use_turborepo_proxy),
            "use_turborepo_proxy should be true when @vercel/microfrontends is NOT present"
        );
    }

    #[test]
    fn test_child_configs_with_missing_default() {
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![(
                "child",
                Err(Error::ChildConfig {
                    reference: "main".into(),
                }),
            )]
            .into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.configs, HashMap::new());
        assert_eq!(result.missing_default_apps, &["main".to_string()]);
    }

    #[test]
    fn test_io_err_stops_traversal() {
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![
                ("a", Err(Error::Io(std::io::Error::other("something")))),
                (
                    "b",
                    Err(Error::ChildConfig {
                        reference: "main".into(),
                    }),
                ),
            ]
            .into_iter(),
            HashMap::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_packages() {
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                    "docs": {
                        "development": {
                            "local": 3000
                        },
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "something.txt",
        )
        .unwrap();
        let missing_result = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config.clone())))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(missing_result.missing_applications, vec!["docs", "web"]);
        let found_result = PackageGraphResult::new(
            HashSet::from_iter(["docs", "web"].iter().copied()),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert!(
            found_result.missing_applications.is_empty(),
            "Expected no missing applications: {:?}",
            found_result.missing_applications
        );
    }

    #[test]
    fn test_port_collection() {
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {
                        "development": {
                            "local": 5588
                        }
                    },
                    "docs": {
                        "development": {
                            "local": {
                                "port": 3030
                            },
                        },
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "something.txt",
        )
        .unwrap();
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        let web_ports = result.configs["web"].ports.clone();
        assert_eq!(
            web_ports.get(&TaskId::new("docs", "dev")).copied(),
            Some(3030)
        );
        assert_eq!(
            web_ports.get(&TaskId::new("web", "dev")).copied(),
            Some(5588)
        );
    }

    #[test]
    fn test_use_turborepo_proxy_false_when_package_has_mfe_dependency() {
        // Create a microfrontends config
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        // When a package depends on @vercel/microfrontends, use_turborepo_proxy should
        // be false
        let mut mfe_dependencies = HashMap::new();
        mfe_dependencies.insert("web", true);

        let result_with_dependency = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config.clone())))].into_iter(),
            mfe_dependencies,
        )
        .unwrap();

        assert_eq!(result_with_dependency.mfe_package, None);
        assert!(result_with_dependency.has_mfe_dependency);
        assert!(
            result_with_dependency
                .configs
                .values()
                .all(|config| !config.use_turborepo_proxy),
            "use_turborepo_proxy should be false when package depends on @vercel/microfrontends"
        );

        // When package does NOT depend on @vercel/microfrontends, use_turborepo_proxy
        // should be true
        let result_without_dependency = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();

        assert_eq!(result_without_dependency.mfe_package, None);
        assert!(!result_without_dependency.has_mfe_dependency);
        assert!(
            result_without_dependency
                .configs
                .values()
                .all(|config| config.use_turborepo_proxy),
            "use_turborepo_proxy should be true when package does NOT depend on \
             @vercel/microfrontends"
        );
    }

    #[test]
    fn test_has_dev_task_with_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_ids = [TaskId::new("web", "dev"), TaskId::new("docs", "build")];

        assert!(configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_without_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_ids = [TaskId::new("web", "build"), TaskId::new("docs", "lint")];

        assert!(!configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_only_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_ids = [TaskId::new("web", "dev")];

        assert!(configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_empty() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_ids: Vec<TaskId> = vec![];

        assert!(!configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_config_in_correct_package() {
        // Config file is in "web" package, and "web" is the root route app (no routing)
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                    "docs": {
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web", "docs"].iter().copied()),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        );

        assert!(result.is_ok(), "Config in correct package should succeed");
    }

    #[test]
    fn test_config_in_wrong_package() {
        // Config file is in "docs" package, but "web" is the root route app
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                    "docs": {
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web", "docs"].iter().copied()),
            vec![("docs", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        );

        match result {
            Err(turborepo_microfrontends::Error::ConfigInWrongPackage { .. }) => {
                // Expected error
            }
            Err(other) => panic!("Expected ConfigInWrongPackage error, got: {:?}", other),
            Ok(_) => panic!("Expected error but got success"),
        }
    }

    #[test]
    fn test_config_with_package_name_mapping() {
        // Config file is in "marketing" package, which is where "web" app (root route)
        // is actually implemented
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {
                        "packageName": "marketing"
                    },
                    "docs": {
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["marketing", "docs"].iter().copied()),
            vec![("marketing", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        );

        assert!(
            result.is_ok(),
            "Config in correct package (with packageName mapping) should succeed"
        );
    }

    #[test]
    fn test_config_with_package_name_mapping_in_wrong_package() {
        // Config file is in "docs" package, but "marketing" maps to "web" app (root
        // route)
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {
                        "packageName": "foo"
                    },
                    "docs": {
                        "routing": [{"paths": ["/docs", "/docs/:path*"]}]
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["foo", "docs"].iter().copied()),
            vec![("docs", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        );

        match result {
            Err(turborepo_microfrontends::Error::ConfigInWrongPackage { .. }) => {
                // Expected error
            }
            Err(other) => panic!("Expected ConfigInWrongPackage error, got: {:?}", other),
            Ok(_) => panic!("Expected error but got success with packageName mapping"),
        }
    }

    #[test]
    fn test_task_uses_turborepo_proxy_when_enabled() {
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {},
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web"].iter().copied()),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();

        let configs = MicrofrontendsConfigs {
            configs: result.configs,
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_id = TaskId::new("web", "dev");
        assert!(
            configs.task_uses_turborepo_proxy(&task_id),
            "Task should be using Turborepo proxy when @vercel/microfrontends is not present"
        );
    }

    #[test]
    fn test_task_uses_turborepo_proxy_returns_false_for_non_mfe_task() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_id = TaskId::new("web", "build");
        assert!(
            !configs.task_uses_turborepo_proxy(&task_id),
            "Non-MFE task should not be using Turborepo proxy"
        );
    }

    #[test]
    fn test_turbo_mfe_port_with_port_number() {
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {
                        "development": {
                            "local": 3001
                        }
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web"].iter().copied()),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();

        let configs = MicrofrontendsConfigs {
            configs: result.configs,
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_id = TaskId::new("web", "dev");
        assert_eq!(
            configs.dev_task_port(&task_id),
            Some(3001),
            "Port should be extracted from local config"
        );
    }

    #[test]
    fn test_turbo_mfe_port_with_url_string() {
        let config = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "applications": {
                    "web": {
                        "development": {
                            "local": "http://localhost:3000"
                        }
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web"].iter().copied()),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();

        let configs = MicrofrontendsConfigs {
            configs: result.configs,
            mfe_package: None,
            has_mfe_dependency: false,
        };

        let task_id = TaskId::new("web", "dev");
        assert_eq!(
            configs.dev_task_port(&task_id),
            Some(3000),
            "Port should be extracted from URL string"
        );
    }

    #[test]
    fn test_vercel_fields_rejected_without_dependency() {
        // Config with Vercel-specific fields
        let config_result = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "$schema": "https://example.com/schema.json",
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": 3000,
                            "task": "dev"
                        }
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        );

        // Should fail for package without @vercel/microfrontends dependency
        let result = PackageGraphResult::new(
            HashSet::from_iter(["web"].iter().copied()),
            vec![("web", config_result.map(Some))].into_iter(),
            HashMap::new(),
        );

        assert!(
            result.is_err(),
            "Config with Vercel fields should be rejected for packages without \
             @vercel/microfrontends"
        );
    }

    #[test]
    fn test_vercel_fields_accepted_with_dependency() {
        // Config with Vercel-specific fields
        let config_result = MfeConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "$schema": "https://example.com/schema.json",
                "version": "1",
                "applications": {
                    "web": {
                        "development": {
                            "local": 3000,
                            "task": "dev"
                        }
                    }
                }
            }))
            .unwrap(),
            "microfrontends.json",
        );

        // Should succeed for package with @vercel/microfrontends dependency
        let mut deps = std::collections::HashMap::new();
        deps.insert("web", true);

        let result = PackageGraphResult::new(
            HashSet::from_iter(["web"].iter().copied()),
            vec![("web", config_result.map(Some))].into_iter(),
            deps,
        );

        assert!(
            result.is_ok(),
            "Config with Vercel fields should be accepted for packages with @vercel/microfrontends"
        );
    }
}
