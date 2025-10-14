use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use tracing::warn;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath, RelativeUnixPathBuf};
use turborepo_microfrontends::{Config as MFEConfig, Error, MICROFRONTENDS_PACKAGE};
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::{TaskId, TaskName};

use crate::{config, turbo_json::TurboJson};

#[derive(Debug, Clone)]
pub struct MicrofrontendsConfigs {
    configs: HashMap<String, ConfigInfo>,
    mfe_package: Option<&'static str>,
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
        let package_names = package_graph
            .packages()
            .map(|(name, _)| name.as_str())
            .collect();
        let package_has_proxy_script: HashMap<&str, bool> = package_graph
            .packages()
            .map(|(name, info)| {
                (
                    name.as_str(),
                    info.package_json.scripts.contains_key("proxy"),
                )
            })
            .collect();
        Self::from_configs(
            package_names,
            package_graph.packages().map(|(name, info)| {
                (
                    name.as_str(),
                    MFEConfig::load_from_dir(repo_root, info.package_path()),
                )
            }),
            package_has_proxy_script,
        )
    }

    /// Constructs a collection of configurations from a list of configurations
    pub fn from_configs<'a>(
        package_names: HashSet<&str>,
        configs: impl Iterator<Item = (&'a str, Result<Option<MFEConfig>, Error>)>,
        package_has_proxy_script: HashMap<&str, bool>,
    ) -> Result<Option<Self>, Error> {
        let PackageGraphResult {
            configs,
            missing_default_apps,
            missing_applications,
            unsupported_version,
            mfe_package,
        } = PackageGraphResult::new(package_names, configs, package_has_proxy_script)?;

        for (package, err) in unsupported_version {
            warn!("Ignoring {package}: {err}");
        }

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

        Ok((!configs.is_empty()).then_some(Self {
            configs,
            mfe_package,
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
            .any(|config| config.use_turborepo_proxy)
    }

    pub fn has_dev_task<'a>(&self, task_ids: impl Iterator<Item = &'a TaskId<'static>>) -> bool {
        task_ids.into_iter().any(|task_id| task_id.task() == "dev")
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
        let results = self.configs.iter().filter_map(|(config, info)| {
            let dev_task = info.tasks.iter().find_map(|(task, _)| {
                (task.package() == package_name.as_str()).then(|| FindResult {
                    dev: Some(task.as_borrowed()),
                    proxy: TaskId::new(config, "proxy"),
                    version: info.version,
                    use_turborepo_proxy: info.use_turborepo_proxy,
                })
            });
            let proxy_owner = (config.as_str() == package_name.as_str()).then(|| FindResult {
                dev: None,
                proxy: TaskId::new(config, "proxy"),
                version: info.version,
                use_turborepo_proxy: info.use_turborepo_proxy,
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
}

impl PackageGraphResult {
    fn new<'a>(
        packages_in_graph: HashSet<&str>,
        packages: impl Iterator<Item = (&'a str, Result<Option<MFEConfig>, Error>)>,
        package_has_proxy_script: HashMap<&str, bool>,
    ) -> Result<Self, Error> {
        let mut configs = HashMap::new();
        let mut referenced_default_apps = HashSet::new();
        let mut referenced_packages = HashSet::new();
        let mut unsupported_version = Vec::new();
        let mut mfe_package = None;
        // We sort packages to ensure deterministic behavior
        let sorted_packages = packages.sorted_by(|(a, _), (b, _)| a.cmp(b));
        for (package_name, config) in sorted_packages {
            if package_name == MICROFRONTENDS_PACKAGE {
                mfe_package = Some(MICROFRONTENDS_PACKAGE);
            }

            let Some(config) = config.or_else(|err| match err {
                turborepo_microfrontends::Error::UnsupportedVersion(_) => {
                    unsupported_version.push((package_name.to_string(), err.to_string()));
                    Ok(None)
                }
                turborepo_microfrontends::Error::ChildConfig { reference } => {
                    referenced_default_apps.insert(reference);
                    Ok(None)
                }
                err => Err(err),
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
            // - No custom proxy script in this package
            let has_custom_proxy = package_has_proxy_script
                .get(package_name)
                .copied()
                .unwrap_or(false);
            info.use_turborepo_proxy = mfe_package.is_none() && !has_custom_proxy;
            referenced_packages.insert(package_name.to_string());
            referenced_packages.extend(info.tasks.keys().map(|task| task.package().to_string()));
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
    fn new(config: &MFEConfig) -> Self {
        let mut ports = HashMap::new();
        let mut tasks = HashMap::new();
        for dev_task in config.development_tasks() {
            let task = TaskId::new(dev_task.package, dev_task.task.unwrap_or("dev")).into_owned();
            if let Some(port) = config.port(dev_task.application_name) {
                ports.insert(task.clone(), port);
            }
            tasks.insert(task, dev_task.application_name.to_owned());
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

    macro_rules! mfe_configs {
        {$($config_owner:expr => $dev_tasks:expr),+} => {
            {
                let mut _map = std::collections::HashMap::new();
                $(
                    let mut _dev_tasks = std::collections::HashMap::new();
                    for _dev_task in $dev_tasks.as_slice() {
                        let _dev_task_id = turborepo_task_id::TaskName::from(*_dev_task).task_id().unwrap().into_owned();
                        let _dev_application = _dev_task_id.package().to_owned();
                        _dev_tasks.insert(_dev_task_id, _dev_application);
                    }
                    _map.insert($config_owner.to_string(), ConfigInfo { tasks: _dev_tasks, version: "1", path: None, ports: std::collections::HashMap::new(), use_turborepo_proxy: false });
                )+
                _map
            }
        };
    }

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
        let config = MFEConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "version": "1",
                "applications": {
                    "web": {},
                    "docs": {
                        "development": {
                            "task": "serve"
                        }
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
    fn test_use_turborepo_proxy_disabled_with_custom_proxy_script() {
        // Create a microfrontends config
        let config = MFEConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "version": "1",
                "applications": {
                    "web": {},
                }
            }))
            .unwrap(),
            "microfrontends.json",
        )
        .unwrap();

        // When package has a custom proxy script, use_turborepo_proxy should be false
        let mut proxy_scripts = HashMap::new();
        proxy_scripts.insert("web", true);

        let result_with_proxy_script = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config)))].into_iter(),
            proxy_scripts,
        )
        .unwrap();

        assert_eq!(result_with_proxy_script.mfe_package, None);
        assert!(
            result_with_proxy_script
                .configs
                .values()
                .all(|config| !config.use_turborepo_proxy),
            "use_turborepo_proxy should be false when package has custom proxy script"
        );
    }

    #[test]
    fn test_unsupported_versions_ignored() {
        let result = PackageGraphResult::new(
            HashSet::default(),
            vec![("foo", Err(Error::UnsupportedVersion("bad version".into())))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.configs, HashMap::new());
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
    fn test_dev_task_collection() {
        let config = MFEConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "version": "1",
                "applications": {
                    "web": {},
                    "docs": {
                        "development": {
                            "task": "serve"
                        }
                    }
                }
            }))
            .unwrap(),
            "something.txt",
        )
        .unwrap();
        let mut result = PackageGraphResult::new(
            HashSet::default(),
            vec![("web", Ok(Some(config)))].into_iter(),
            HashMap::new(),
        )
        .unwrap();
        result.configs.values_mut().for_each(|config| {
            config.ports.clear();
            config.use_turborepo_proxy = false;
        });
        assert_eq!(
            result.configs,
            mfe_configs!(
                "web" => ["web#dev", "docs#serve"]
            )
        )
    }

    #[test]
    fn test_missing_packages() {
        let config = MFEConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "version": "1",
                "applications": {
                    "web": {},
                    "docs": {
                        "development": {
                            "task": "serve"
                        }
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
        let config = MFEConfig::from_str(
            &serde_json::to_string_pretty(&json!({
                "version": "1",
                "applications": {
                    "web": {},
                    "docs": {
                        "development": {
                            "task": "serve",
                            "local": {
                                "port": 3030
                            }
                        }
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
            web_ports.get(&TaskId::new("docs", "serve")).copied(),
            Some(3030)
        );
        assert_eq!(
            web_ports.get(&TaskId::new("web", "dev")).copied(),
            Some(5588)
        );
    }

    #[test]
    fn test_configs_added_as_global_deps() {
        let configs = MicrofrontendsConfigs {
            configs: vec![(
                "web".to_owned(),
                ConfigInfo {
                    path: Some(RelativeUnixPathBuf::new("web/microfrontends.json").unwrap()),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            mfe_package: None,
        };

        let turbo_json = TurboJson::default();
        let actual = configs
            .update_turbo_json(&PackageName::Root, Ok(turbo_json))
            .unwrap();
        assert_eq!(actual.global_deps, &["web/microfrontends.json".to_owned()]);
    }

    #[test]
    fn test_has_dev_task_with_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
        };

        let task_ids = vec![TaskId::new("web", "dev"), TaskId::new("docs", "build")];

        assert!(configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_without_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
        };

        let task_ids = vec![TaskId::new("web", "build"), TaskId::new("docs", "lint")];

        assert!(!configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_only_dev() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
        };

        let task_ids = vec![TaskId::new("web", "dev")];

        assert!(configs.has_dev_task(task_ids.iter()));
    }

    #[test]
    fn test_has_dev_task_empty() {
        let configs = MicrofrontendsConfigs {
            configs: HashMap::new(),
            mfe_package: None,
        };

        let task_ids: Vec<TaskId> = vec![];

        assert!(!configs.has_dev_task(task_ids.iter()));
    }
}
