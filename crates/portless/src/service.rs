//! Native startup-service specifications and lifecycle operations.
//!
//! The generated launchd plist, systemd unit, and Task Scheduler command file
//! intentionally mirror portless 0.15.1 so an installation can be inspected or
//! replaced by either implementation.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use thiserror::Error;

pub const SERVICE_LABEL: &str = "sh.portless.proxy";
pub const SYSTEMD_SERVICE: &str = "portless.service";
pub const WINDOWS_TASK_NAME: &str = "Portless Proxy";
pub const INTERNAL_ELEVATED_ENV: &str = "PORTLESS_INTERNAL_SERVICE_ELEVATED";
pub const DEFAULT_TLD: &str = "localhost";
pub const DEFAULT_SERVICE_PORT: u16 = 443;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedPlatform {
    MacOs,
    Linux,
    Windows,
}

impl SupportedPlatform {
    pub fn current() -> Result<Self, ServiceError> {
        if cfg!(target_os = "macos") {
            Ok(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Ok(Self::Windows)
        } else {
            Err(ServiceError::UnsupportedPlatform)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstallConfig {
    pub state_dir: Option<PathBuf>,
    pub proxy_port: u16,
    pub use_https: bool,
    pub custom_cert_path: Option<PathBuf>,
    pub custom_key_path: Option<PathBuf>,
    pub lan_mode: bool,
    pub lan_ip: Option<String>,
    pub lan_ip_explicit: bool,
    pub tld: String,
    pub tlds: Vec<String>,
    pub use_wildcard: bool,
    pub extra_env: BTreeMap<String, String>,
}

impl Default for ServiceInstallConfig {
    fn default() -> Self {
        Self {
            state_dir: None,
            proxy_port: DEFAULT_SERVICE_PORT,
            use_https: true,
            custom_cert_path: None,
            custom_key_path: None,
            lan_mode: false,
            lan_ip: None,
            lan_ip_explicit: false,
            tld: DEFAULT_TLD.into(),
            tlds: vec![DEFAULT_TLD.into()],
            use_wildcard: false,
            extra_env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedServiceConfig {
    pub state_dir: PathBuf,
    pub proxy_port: u16,
    pub use_https: bool,
    pub custom_cert_path: Option<PathBuf>,
    pub custom_key_path: Option<PathBuf>,
    pub lan_mode: bool,
    pub lan_ip: Option<String>,
    pub lan_ip_explicit: bool,
    pub tld: String,
    pub tlds: Vec<String>,
    pub use_wildcard: bool,
    pub extra_env: BTreeMap<String, String>,
}

impl NormalizedServiceConfig {
    fn from_install(config: ServiceInstallConfig, state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            proxy_port: config.proxy_port,
            use_https: config.use_https,
            custom_cert_path: config.custom_cert_path,
            custom_key_path: config.custom_key_path,
            lan_mode: config.lan_mode,
            lan_ip: config.lan_ip,
            lan_ip_explicit: config.lan_ip_explicit,
            tld: config.tld,
            tlds: config.tlds,
            use_wildcard: config.use_wildcard,
            extra_env: config.extra_env,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildServiceSpecOptions {
    pub platform: SupportedPlatform,
    /// Runtime executable (`nodePath` in the TypeScript implementation).
    pub node_path: PathBuf,
    /// Portless entry script, or the Rust executable when `node_path` is empty.
    pub entry_script: PathBuf,
    pub user_home: PathBuf,
    pub uid: Option<String>,
    pub gid: Option<String>,
    pub username: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub path_env: Option<String>,
    pub program_data: Option<PathBuf>,
    pub install_config: ServiceInstallConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceSpec {
    Launchd {
        label: String,
        plist_path: PathBuf,
        plist: String,
        state_dir: PathBuf,
        config: NormalizedServiceConfig,
        program_arguments: Vec<String>,
    },
    Systemd {
        service_name: String,
        unit_path: PathBuf,
        unit: String,
        state_dir: PathBuf,
        config: NormalizedServiceConfig,
        exec_start: Vec<String>,
    },
    Windows {
        task_name: String,
        state_dir: PathBuf,
        config: NormalizedServiceConfig,
        script_dir: PathBuf,
        script_path: PathBuf,
        script: String,
        task_run: String,
        create_args: Vec<String>,
        run_args: Vec<String>,
        delete_args: Vec<String>,
        query_args: Vec<String>,
    },
}

impl ServiceSpec {
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        match self {
            Self::Launchd { state_dir, .. }
            | Self::Systemd { state_dir, .. }
            | Self::Windows { state_dir, .. } => state_dir,
        }
    }

    #[must_use]
    pub fn config(&self) -> &NormalizedServiceConfig {
        match self {
            Self::Launchd { config, .. }
            | Self::Systemd { config, .. }
            | Self::Windows { config, .. } => config,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCommandOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ServiceCommandOutput {
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

pub trait ServiceCommandRunner {
    fn run(
        &self,
        command: &str,
        args: &[String],
        inherit_stdio: bool,
    ) -> io::Result<ServiceCommandOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdServiceCommandRunner;

impl ServiceCommandRunner for StdServiceCommandRunner {
    fn run(
        &self,
        command: &str,
        args: &[String],
        inherit_stdio: bool,
    ) -> io::Result<ServiceCommandOutput> {
        let output = Command::new(command)
            .args(args)
            .stdin(if inherit_stdio {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .stdout(if inherit_stdio {
                Stdio::inherit()
            } else {
                Stdio::piped()
            })
            .stderr(if inherit_stdio {
                Stdio::inherit()
            } else {
                Stdio::piped()
            })
            .output()?;
        Ok(ServiceCommandOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("unsupported platform")]
    UnsupportedPlatform,
    #[error("{0}")]
    InvalidConfig(String),
    #[error("{command} failed: {detail}")]
    Command { command: String, detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub installed: bool,
    pub manager_state: String,
    pub proxy_running: bool,
    pub config: NormalizedServiceConfig,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUninstallResult {
    pub removed: bool,
    pub installed: bool,
    pub error: Option<String>,
    pub needs_elevation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallAction {
    WriteFile {
        path: PathBuf,
        contents: String,
        mode: Option<u32>,
    },
    CreateDir(PathBuf),
    Run {
        command: String,
        args: Vec<String>,
        required: bool,
    },
    RemoveFile(PathBuf),
    RemoveDir(PathBuf),
}

/// Parse the options accepted by `portless service install`.
pub fn parse_service_install_config(
    args: &[String],
    environment: &BTreeMap<String, String>,
    allow_runtime_flags: bool,
) -> Result<ServiceInstallConfig, ServiceError> {
    let mut config = ServiceInstallConfig::default();
    if let Some(value) = environment.get("PORTLESS_SYNC_HOSTS")
        && !value.is_empty()
    {
        config
            .extra_env
            .insert("PORTLESS_SYNC_HOSTS".into(), value.clone());
    }
    if let Some(value) = environment.get("PORTLESS_STATE_DIR") {
        config.state_dir = Some(PathBuf::from(value));
    }
    if let Some(value) = environment
        .get("PORTLESS_HTTPS")
        .and_then(|value| parse_boolean(value))
    {
        config.use_https = value;
    }
    if let Some(value) = environment
        .get("PORTLESS_LAN")
        .and_then(|value| parse_boolean(value))
    {
        config.lan_mode = value;
    }
    if let Some(value) = environment.get("PORTLESS_LAN_IP") {
        config.lan_mode = true;
        config.lan_ip = Some(value.clone());
        config.lan_ip_explicit = true;
    }
    if let Some(value) = environment.get("PORTLESS_TLD") {
        config.tlds = normalize_tlds(parse_tld_list(value, "PORTLESS_TLD")?);
        config.tld = primary_tld(&config.tlds);
    }
    if let Some(value) = environment
        .get("PORTLESS_WILDCARD")
        .and_then(|value| parse_boolean(value))
    {
        config.use_wildcard = value;
    }
    if let Some(value) = environment.get("PORTLESS_PORT") {
        config.proxy_port = parse_port(value, "PORTLESS_PORT")?;
    } else {
        config.proxy_port = protocol_port(config.use_https);
    }

    let tokens = if args.first().is_some_and(|arg| arg == "service") {
        args.get(2..).unwrap_or_default()
    } else {
        args
    };
    let mut explicit_port = false;
    let mut tld_seen = false;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "-p" | "--port" => {
                config.proxy_port = parse_port(flag_value(tokens, index, token)?, token)?;
                explicit_port = true;
                index += 2;
            }
            "--https" => {
                config.use_https = true;
                index += 1;
            }
            "--no-tls" => {
                config.use_https = false;
                index += 1;
            }
            "--lan" => {
                config.lan_mode = true;
                index += 1;
            }
            "--ip" => {
                config.lan_mode = true;
                config.lan_ip = Some(flag_value(tokens, index, token)?.to_owned());
                config.lan_ip_explicit = true;
                index += 2;
            }
            "--tld" => {
                let parsed = parse_tld_list(flag_value(tokens, index, token)?, token)?;
                let mut values = if tld_seen { config.tlds } else { Vec::new() };
                values.extend(parsed);
                config.tlds = normalize_tlds(values);
                config.tld = primary_tld(&config.tlds);
                tld_seen = true;
                index += 2;
            }
            "--wildcard" => {
                config.use_wildcard = true;
                index += 1;
            }
            "--cert" => {
                config.custom_cert_path = Some(PathBuf::from(flag_value(tokens, index, token)?));
                config.use_https = true;
                index += 2;
            }
            "--key" => {
                config.custom_key_path = Some(PathBuf::from(flag_value(tokens, index, token)?));
                config.use_https = true;
                index += 2;
            }
            "--state-dir" => {
                config.state_dir = Some(PathBuf::from(flag_value(tokens, index, token)?));
                index += 2;
            }
            "--foreground" | "--skip-trust" if allow_runtime_flags => index += 1,
            _ => {
                return Err(ServiceError::InvalidConfig(format!(
                    "Unknown service install option \"{token}\"."
                )));
            }
        }
    }
    if config.custom_cert_path.is_some() != config.custom_key_path.is_some() {
        return Err(ServiceError::InvalidConfig(
            "--cert and --key must be used together.".into(),
        ));
    }
    if !explicit_port && !environment.contains_key("PORTLESS_PORT") {
        config.proxy_port = protocol_port(config.use_https);
    }
    if config.lan_mode {
        config.tlds = vec!["local".into()];
        config.tld = "local".into();
    } else {
        config.lan_ip = None;
        config.lan_ip_explicit = false;
    }
    Ok(config)
}

/// Expand home-relative service paths and resolve relative paths against the
/// invoking process's working directory.
///
/// This matches upstream's `normalizeServiceInstallPaths`: only the state
/// directory and custom certificate/key paths are transformed.
#[must_use]
pub fn normalize_service_install_paths(
    mut config: ServiceInstallConfig,
    platform: SupportedPlatform,
    user_home: &Path,
    current_dir: &Path,
) -> ServiceInstallConfig {
    config.state_dir = config
        .state_dir
        .map(|path| resolve_service_path(&path, platform, user_home, current_dir));
    config.custom_cert_path = config
        .custom_cert_path
        .map(|path| resolve_service_path(&path, platform, user_home, current_dir));
    config.custom_key_path = config
        .custom_key_path
        .map(|path| resolve_service_path(&path, platform, user_home, current_dir));
    config
}

/// Build the native manager specification without touching the filesystem.
pub fn build_service_spec(
    mut options: BuildServiceSpecOptions,
) -> Result<ServiceSpec, ServiceError> {
    options.install_config.tlds = if options.install_config.lan_mode {
        vec!["local".into()]
    } else {
        normalize_tlds(if options.install_config.tlds.is_empty() {
            vec![options.install_config.tld.clone()]
        } else {
            options.install_config.tlds.clone()
        })
    };
    options.install_config.tld = primary_tld(&options.install_config.tlds);
    let state_dir = options
        .state_dir
        .clone()
        .or_else(|| options.install_config.state_dir.clone())
        .unwrap_or_else(|| default_state_dir(options.platform, &options.user_home));
    let config = NormalizedServiceConfig::from_install(options.install_config, state_dir.clone());
    let path_env = options
        .path_env
        .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into());
    let command = build_proxy_command(&options.entry_script, &config);
    let mut program_command = Vec::new();
    if !options.node_path.as_os_str().is_empty() {
        program_command.push(path_string(&options.node_path));
    }
    program_command.extend(command);
    let env = build_service_env(
        options.platform,
        &config,
        &options.user_home,
        options.uid.as_deref(),
        options.gid.as_deref(),
        &path_env,
    );
    match options.platform {
        SupportedPlatform::MacOs => {
            let plist_path = PathBuf::from(format!("/Library/LaunchDaemons/{SERVICE_LABEL}.plist"));
            Ok(ServiceSpec::Launchd {
                label: SERVICE_LABEL.into(),
                plist_path,
                plist: build_launchd_plist(&config.state_dir, &program_command, &env),
                state_dir,
                config,
                program_arguments: program_command,
            })
        }
        SupportedPlatform::Linux => {
            let unit_path = PathBuf::from(format!("/etc/systemd/system/{SYSTEMD_SERVICE}"));
            Ok(ServiceSpec::Systemd {
                service_name: SYSTEMD_SERVICE.into(),
                unit_path,
                unit: build_systemd_unit(&program_command, &env, &path_env),
                state_dir,
                config,
                exec_start: program_command,
            })
        }
        SupportedPlatform::Windows => {
            let program_data = options
                .program_data
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            let script_dir = windows_join(&program_data, &["portless", "service"]);
            let script_path = windows_join(&script_dir, &["portless-service.cmd"]);
            let script = build_windows_script(&program_command, &env);
            let task_run = windows_quote(&path_string(&script_path));
            Ok(ServiceSpec::Windows {
                task_name: WINDOWS_TASK_NAME.into(),
                state_dir,
                config,
                script_dir,
                script_path,
                script,
                task_run: task_run.clone(),
                create_args: strings([
                    "/Create",
                    "/TN",
                    WINDOWS_TASK_NAME,
                    "/SC",
                    "ONSTART",
                    "/RU",
                    "SYSTEM",
                    "/RL",
                    "HIGHEST",
                    "/TR",
                    &task_run,
                    "/F",
                ]),
                run_args: strings(["/Run", "/TN", WINDOWS_TASK_NAME]),
                delete_args: strings(["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"]),
                query_args: strings(["/Query", "/TN", WINDOWS_TASK_NAME, "/FO", "LIST", "/V"]),
            })
        }
    }
}

#[must_use]
pub fn install_actions(spec: &ServiceSpec) -> Vec<InstallAction> {
    let mut actions = match spec {
        ServiceSpec::Launchd {
            label,
            plist_path,
            plist,
            ..
        } => vec![
            InstallAction::Run {
                command: "launchctl".into(),
                args: vec!["bootout".into(), "system".into(), path_string(plist_path)],
                required: false,
            },
            InstallAction::WriteFile {
                path: plist_path.clone(),
                contents: plist.clone(),
                mode: Some(0o644),
            },
            InstallAction::Run {
                command: "chown".into(),
                args: vec!["root:wheel".into(), path_string(plist_path)],
                required: true,
            },
            InstallAction::Run {
                command: "launchctl".into(),
                args: vec!["bootstrap".into(), "system".into(), path_string(plist_path)],
                required: true,
            },
            InstallAction::Run {
                command: "launchctl".into(),
                args: vec!["enable".into(), format!("system/{label}")],
                required: true,
            },
            InstallAction::Run {
                command: "launchctl".into(),
                args: vec!["kickstart".into(), "-k".into(), format!("system/{label}")],
                required: true,
            },
        ],
        ServiceSpec::Systemd {
            service_name,
            unit_path,
            unit,
            ..
        } => vec![
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["disable".into(), "--now".into(), service_name.clone()],
                required: false,
            },
            InstallAction::WriteFile {
                path: unit_path.clone(),
                contents: unit.clone(),
                mode: Some(0o644),
            },
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["daemon-reload".into()],
                required: true,
            },
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["enable".into(), service_name.clone()],
                required: true,
            },
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["restart".into(), service_name.clone()],
                required: true,
            },
        ],
        ServiceSpec::Windows {
            task_name,
            script_dir,
            script_path,
            script,
            create_args,
            run_args,
            ..
        } => vec![
            InstallAction::Run {
                command: "schtasks".into(),
                args: vec!["/End".into(), "/TN".into(), task_name.clone()],
                required: false,
            },
            InstallAction::CreateDir(script_dir.clone()),
            InstallAction::WriteFile {
                path: script_path.clone(),
                contents: script.clone(),
                mode: None,
            },
            InstallAction::Run {
                command: "schtasks".into(),
                args: create_args.clone(),
                required: true,
            },
            InstallAction::Run {
                command: "schtasks".into(),
                args: run_args.clone(),
                required: false,
            },
        ],
    };
    if let Some(stop) = target_proxy_stop_action(spec) {
        actions.insert(1, stop);
    }
    actions
}

#[must_use]
pub fn uninstall_actions(spec: &ServiceSpec) -> Vec<InstallAction> {
    match spec {
        ServiceSpec::Launchd { plist_path, .. } => vec![
            InstallAction::Run {
                command: "launchctl".into(),
                args: vec!["bootout".into(), "system".into(), path_string(plist_path)],
                required: false,
            },
            InstallAction::RemoveFile(plist_path.clone()),
        ],
        ServiceSpec::Systemd {
            service_name,
            unit_path,
            ..
        } => vec![
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["disable".into(), "--now".into(), service_name.clone()],
                required: false,
            },
            InstallAction::RemoveFile(unit_path.clone()),
            InstallAction::Run {
                command: "systemctl".into(),
                args: vec!["daemon-reload".into()],
                required: false,
            },
        ],
        ServiceSpec::Windows {
            task_name,
            delete_args,
            script_dir,
            ..
        } => vec![
            InstallAction::Run {
                command: "schtasks".into(),
                args: vec!["/End".into(), "/TN".into(), task_name.clone()],
                required: false,
            },
            InstallAction::Run {
                command: "schtasks".into(),
                args: delete_args.clone(),
                required: false,
            },
            InstallAction::RemoveDir(script_dir.clone()),
        ],
    }
}

pub struct ServiceManager<R = StdServiceCommandRunner> {
    runner: R,
}

impl Default for ServiceManager<StdServiceCommandRunner> {
    fn default() -> Self {
        Self {
            runner: StdServiceCommandRunner,
        }
    }
}

impl<R: ServiceCommandRunner> ServiceManager<R> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn install(&self, spec: &ServiceSpec) -> Result<(), ServiceError> {
        fs::create_dir_all(spec.state_dir())?;
        for action in install_actions(spec) {
            self.execute(action)?;
        }
        Ok(())
    }

    pub fn uninstall(&self, spec: &ServiceSpec) -> Result<(), ServiceError> {
        for action in uninstall_actions(spec) {
            self.execute(action)?;
        }
        Ok(())
    }

    pub fn try_uninstall(&self, spec: &ServiceSpec) -> ServiceUninstallResult {
        let installed = self.is_installed(spec);
        if !installed {
            return ServiceUninstallResult {
                removed: false,
                installed: false,
                error: None,
                needs_elevation: false,
            };
        }
        match self.uninstall(spec) {
            Ok(()) => ServiceUninstallResult {
                removed: true,
                installed: true,
                error: None,
                needs_elevation: false,
            },
            Err(error) => {
                let message = error.to_string();
                ServiceUninstallResult {
                    removed: false,
                    installed: true,
                    needs_elevation: is_permission_error(&message),
                    error: Some(message),
                }
            }
        }
    }

    pub fn status(
        &self,
        spec: &ServiceSpec,
        proxy_running: bool,
    ) -> Result<ServiceStatus, ServiceError> {
        let config = read_installed_service_config(spec).unwrap_or_else(|| spec.config().clone());
        match spec {
            ServiceSpec::Launchd {
                label, plist_path, ..
            } => {
                let output =
                    self.optional("launchctl", &["print".into(), format!("system/{label}")]);
                let installed = plist_path.exists();
                let combined = format!("{}{}", output.stdout, output.stderr);
                let running = output.success()
                    && (combined.contains("state = running") || contains_pid_assignment(&combined));
                Ok(ServiceStatus {
                    installed,
                    manager_state: if running {
                        "running".into()
                    } else if installed {
                        "installed".into()
                    } else {
                        "not installed".into()
                    },
                    proxy_running,
                    config,
                    details: Some(path_string(plist_path)),
                })
            }
            ServiceSpec::Systemd {
                service_name,
                unit_path,
                ..
            } => {
                let enabled =
                    self.optional("systemctl", &["is-enabled".into(), service_name.clone()]);
                let active =
                    self.optional("systemctl", &["is-active".into(), service_name.clone()]);
                let installed = enabled.success() || active.success() || unit_path.exists();
                Ok(ServiceStatus {
                    installed,
                    manager_state: if active.success() {
                        let state = active.stdout.trim();
                        if state.is_empty() {
                            "active".into()
                        } else {
                            state.into()
                        }
                    } else if installed {
                        "installed".into()
                    } else {
                        "not installed".into()
                    },
                    proxy_running,
                    config,
                    details: Some(path_string(unit_path)),
                })
            }
            ServiceSpec::Windows {
                task_name,
                query_args,
                ..
            } => {
                let query = self.optional("schtasks", query_args);
                let installed = query.success();
                let manager_state = if installed {
                    windows_status(&format!("{}{}", query.stdout, query.stderr))
                        .unwrap_or_else(|| "installed".into())
                } else {
                    "not installed".into()
                };
                Ok(ServiceStatus {
                    installed,
                    manager_state,
                    proxy_running,
                    config,
                    details: Some(task_name.clone()),
                })
            }
        }
    }

    fn execute(&self, action: InstallAction) -> Result<(), ServiceError> {
        match action {
            InstallAction::WriteFile {
                path,
                contents,
                mode,
            } => {
                fs::write(&path, contents)?;
                if let Some(mode) = mode {
                    set_mode(&path, mode)?;
                }
                Ok(())
            }
            InstallAction::CreateDir(path) => fs::create_dir_all(path).map_err(Into::into),
            InstallAction::Run {
                command,
                args,
                required,
            } => {
                let result = self.runner.run(&command, &args, false)?;
                if required && !result.success() {
                    Err(ServiceError::Command {
                        command,
                        detail: command_detail(&result),
                    })
                } else {
                    Ok(())
                }
            }
            InstallAction::RemoveFile(path) => remove_file_if_present(&path).map_err(Into::into),
            InstallAction::RemoveDir(path) => remove_dir_if_present(&path).map_err(Into::into),
        }
    }

    fn optional(&self, command: &str, args: &[String]) -> ServiceCommandOutput {
        self.runner
            .run(command, args, false)
            .unwrap_or_else(|error| ServiceCommandOutput {
                status: None,
                stdout: String::new(),
                stderr: error.to_string(),
            })
    }

    fn is_installed(&self, spec: &ServiceSpec) -> bool {
        match spec {
            ServiceSpec::Launchd { plist_path, .. } => plist_path.exists(),
            ServiceSpec::Systemd { unit_path, .. } => unit_path.exists(),
            ServiceSpec::Windows { query_args, .. } => {
                self.optional("schtasks", query_args).success()
            }
        }
    }
}

/// Build the arguments passed after `sudo`.
#[must_use]
pub fn build_elevated_env_args(
    home: &Path,
    state_dir: &Path,
    environment: &BTreeMap<String, String>,
    extra_env: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut override_keys: BTreeSet<&str> = BTreeSet::new();
    override_keys.insert("PORTLESS_STATE_DIR");
    override_keys.extend(extra_env.keys().map(String::as_str));
    let mut args = vec!["env".into()];
    args.extend(
        environment
            .iter()
            .filter(|(key, value)| {
                key.starts_with("PORTLESS_")
                    && !value.is_empty()
                    && !override_keys.contains(key.as_str())
            })
            .map(|(key, value)| format!("{key}={value}")),
    );
    args.extend(
        extra_env
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );
    args.push(format!("HOME={}", path_string(home)));
    args.push(format!("PORTLESS_STATE_DIR={}", path_string(state_dir)));
    args
}

#[must_use]
pub fn build_service_uninstall_sudo_args(
    entry_script: &Path,
    node_path: &Path,
    home: &Path,
    state_dir: &Path,
    environment: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut args = build_elevated_env_args(home, state_dir, environment, &BTreeMap::new());
    if !node_path.as_os_str().is_empty() {
        args.push(path_string(node_path));
    }
    args.extend([
        path_string(entry_script),
        "service".into(),
        "uninstall".into(),
    ]);
    args
}

#[must_use]
pub fn build_install_elevation_args(
    entry_script: &Path,
    node_path: &Path,
    original_args: &[String],
    home: &Path,
    state_dir: &Path,
    environment: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut extra = BTreeMap::new();
    extra.insert(INTERNAL_ELEVATED_ENV.into(), "1".into());
    let mut args = build_elevated_env_args(home, state_dir, environment, &extra);
    if !node_path.as_os_str().is_empty() {
        args.push(path_string(node_path));
    }
    args.push(path_string(entry_script));
    args.extend(original_args.iter().cloned());
    args
}

#[must_use]
pub fn read_installed_service_config(spec: &ServiceSpec) -> Option<NormalizedServiceConfig> {
    let snapshot = read_installed_snapshot(spec)?;
    installed_config_from_snapshot(&snapshot, spec.config())
}

#[derive(Debug)]
struct InstalledSnapshot {
    command: Vec<String>,
    env: BTreeMap<String, String>,
}

fn build_proxy_command(entry_script: &Path, config: &NormalizedServiceConfig) -> Vec<String> {
    let mut command = vec![
        path_string(entry_script),
        "proxy".into(),
        "start".into(),
        "--foreground".into(),
        "--port".into(),
        config.proxy_port.to_string(),
    ];
    if config.use_https {
        if let (Some(cert), Some(key)) = (&config.custom_cert_path, &config.custom_key_path) {
            command.extend([
                "--cert".into(),
                path_string(cert),
                "--key".into(),
                path_string(key),
            ]);
        } else {
            command.push("--https".into());
        }
    } else {
        command.push("--no-tls".into());
    }
    if config.lan_mode {
        command.push("--lan".into());
        if config.lan_ip_explicit
            && let Some(ip) = &config.lan_ip
        {
            command.extend(["--ip".into(), ip.clone()]);
        }
    } else if config.tlds.len() > 1 || config.tld != DEFAULT_TLD {
        for tld in &config.tlds {
            command.extend(["--tld".into(), tld.clone()]);
        }
    }
    if config.use_wildcard {
        command.push("--wildcard".into());
    }
    command.push("--skip-trust".into());
    command
}

fn target_proxy_stop_action(spec: &ServiceSpec) -> Option<InstallAction> {
    let command = match spec {
        ServiceSpec::Launchd {
            program_arguments, ..
        } => program_arguments.clone(),
        ServiceSpec::Systemd { exec_start, .. } => exec_start.clone(),
        ServiceSpec::Windows { script, .. } => script
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.eq_ignore_ascii_case("@echo off"))
            .rfind(|line| !line.to_ascii_lowercase().starts_with("set \""))
            .map(|line| parse_quoted_words(line, false))?,
    };
    let proxy_index = command.iter().position(|arg| arg == "proxy")?;
    let executable = command.first()?.clone();
    let mut args = command.get(1..proxy_index)?.to_vec();
    args.extend([
        "proxy".into(),
        "stop".into(),
        "--port".into(),
        spec.config().proxy_port.to_string(),
    ]);
    Some(InstallAction::Run {
        command: executable,
        args,
        required: true,
    })
}

fn build_service_env(
    platform: SupportedPlatform,
    config: &NormalizedServiceConfig,
    home: &Path,
    uid: Option<&str>,
    gid: Option<&str>,
    path_env: &str,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([
        ("PORTLESS_STATE_DIR".into(), path_string(&config.state_dir)),
        ("PORTLESS_PORT".into(), config.proxy_port.to_string()),
        (
            "PORTLESS_HTTPS".into(),
            if config.use_https { "1" } else { "0" }.into(),
        ),
        (
            "PORTLESS_LAN".into(),
            if config.lan_mode { "1" } else { "0" }.into(),
        ),
        (
            "PORTLESS_WILDCARD".into(),
            if config.use_wildcard { "1" } else { "0" }.into(),
        ),
    ]);
    values.extend(config.extra_env.clone());
    if config.lan_mode
        && config.lan_ip_explicit
        && let Some(ip) = &config.lan_ip
    {
        values.insert("PORTLESS_LAN_IP".into(), ip.clone());
    }
    if config.lan_mode {
        values.insert("PORTLESS_TLD".into(), "local".into());
    } else if config.tlds.len() > 1 || config.tld != DEFAULT_TLD {
        values.insert("PORTLESS_TLD".into(), config.tlds.join(","));
    }
    match platform {
        SupportedPlatform::Windows => {
            values.insert("USERPROFILE".into(), path_string(home));
            values.insert("PATH".into(), path_env.into());
        }
        SupportedPlatform::MacOs | SupportedPlatform::Linux => {
            values.insert("HOME".into(), path_string(home));
            if let Some(uid) = uid {
                values.insert("SUDO_UID".into(), uid.into());
            }
            if let Some(gid) = gid {
                values.insert("SUDO_GID".into(), gid.into());
            }
        }
    }
    values
}

fn build_launchd_plist(
    state_dir: &Path,
    command: &[String],
    environment: &BTreeMap<String, String>,
) -> String {
    let args = command
        .iter()
        .map(|arg| format!("      <string>{}</string>", xml_escape(arg)))
        .collect::<Vec<_>>()
        .join("\n");
    let env = environment
        .iter()
        .map(|(key, value)| {
            format!(
                "      <key>{}</key>\n      <string>{}</string>",
                xml_escape(key),
                xml_escape(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let log = xml_escape(&path_string(&state_dir.join("service.log")));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>{SERVICE_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{args}
    </array>
    <key>EnvironmentVariables</key>
    <dict>
{env}
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
  </dict>
</plist>
"#
    )
}

fn build_systemd_unit(
    command: &[String],
    environment: &BTreeMap<String, String>,
    path_env: &str,
) -> String {
    let env_lines = environment
        .iter()
        .map(|(key, value)| format!("Environment={key}={}", systemd_escape(value)))
        .collect::<Vec<_>>()
        .join("\n");
    let exec = command
        .iter()
        .map(|value| systemd_escape(value))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        concat!(
            "[Unit]\n",
            "Description=Portless HTTPS proxy\n",
            "After=network-online.target\n",
            "Wants=network-online.target\n\n",
            "[Service]\n",
            "Type=simple\n",
            "{env_lines}\n",
            "Environment=PATH={}\n",
            "ExecStart={exec}\n",
            "Restart=on-failure\n",
            "RestartSec=2\n",
            "KillSignal=SIGTERM\n",
            "TimeoutStopSec=5\n\n",
            "[Install]\n",
            "WantedBy=multi-user.target\n"
        ),
        systemd_escape(path_env),
        env_lines = env_lines,
        exec = exec
    )
}

fn build_windows_script(command: &[String], environment: &BTreeMap<String, String>) -> String {
    let variables = environment
        .iter()
        .map(|(key, value)| {
            format!(
                "set \"{key}={}\"",
                value.replace('"', "").replace('%', "%%")
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    let command = command
        .iter()
        .map(|arg| windows_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!("@echo off\r\n{variables}\r\n{command}\r\n")
}

fn read_installed_snapshot(spec: &ServiceSpec) -> Option<InstalledSnapshot> {
    match spec {
        ServiceSpec::Launchd { plist_path, .. } => {
            let contents = fs::read_to_string(plist_path).ok()?;
            let args_block = xml_block_after_key(&contents, "ProgramArguments", "array")?;
            let env_block =
                xml_block_after_key(&contents, "EnvironmentVariables", "dict").unwrap_or_default();
            Some(InstalledSnapshot {
                command: parse_xml_strings(args_block),
                env: parse_xml_env(env_block),
            })
        }
        ServiceSpec::Systemd { unit_path, .. } => {
            let contents = fs::read_to_string(unit_path).ok()?;
            let mut environment = BTreeMap::new();
            let mut command = None;
            for line in contents.lines() {
                if let Some(entry) = line.strip_prefix("Environment=") {
                    if let Some((key, value)) = entry.split_once('=') {
                        environment.insert(
                            key.into(),
                            parse_quoted_words(value, true)
                                .into_iter()
                                .next()
                                .unwrap_or_default(),
                        );
                    }
                } else if let Some(value) = line.strip_prefix("ExecStart=") {
                    command = Some(parse_quoted_words(value, true));
                }
            }
            Some(InstalledSnapshot {
                command: command?,
                env: environment,
            })
        }
        ServiceSpec::Windows { script_path, .. } => {
            let contents = fs::read_to_string(script_path).ok()?;
            let mut environment = BTreeMap::new();
            let mut command = None;
            for line in contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                if line.eq_ignore_ascii_case("@echo off") {
                    continue;
                }
                if let Some(value) = line
                    .strip_prefix("set \"")
                    .and_then(|v| v.strip_suffix('"'))
                    && let Some((key, value)) = value.split_once('=')
                {
                    environment.insert(key.into(), value.replace("%%", "%"));
                    continue;
                }
                command = Some(parse_quoted_words(line, false));
            }
            Some(InstalledSnapshot {
                command: command?,
                env: environment,
            })
        }
    }
}

fn installed_config_from_snapshot(
    snapshot: &InstalledSnapshot,
    fallback: &NormalizedServiceConfig,
) -> Option<NormalizedServiceConfig> {
    let proxy_index = snapshot
        .command
        .windows(2)
        .position(|pair| pair == ["proxy", "start"])?;
    let mut args = vec!["service".into(), "install".into()];
    args.extend(snapshot.command.get(proxy_index + 2..)?.iter().cloned());
    let parsed = parse_service_install_config(&args, &snapshot.env, true).ok()?;
    let state_dir = parsed
        .state_dir
        .clone()
        .or_else(|| snapshot.env.get("PORTLESS_STATE_DIR").map(PathBuf::from))
        .unwrap_or_else(|| fallback.state_dir.clone());
    Some(NormalizedServiceConfig::from_install(parsed, state_dir))
}

fn xml_block_after_key<'a>(contents: &'a str, key: &str, tag: &str) -> Option<&'a str> {
    let marker = format!("<key>{key}</key>");
    let after_key = contents.split_once(&marker)?.1;
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    let after_open = after_key.split_once(&opening)?.1;
    Some(after_open.split_once(&closing)?.0)
}

fn parse_xml_strings(block: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = block;
    while let Some((_, after)) = rest.split_once("<string>") {
        let Some((value, next)) = after.split_once("</string>") else {
            break;
        };
        values.push(xml_unescape(value));
        rest = next;
    }
    values
}

fn parse_xml_env(block: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let mut rest = block;
    while let Some((_, after_key)) = rest.split_once("<key>") {
        let Some((key, after_key_end)) = after_key.split_once("</key>") else {
            break;
        };
        let Some((_, after_string)) = after_key_end.split_once("<string>") else {
            break;
        };
        let Some((value, next)) = after_string.split_once("</string>") else {
            break;
        };
        values.insert(xml_unescape(key), xml_unescape(value));
        rest = next;
    }
    values
}

fn parse_quoted_words(input: &str, unescape_backslash: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut in_word = false;
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character == '"' {
            in_quote = !in_quote;
            in_word = true;
        } else if character == '\\'
            && index + 1 < chars.len()
            && (chars[index + 1] == '"' || (unescape_backslash && chars[index + 1] == '\\'))
        {
            current.push(chars[index + 1]);
            in_word = true;
            index += 1;
        } else if character.is_whitespace() && !in_quote {
            if in_word {
                words.push(std::mem::take(&mut current));
                in_word = false;
            }
        } else {
            current.push(character);
            in_word = true;
        }
        index += 1;
    }
    if in_word {
        words.push(current);
    }
    words
}

fn parse_tld_list(value: &str, source: &str) -> Result<Vec<String>, ServiceError> {
    let values = value
        .split(',')
        .map(|tld| tld.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|tld| !tld.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty()
        || values.iter().any(|tld| {
            !tld.chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
    {
        return Err(ServiceError::InvalidConfig(format!(
            "{source} must contain valid TLD names."
        )));
    }
    Ok(values)
}

fn normalize_tlds(values: Vec<String>) -> Vec<String> {
    let values = if values.is_empty() {
        vec![DEFAULT_TLD.into()]
    } else {
        values
    };
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn primary_tld(tlds: &[String]) -> String {
    tlds.first().cloned().unwrap_or_else(|| DEFAULT_TLD.into())
}

fn protocol_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn parse_port(value: &str, source: &str) -> Result<u16, ServiceError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            ServiceError::InvalidConfig(format!("{source} must be a number between 1 and 65535."))
        })
}

fn flag_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, ServiceError> {
    args.get(index + 1)
        .filter(|value| !value.starts_with('-'))
        .map(String::as_str)
        .ok_or_else(|| ServiceError::InvalidConfig(format!("{flag} requires a value.")))
}

fn default_state_dir(platform: SupportedPlatform, home: &Path) -> PathBuf {
    match platform {
        SupportedPlatform::Windows => windows_join(home, &[".portless"]),
        SupportedPlatform::MacOs | SupportedPlatform::Linux => home.join(".portless"),
    }
}

fn resolve_service_path(
    value: &Path,
    platform: SupportedPlatform,
    user_home: &Path,
    current_dir: &Path,
) -> PathBuf {
    let value = path_string(value);
    match platform {
        SupportedPlatform::Windows => resolve_windows_service_path(&value, user_home, current_dir),
        SupportedPlatform::MacOs | SupportedPlatform::Linux => {
            resolve_unix_service_path(&value, user_home, current_dir)
        }
    }
}

fn expand_home_path(value: &str, user_home: &Path, separator: char) -> String {
    if value == "~" {
        return path_string(user_home);
    }
    if let Some(remainder) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        let mut expanded = path_string(user_home)
            .trim_end_matches(['/', '\\'])
            .to_owned();
        expanded.push(separator);
        expanded.push_str(remainder);
        return expanded;
    }
    value.to_owned()
}

fn resolve_unix_service_path(value: &str, user_home: &Path, current_dir: &Path) -> PathBuf {
    let expanded = expand_home_path(value, user_home, '/');
    let absolute = if expanded.starts_with('/') {
        expanded
    } else {
        format!(
            "{}/{}",
            path_string(current_dir).trim_end_matches('/'),
            expanded
        )
    };
    PathBuf::from(normalize_absolute_path(&absolute, '/', 0))
}

fn resolve_windows_service_path(value: &str, user_home: &Path, current_dir: &Path) -> PathBuf {
    let expanded = expand_home_path(value, user_home, '\\').replace('/', "\\");
    let absolute = if is_windows_absolute(&expanded) {
        expanded
    } else if expanded.starts_with('\\') {
        let current = path_string(current_dir).replace('/', "\\");
        let drive = current.get(..2).filter(|prefix| prefix.ends_with(':'));
        format!("{}{expanded}", drive.unwrap_or_default())
    } else {
        format!(
            "{}\\{}",
            path_string(current_dir).trim_end_matches(['/', '\\']),
            expanded
        )
        .replace('/', "\\")
    };
    let protected_components = if absolute.starts_with(r"\\") { 2 } else { 0 };
    PathBuf::from(normalize_absolute_path(
        &absolute,
        '\\',
        protected_components,
    ))
}

fn is_windows_absolute(value: &str) -> bool {
    value.starts_with(r"\\")
        || value
            .as_bytes()
            .get(1..3)
            .is_some_and(|suffix| suffix[0] == b':' && matches!(suffix[1], b'\\' | b'/'))
}

fn normalize_absolute_path(value: &str, separator: char, protected_components: usize) -> String {
    let has_unc_prefix = separator == '\\' && value.starts_with(r"\\");
    let has_unix_root = separator == '/' && value.starts_with('/');
    let drive = (separator == '\\')
        .then(|| value.get(..2))
        .flatten()
        .filter(|prefix| prefix.ends_with(':'));
    let body = drive.map_or(value, |drive| &value[drive.len()..]);
    let mut components = Vec::new();
    for component in body.split(separator) {
        match component {
            "" | "." => {}
            ".." if components.len() > protected_components => {
                components.pop();
            }
            ".." => {}
            component => components.push(component),
        }
    }
    let joined = components.join(&separator.to_string());
    if let Some(drive) = drive {
        if joined.is_empty() {
            format!("{drive}{separator}")
        } else {
            format!("{drive}{separator}{joined}")
        }
    } else if has_unc_prefix {
        format!(r"\\{joined}")
    } else if has_unix_root {
        format!("/{joined}")
    } else {
        joined
    }
}

fn windows_join(base: &Path, parts: &[&str]) -> PathBuf {
    let mut value = path_string(base).trim_end_matches(['\\', '/']).to_owned();
    for part in parts {
        value.push('\\');
        value.push_str(part);
    }
    PathBuf::from(value)
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

fn systemd_escape(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn windows_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn command_detail(output: &ServiceCommandOutput) -> String {
    for detail in [&output.stderr, &output.stdout] {
        let trimmed = detail.trim();
        if !trimmed.is_empty() {
            return trimmed.into();
        }
    }
    "command failed".into()
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_dir_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

fn is_permission_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "eacces",
        "eperm",
        "permission denied",
        "operation not permitted",
        "access is denied",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn contains_pid_assignment(output: &str) -> bool {
    output.lines().any(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .is_some_and(|pid| pid.trim().chars().all(|c| c.is_ascii_digit()))
    })
}

fn windows_status(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        key.eq_ignore_ascii_case("Status")
            .then(|| value.trim().to_owned())
    })
}

/// Resolve the current environment into spec-building options for the Rust
/// executable. `entry_script` is normally `current_exe()` and `node_path` is
/// empty, producing one executable at the front of the manager command.
pub fn current_service_spec(
    entry_script: impl Into<PathBuf>,
    install_config: Option<ServiceInstallConfig>,
) -> Result<ServiceSpec, ServiceError> {
    let platform = SupportedPlatform::current()?;
    let environment: BTreeMap<String, String> = env::vars().collect();
    let current_dir = env::current_dir()?;
    let home = environment
        .get(if platform == SupportedPlatform::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| current_dir.clone());
    let mut config = install_config.unwrap_or_default();
    if config.state_dir.is_none() {
        config.state_dir = environment.get("PORTLESS_STATE_DIR").map(PathBuf::from);
    }
    let config = normalize_service_install_paths(config, platform, &home, &current_dir);
    build_service_spec(BuildServiceSpecOptions {
        platform,
        node_path: PathBuf::new(),
        entry_script: entry_script.into(),
        user_home: home,
        uid: environment.get("SUDO_UID").cloned(),
        gid: environment.get("SUDO_GID").cloned(),
        username: environment
            .get("SUDO_USER")
            .or_else(|| environment.get("USER"))
            .cloned(),
        state_dir: config.state_dir.clone(),
        path_env: environment.get("PATH").cloned(),
        program_data: environment.get("ProgramData").map(PathBuf::from),
        install_config: config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(platform: SupportedPlatform) -> BuildServiceSpecOptions {
        BuildServiceSpecOptions {
            platform,
            node_path: "/usr/bin/node".into(),
            entry_script: "/opt/portless/cli.js".into(),
            user_home: "/Users/test user".into(),
            uid: Some("501".into()),
            gid: Some("20".into()),
            username: Some("test".into()),
            state_dir: Some("/Users/test user/.portless".into()),
            path_env: Some("/usr/local/bin:/usr/bin:/bin".into()),
            program_data: Some(r"C:\ProgramData".into()),
            install_config: ServiceInstallConfig::default(),
        }
    }

    #[test]
    fn launchd_spec_contains_arguments_environment_and_logs() {
        let spec = build_service_spec(options(SupportedPlatform::MacOs)).expect("build spec");
        let ServiceSpec::Launchd { plist, .. } = spec else {
            panic!("expected launchd");
        };
        assert!(plist.contains("<string>/usr/bin/node</string>"));
        assert!(plist.contains("<key>PORTLESS_STATE_DIR</key>"));
        assert!(plist.contains("service.log"));
    }

    #[test]
    fn systemd_values_are_quoted() {
        let spec = build_service_spec(options(SupportedPlatform::Linux)).expect("build spec");
        let ServiceSpec::Systemd { unit, .. } = spec else {
            panic!("expected systemd");
        };
        assert!(unit.contains("ExecStart=\"/usr/bin/node\""));
        assert!(unit.contains("Environment=PORTLESS_STATE_DIR=\"/Users/test user/.portless\""));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn windows_task_runs_as_system_at_startup() {
        let spec = build_service_spec(options(SupportedPlatform::Windows)).expect("build spec");
        let ServiceSpec::Windows {
            create_args,
            script,
            ..
        } = spec
        else {
            panic!("expected windows");
        };
        assert!(create_args.windows(2).any(|pair| pair == ["/RU", "SYSTEM"]));
        assert!(
            create_args
                .windows(2)
                .any(|pair| pair == ["/SC", "ONSTART"])
        );
        assert!(script.contains("set \"PORTLESS_PORT=443\""));
        assert!(script.contains("\r\n"));
    }

    #[test]
    fn parser_applies_multi_tld_and_lan_rules() {
        let args = strings([
            "service",
            "install",
            "--tld",
            "test,localhost",
            "--tld",
            "internal",
        ]);
        let parsed =
            parse_service_install_config(&args, &BTreeMap::new(), false).expect("parse config");
        assert_eq!(parsed.tlds, ["test", "localhost", "internal"]);
        let lan = parse_service_install_config(
            &strings(["service", "install", "--lan", "--tld", "test"]),
            &BTreeMap::new(),
            false,
        )
        .expect("parse lan");
        assert_eq!(lan.tlds, ["local"]);
    }

    #[test]
    fn normalizes_unix_service_paths_with_home_and_working_directory() {
        let config = ServiceInstallConfig {
            state_dir: Some("../state/./portless".into()),
            custom_cert_path: Some("~/certs/server.pem".into()),
            custom_key_path: Some(r"~\certs/server-key.pem".into()),
            ..ServiceInstallConfig::default()
        };
        let normalized = normalize_service_install_paths(
            config,
            SupportedPlatform::Linux,
            Path::new("/home/alice"),
            Path::new("/work/repo"),
        );

        assert_eq!(
            normalized.state_dir,
            Some(PathBuf::from("/work/state/portless"))
        );
        assert_eq!(
            normalized.custom_cert_path,
            Some(PathBuf::from("/home/alice/certs/server.pem"))
        );
        assert_eq!(
            normalized.custom_key_path,
            Some(PathBuf::from("/home/alice/certs/server-key.pem"))
        );
    }

    #[test]
    fn normalizes_windows_service_paths_lexically() {
        let config = ServiceInstallConfig {
            state_dir: Some(r"~\state\..\portless".into()),
            custom_cert_path: Some(r"certs\server.pem".into()),
            custom_key_path: Some(r"D:/shared/./server-key.pem".into()),
            ..ServiceInstallConfig::default()
        };
        let normalized = normalize_service_install_paths(
            config,
            SupportedPlatform::Windows,
            Path::new(r"C:\Users\Alice"),
            Path::new(r"C:\work\repo"),
        );

        assert_eq!(
            normalized.state_dir,
            Some(PathBuf::from(r"C:\Users\Alice\portless"))
        );
        assert_eq!(
            normalized.custom_cert_path,
            Some(PathBuf::from(r"C:\work\repo\certs\server.pem"))
        );
        assert_eq!(
            normalized.custom_key_path,
            Some(PathBuf::from(r"D:\shared\server-key.pem"))
        );
    }

    #[test]
    fn normalized_paths_feed_elevation_and_persisted_spec() {
        let config = normalize_service_install_paths(
            ServiceInstallConfig {
                state_dir: Some("relative-state".into()),
                custom_cert_path: Some("cert.pem".into()),
                custom_key_path: Some("key.pem".into()),
                ..ServiceInstallConfig::default()
            },
            SupportedPlatform::Linux,
            Path::new("/home/alice"),
            Path::new("/work/repo"),
        );
        let mut build_options = options(SupportedPlatform::Linux);
        build_options.state_dir = None;
        build_options.install_config = config;
        let spec = build_service_spec(build_options).expect("build normalized spec");
        let ServiceSpec::Systemd {
            state_dir,
            exec_start,
            unit,
            ..
        } = spec
        else {
            panic!("expected systemd");
        };

        assert_eq!(state_dir, PathBuf::from("/work/repo/relative-state"));
        assert!(exec_start.contains(&"/work/repo/cert.pem".into()));
        assert!(exec_start.contains(&"/work/repo/key.pem".into()));
        assert!(unit.contains("PORTLESS_STATE_DIR=\"/work/repo/relative-state\""));
        let sudo_args = build_install_elevation_args(
            Path::new("/usr/bin/portless"),
            Path::new(""),
            &strings(["service", "install"]),
            Path::new("/home/alice"),
            &state_dir,
            &BTreeMap::new(),
        );
        assert!(sudo_args.contains(&"PORTLESS_STATE_DIR=/work/repo/relative-state".into()));
    }

    #[test]
    fn elevation_preserves_portless_environment_and_overrides_state() {
        let environment = BTreeMap::from([
            ("PORTLESS_PORT".into(), "8443".into()),
            ("PORTLESS_STATE_DIR".into(), "/old".into()),
        ]);
        let extra = BTreeMap::from([(INTERNAL_ELEVATED_ENV.into(), "1".into())]);
        let args = build_elevated_env_args(
            Path::new("/home/me"),
            Path::new("/state"),
            &environment,
            &extra,
        );
        assert!(args.contains(&"PORTLESS_PORT=8443".into()));
        assert!(args.contains(&"PORTLESS_STATE_DIR=/state".into()));
        assert!(!args.contains(&"PORTLESS_STATE_DIR=/old".into()));
    }
}
