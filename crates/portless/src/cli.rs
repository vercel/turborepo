//! Command-line interface for Portless 0.15.1.
//!
//! Argument parsing is intentionally performed here rather than through a
//! derive-based parser: Portless has two command-shaped app modes and must stop
//! interpreting options at exactly the same points as the JavaScript CLI.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fs, io,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, ExitCode, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, anyhow, bail};
#[cfg(unix)]
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};

use crate::{
    auto::{detect_worktree_prefix, infer_project_name, truncate_label},
    certs, clean,
    config::{self, AppConfig},
    hosts, mdns, ngrok,
    process::{self, ChildEnv, PortlessChildEnv},
    proxy::{ProxyOptions, ProxyServer, TlsConfig},
    routes::{Route, RouteMetadata, RouteStore},
    service::{self, ServiceManager},
    tailscale::{self, EnsureTailscaleReadyOptions, TailscaleMode, TailscaleRoute},
    turbo,
    workspace::{self, WorkspacePackage},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TLD: &str = "localhost";
const FALLBACK_PROXY_PORT: u16 = 1355;
const PRIVILEGED_PORT_THRESHOLD: u16 = 1024;
const INTERNAL_LAN_IP_FLAG: &str = "--lan-ip-auto";
const INTERNAL_LAN_IP_ENV: &str = "PORTLESS_INTERNAL_LAN_IP";
const WAIT_ATTEMPTS: usize = 20;
const WAIT_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq)]
struct State {
    dir: PathBuf,
    port: u16,
    tls: bool,
    tlds: Vec<String>,
    lan_ip: Option<Ipv4Addr>,
    custom_cert: bool,
}

impl State {
    fn primary_tld(&self) -> &str {
        self.tlds.first().map_or(DEFAULT_TLD, String::as_str)
    }

    fn lan_mode(&self) -> bool {
        self.lan_ip.is_some() || self.tlds.iter().any(|tld| tld == "local")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Sharing {
    tailscale: bool,
    funnel: bool,
    ngrok: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ParsedApp {
    name: Option<String>,
    force: bool,
    app_port: Option<u16>,
    command: Vec<String>,
    sharing: Sharing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProxyConfig {
    port: u16,
    port_explicit: bool,
    tls: bool,
    tls_explicit: bool,
    foreground: bool,
    skip_trust: bool,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    lan: bool,
    lan_explicit: bool,
    lan_ip: Option<Ipv4Addr>,
    lan_ip_explicit: bool,
    tlds: Vec<String>,
    tlds_explicit: bool,
    wildcard: bool,
}

#[derive(Clone, Debug)]
struct MultiApp {
    package: WorkspacePackage,
    name: String,
    label: String,
    command: Vec<String>,
    app_port: Option<u16>,
    proxied: bool,
}

/// Run the command and convert failures to the CLI's conventional exit code.
pub fn run<I, T>(arguments: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args = arguments
        .into_iter()
        .map(Into::into)
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if let Err(error) = block_one_off_package_runner() {
        eprintln!("Error: {error:#}");
        return ExitCode::FAILURE;
    }
    let global_script = match strip_global_flags(&mut args) {
        Ok(script) => script,
        Err(error) => {
            eprintln!("Error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Error: could not start async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(main_async(args, global_script)) {
        Ok(code) => exit_code(code),
        Err(error) if error.downcast_ref::<CliControl>().is_some() => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn exit_code(code: i32) -> ExitCode {
    u8::try_from(code.clamp(0, 255)).map_or(ExitCode::FAILURE, ExitCode::from)
}

async fn main_async(mut args: Vec<String>, global_script: Option<String>) -> Result<i32> {
    if args.first().is_some_and(|arg| arg == "--name") {
        args.remove(0);
        let name = args
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("--name requires an app name"))?;
        let mut parsed = parse_named_args(&args)?;
        parsed.name = Some(name);
        return dispatch_app(parsed, false, global_script.as_deref()).await;
    }

    let run_mode = args.first().is_some_and(|arg| arg == "run");
    if run_mode {
        args.remove(0);
    }

    if bypass_requested()
        && (run_mode
            || args.is_empty()
            || (args.len() >= 2
                && !matches!(
                    args.first().map(String::as_str),
                    Some("proxy" | "clean" | "doctor" | "service")
                )))
    {
        let parsed = if run_mode {
            parse_run_args(&args)?
        } else if args.is_empty() {
            ParsedApp::default()
        } else {
            parse_named_args(&args)?
        };
        return run_bypassed(parsed.command, global_script.as_deref()).await;
    }

    if run_mode {
        return dispatch_app(parse_run_args(&args)?, true, global_script.as_deref()).await;
    }

    match args.first().map(String::as_str) {
        Some("-h" | "--help") => {
            print_help();
            Ok(0)
        }
        Some("-v" | "--version") => {
            println!("{VERSION}");
            Ok(0)
        }
        None | Some("--") => {
            let extra = if args.first().is_some_and(|arg| arg == "--") {
                args.get(1..).unwrap_or_default()
            } else {
                &[]
            };
            if handle_default(global_script.as_deref(), extra).await? {
                Ok(0)
            } else {
                print_help();
                Ok(0)
            }
        }
        Some("get") => handle_get(&args).await,
        Some("alias") => handle_alias(&args).await,
        Some("list") => handle_list().await,
        Some("doctor") => handle_doctor(&args).await,
        Some("trust") => handle_trust().await,
        Some("clean") => handle_clean(&args).await,
        Some("prune") => handle_prune(&args).await,
        Some("hosts") => handle_hosts(&args).await,
        Some("proxy") => handle_proxy(&args).await,
        Some("service") => handle_service(&args).await,
        Some(_) => dispatch_app(parse_named_args(&args)?, false, global_script.as_deref()).await,
    }
}

fn block_one_off_package_runner() -> Result<()> {
    let local = is_locally_installed();
    let npx = env::var("npm_command").as_deref() == Ok("exec")
        && env::var_os("npm_lifecycle_event").is_none();
    let pnpm_dlx = env::var_os("PNPM_SCRIPT_SRC_DIR").is_some()
        && env::var_os("npm_lifecycle_event").is_none();
    if (npx || pnpm_dlx) && !local {
        bail!(
            "portless should not be run via npx or pnpm dlx.\nInstall globally or as a project \
             dependency:\n  npm install -g portless\n  npm install -D portless"
        );
    }
    Ok(())
}

fn is_locally_installed() -> bool {
    env::current_dir().ok().is_some_and(|cwd| {
        cwd.ancestors()
            .any(|dir| dir.join("node_modules/portless/package.json").exists())
    })
}

fn strip_global_flags(args: &mut Vec<String>) -> Result<Option<String>> {
    if strip_bool_flag(args, "--lan") {
        set_env("PORTLESS_LAN", "1");
    }
    if let Some(ip) = strip_value_flag(args, "--ip")? {
        let _: IpAddr = ip
            .parse()
            .with_context(|| format!("invalid IP address \"{ip}\""))?;
        set_env("PORTLESS_LAN_IP", &ip);
        set_env("PORTLESS_LAN", "1");
    }
    if let Some(ip) = strip_value_flag(args, INTERNAL_LAN_IP_FLAG)? {
        let _: IpAddr = ip
            .parse()
            .with_context(|| format!("invalid IP address \"{ip}\""))?;
        set_env(INTERNAL_LAN_IP_ENV, &ip);
        set_env("PORTLESS_LAN", "1");
    }
    if strip_bool_flag(args, "--tailscale") {
        set_env("PORTLESS_TAILSCALE", "1");
    }
    if strip_bool_flag(args, "--funnel") {
        set_env("PORTLESS_FUNNEL", "1");
        set_env("PORTLESS_TAILSCALE", "1");
    }
    if strip_bool_flag(args, "--ngrok") {
        set_env("PORTLESS_NGROK", "1");
    }
    strip_value_flag(args, "--script")
}

fn flag_region_end(args: &[String]) -> usize {
    args.iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len())
}

fn strip_bool_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let end = flag_region_end(args);
    if let Some(index) = args.iter().take(end).position(|arg| arg == flag) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn strip_value_flag(args: &mut Vec<String>, flag: &str) -> Result<Option<String>> {
    let end = flag_region_end(args);
    let Some(index) = args.iter().take(end).position(|arg| arg == flag) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| anyhow!("{flag} requires a value"))?;
    args.drain(index..=index + 1);
    Ok(Some(value))
}

fn set_env(key: &str, value: &str) {
    // SAFETY: CLI argument normalization happens before the runtime starts
    // application worker tasks.
    unsafe { env::set_var(key, value) };
}

fn enabled(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| matches!(value.to_str(), Some("1" | "true")))
}

fn bypass_requested() -> bool {
    process::portless_bypass_requested(env::var_os("PORTLESS").as_deref())
}

fn parse_run_args(args: &[String]) -> Result<ParsedApp> {
    parse_app_flags(args, false)
}

fn parse_named_args(args: &[String]) -> Result<ParsedApp> {
    parse_app_flags(args, true)
}

fn parse_app_flags(args: &[String], named: bool) -> Result<ParsedApp> {
    let mut parsed = ParsedApp::default();
    let mut index = 0;
    let mut parsing_flags = true;
    while index < args.len() {
        let arg = &args[index];
        if parsing_flags && arg == "--" {
            parsing_flags = false;
            index += 1;
            continue;
        }
        if named && parsed.name.is_none() && (!parsing_flags || !arg.starts_with('-')) {
            parsed.name = Some(arg.clone());
            index += 1;
            continue;
        }
        let flags_allowed = parsing_flags
            && (parsed.name.is_none() || !named || arg.starts_with("--"))
            && arg.starts_with('-');
        if flags_allowed {
            match arg.as_str() {
                "-h" | "--help" if !named => {
                    print_run_help();
                    return Err(CliControl::success().into());
                }
                "--force" => parsed.force = true,
                "--app-port" => {
                    parsed.app_port = Some(parse_port(
                        args.get(index + 1),
                        "--app-port requires a port number",
                    )?);
                    index += 1;
                }
                "--name" if !named => {
                    parsed.name = Some(
                        args.get(index + 1)
                            .filter(|name| !name.starts_with('-'))
                            .cloned()
                            .ok_or_else(|| anyhow!("--name requires a name value"))?,
                    );
                    index += 1;
                }
                "--tailscale" => parsed.sharing.tailscale = true,
                "--funnel" => {
                    parsed.sharing.funnel = true;
                    parsed.sharing.tailscale = true;
                }
                "--ngrok" => parsed.sharing.ngrok = true,
                _ => bail!("Unknown flag \"{arg}\""),
            }
            index += 1;
            continue;
        }
        parsed.command.extend_from_slice(&args[index..]);
        break;
    }
    if parsed.app_port.is_none() {
        parsed.app_port = env::var("PORTLESS_APP_PORT")
            .ok()
            .map(|value| parse_port(Some(&value), "invalid PORTLESS_APP_PORT"))
            .transpose()?;
    }
    if named && parsed.name.is_none() {
        bail!("No app name provided");
    }
    Ok(parsed)
}

fn parse_port(value: Option<&String>, missing: &str) -> Result<u16> {
    value
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| anyhow!("{missing}"))?
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| anyhow!("port must be between 1 and 65535"))
}

#[derive(Debug)]
struct CliControl(i32);

impl CliControl {
    const fn success() -> Self {
        Self(0)
    }
}

impl std::fmt::Display for CliControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "command completed with status {}", self.0)
    }
}

impl std::error::Error for CliControl {}

async fn run_bypassed(mut command: Vec<String>, script: Option<&str>) -> Result<i32> {
    if command.is_empty() {
        let cwd = env::current_dir()?;
        let config = load_app_config(&cwd)?;
        let script = script
            .or_else(|| config.as_ref().and_then(|config| config.script.as_deref()))
            .unwrap_or("dev");
        command = config::resolve_script_command(script, &cwd)
            .ok_or_else(|| anyhow!("No command provided"))?;
    }
    run_direct_command(&command, &env::current_dir()?, None).await
}

async fn dispatch_app(mut parsed: ParsedApp, inferred: bool, script: Option<&str>) -> Result<i32> {
    let cwd = env::current_dir()?;
    let app_config = load_app_config(&cwd)?;
    if parsed.command.is_empty() {
        let script = script
            .or_else(|| {
                app_config
                    .as_ref()
                    .and_then(|config| config.script.as_deref())
            })
            .unwrap_or("dev");
        parsed.command = config::resolve_script_command(script, &cwd)
            .ok_or_else(|| anyhow!("No command provided"))?;
    }
    if parsed.app_port.is_none() {
        parsed.app_port = app_config.as_ref().and_then(|config| config.app_port);
    }
    if !single_app_proxy_enabled(app_config.as_ref()) {
        return run_direct_command(&parsed.command, &cwd, None).await;
    }

    let (base_name, source) = if let Some(name) = &parsed.name {
        (normalize_name(name), "--name flag")
    } else if let Some(name) = app_config
        .as_ref()
        .and_then(|config| config.name.as_deref())
    {
        (normalize_name(name), "portless.json")
    } else {
        let name = infer_project_name(&cwd)?;
        (name.name, name.source)
    };
    let worktree = inferred.then(|| detect_worktree_prefix(&cwd)).flatten();
    let name = worktree.as_ref().map_or_else(
        || base_name.clone(),
        |prefix| format!("{}.{}", prefix.prefix, base_name),
    );
    run_app(name, parsed, source, worktree.as_ref(), &cwd).await
}

fn single_app_proxy_enabled(config: Option<&AppConfig>) -> bool {
    config.and_then(|config| config.proxy) != Some(false)
}

fn normalize_name(name: &str) -> String {
    name.split('.')
        .map(truncate_label)
        .collect::<Vec<_>>()
        .join(".")
}

fn load_app_config(cwd: &Path) -> Result<Option<AppConfig>> {
    let Some(loaded) = config::load_config(cwd)? else {
        return Ok(None);
    };
    Ok(Some(config::resolve_app_config(
        &loaded.config,
        loaded.config_dir,
        cwd,
    )))
}

async fn run_app(
    name: String,
    mut parsed: ParsedApp,
    source: &str,
    worktree: Option<&crate::auto::WorktreePrefix>,
    cwd: &Path,
) -> Result<i32> {
    println!("\nportless\n");
    let _ = build_hostnames(&name, &[DEFAULT_TLD.to_owned()])?;
    let share = sharing_from_env(parsed.sharing.clone());
    let tailscale_ready = if share.tailscale {
        Some(tailscale::ensure_tailscale_ready(
            EnsureTailscaleReadyOptions {
                require_funnel: share.funnel,
                require_https: true,
                runner: None,
            },
        )?)
    } else {
        None
    };
    if share.ngrok {
        ngrok::ensure_ngrok_available()?;
    }

    let mut state = discover_state().await;
    if !is_proxy_running(state.port).await {
        state = ensure_proxy_running(&state).await?;
    } else {
        let desired = proxy_config_from_env()?;
        if let Some(mismatch) = active_proxy_mismatch(&desired, &state) {
            bail!(
                "Proxy is already running with different {mismatch}. Stop it first with: portless \
                 proxy stop"
            );
        }
        println!("-- Proxy is running");
    }

    let hostnames = build_hostnames(&name, &state.tlds)?;
    let hostname = hostnames
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("no hostname generated"))?;
    println!("-- {} (auto-resolves to 127.0.0.1)", hostnames.join(", "));
    println!(
        "-- Name \"{}\" (from {source})",
        name.rsplit('.').next().unwrap_or(&name)
    );
    if let Some(worktree) = worktree {
        println!(
            "-- Prefix \"{}\" (from {})",
            worktree.prefix, worktree.source
        );
    }

    let app_port = match parsed.app_port {
        Some(port) => port,
        None => process::find_free_port().await?,
    };
    println!(
        "-- Using port {app_port}{}",
        if parsed.app_port.is_some() {
            " (fixed)"
        } else {
            ""
        }
    );
    let executable = env::current_exe()?;
    let store = RouteStore::with_warning(&state.dir, |warning| eprintln!("Warning: {warning}"));
    let owner = i32::try_from(std::process::id()).context("process ID is too large")?;
    let killed = add_routes(&store, &hostnames, app_port, owner, parsed.force)?;
    if !killed.is_empty() {
        println!(
            "Killed existing process(es): {}",
            killed
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let url = format_url(&hostname, state.port, state.tls);
    println!("\n  -> {url}\n");
    for extra in hostnames.iter().skip(1) {
        println!("  also -> {}", format_url(extra, state.port, state.tls));
    }

    let mut tailscale_port = None;
    let mut ngrok_process = None;
    let mut tailscale_url = None;
    let mut ngrok_url = None;

    if share.tailscale {
        for attempt in 0..3 {
            let mode = if share.funnel {
                TailscaleMode::Funnel
            } else {
                TailscaleMode::Serve
            };
            let port =
                tailscale::find_available_serve_port(&tailscale::get_used_serve_ports(), mode)?;
            let result = if share.funnel {
                tailscale::register_funnel(app_port, port)
            } else {
                tailscale::register_serve(app_port, port)
            };
            match result {
                Ok(()) => {
                    tailscale_port = Some(port);
                    break;
                }
                Err(error) if attempt < 2 && error.to_string().contains("already in use") => {}
                Err(error) => {
                    if !error.to_string().contains("already in use") {
                        tailscale::unregister_tailscale(TailscaleRoute {
                            tailscale_https_port: Some(port),
                            tailscale_funnel: share.funnel,
                        });
                    }
                    remove_routes(&store, &hostnames, Some(owner));
                    return Err(error.into());
                }
            }
        }
        if let (Some(port), Some(ready)) = (tailscale_port, tailscale_ready.as_ref()) {
            let shared = tailscale::format_tailscale_url(&ready.base_url, port);
            println!(
                "  {} -> {shared}",
                if share.funnel {
                    "Funnel (public)"
                } else {
                    "Tailscale"
                }
            );
            tailscale_url = Some(shared.clone());
            let _ = store.update_route(
                &hostname,
                RouteMetadata {
                    tailscale_url: Some(Some(shared)),
                    tailscale_https_port: Some(Some(port)),
                    tailscale_funnel: Some(Some(share.funnel)),
                    ..RouteMetadata::default()
                },
            );
        }
    }

    if share.ngrok {
        let exit_store = RouteStore::new(&state.dir);
        let exit_hostname = hostname.clone();
        let ngrok_exited = Arc::new(Mutex::new(false));
        let exit_state = Arc::clone(&ngrok_exited);
        let started = match ngrok::start_ngrok_with_options(
            app_port,
            ngrok::StartNgrokOptions {
                host_header: Some(&hostname),
                on_exit: Some(Arc::new(move |_code, _signal| {
                    clear_ngrok_metadata_on_exit(&exit_store, &exit_hostname, &exit_state);
                })),
                ..ngrok::StartNgrokOptions::default()
            },
        )
        .await
        {
            Ok(started) => started,
            Err(error) => {
                tailscale::unregister_tailscale(TailscaleRoute {
                    tailscale_https_port: tailscale_port,
                    tailscale_funnel: share.funnel,
                });
                remove_routes(&store, &hostnames, Some(owner));
                return Err(error.into());
            }
        };
        println!("  ngrok -> {}", started.url);
        ngrok_url = Some(started.url.clone());
        let ngrok_pid = started.pid.and_then(|pid| i32::try_from(pid).ok());
        let metadata_error = match commit_ngrok_metadata(
            &store,
            &hostname,
            &ngrok_exited,
            &started.url,
            ngrok_pid,
        ) {
            Ok(true) => None,
            Ok(false) => Some(anyhow!("ngrok exited before startup completed")),
            Err(error) => Some(error),
        };
        if let Some(error) = metadata_error {
            ngrok::stop_ngrok_process(Some(&started.process));
            tailscale::unregister_tailscale(TailscaleRoute {
                tailscale_https_port: tailscale_port,
                tailscale_funnel: share.funnel,
            });
            remove_routes(&store, &hostnames, Some(owner));
            return Err(error);
        }
        ngrok_process = Some(started.process);
    }

    process::inject_framework_flags_with_lan_mode(&mut parsed.command, app_port, state.lan_mode());
    let inherited: ChildEnv = env::vars_os().collect();
    let ca = generated_ca_for_child(&state, &inherited);
    let child_env = process::build_portless_child_env(
        &inherited,
        &PortlessChildEnv {
            port: app_port,
            host: expo_host(&parsed.command, state.lan_mode()),
            portless_url: &url,
            vite_allowed_hosts: &format_allowed_hosts(&state.tlds),
            lan_mode: state.lan_mode(),
            tailscale_url: tailscale_url.as_deref(),
            ngrok_url: ngrok_url.as_deref(),
            node_extra_ca_certs: ca.as_deref(),
        },
        cwd,
        &executable,
    );
    println!(
        "Running: PORT={app_port} PORTLESS_URL={url} {}\n",
        parsed.command.join(" ")
    );
    let status = run_child_with_signals(&parsed.command, cwd, child_env).await;

    ngrok::stop_ngrok_process(ngrok_process.as_ref());
    tailscale::unregister_tailscale(TailscaleRoute {
        tailscale_https_port: tailscale_port,
        tailscale_funnel: share.funnel,
    });
    remove_routes(&store, &hostnames, Some(owner));
    status
}

fn generated_ca_for_child(state: &State, inherited: &ChildEnv) -> Option<PathBuf> {
    (state.tls && !state.custom_cert)
        .then(|| state.dir.join(certs::CA_CERT_FILE))
        .filter(|path| path.exists())
        .filter(|_| !inherited.contains_key(OsStr::new("NODE_EXTRA_CA_CERTS")))
}

fn clear_ngrok_metadata_on_exit(store: &RouteStore, hostname: &str, lifecycle: &Mutex<bool>) {
    if let Ok(mut exited) = lifecycle.lock() {
        *exited = true;
        let _ = store.update_route(
            hostname,
            RouteMetadata {
                ngrok_url: Some(None),
                ngrok_pid: Some(None),
                ..RouteMetadata::default()
            },
        );
    }
}

fn commit_ngrok_metadata(
    store: &RouteStore,
    hostname: &str,
    lifecycle: &Mutex<bool>,
    url: &str,
    pid: Option<i32>,
) -> Result<bool> {
    let exited = lifecycle
        .lock()
        .map_err(|_| anyhow!("ngrok lifecycle lock is poisoned"))?;
    if *exited {
        return Ok(false);
    }
    store.update_route(
        hostname,
        RouteMetadata {
            ngrok_url: Some(Some(url.to_owned())),
            ngrok_pid: Some(pid),
            ..RouteMetadata::default()
        },
    )?;
    Ok(true)
}

fn sharing_from_env(mut sharing: Sharing) -> Sharing {
    sharing.funnel |= enabled(env::var_os("PORTLESS_FUNNEL").as_deref());
    sharing.tailscale |= sharing.funnel || enabled(env::var_os("PORTLESS_TAILSCALE").as_deref());
    sharing.ngrok |= enabled(env::var_os("PORTLESS_NGROK").as_deref());
    sharing
}

fn expo_host(command: &[String], lan: bool) -> Option<&'static str> {
    let executable = command
        .first()
        .and_then(|arg| Path::new(arg).file_name())
        .and_then(OsStr::to_str);
    if lan && executable == Some("expo") {
        None
    } else {
        Some("127.0.0.1")
    }
}

async fn run_child_with_signals(args: &[String], cwd: &Path, child_env: ChildEnv) -> Result<i32> {
    let mut command = process::shell_command(args, &child_env);
    command.current_dir(cwd);
    let mut child = tokio::process::Command::from(command).spawn()?;
    let pid = child.id();
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        tokio::select! {
            status = child.wait() => Ok(status_code(status?)),
            _ = interrupt.recv() => {
                if let Some(pid) = pid {
                    let _ = process::signal_process_tree(pid, Signal::SIGINT);
                }
                let _ = child.wait().await;
                Ok(process::signal_exit_code(Signal::SIGINT))
            }
            _ = terminate.recv() => {
                if let Some(pid) = pid {
                    let _ = process::signal_process_tree(pid, Signal::SIGTERM);
                }
                let _ = child.wait().await;
                Ok(process::signal_exit_code(Signal::SIGTERM))
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            status = child.wait() => Ok(status_code(status?)),
            _ = tokio::signal::ctrl_c() => {
                let _ = child.kill().await;
                Ok(130)
            }
        }
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal().map_or(1, |signal| 128 + signal)
        }
        #[cfg(not(unix))]
        {
            1
        }
    })
}

fn add_routes(
    store: &RouteStore,
    hostnames: &[String],
    port: u16,
    pid: i32,
    force: bool,
) -> Result<Vec<i32>> {
    let mut registered = Vec::new();
    let mut killed = BTreeSet::new();
    for hostname in hostnames {
        match store.add_route(hostname, port, pid, force) {
            Ok(previous) => {
                registered.push(hostname.clone());
                killed.extend(previous);
            }
            Err(error) => {
                remove_routes(store, &registered, Some(pid));
                return Err(error);
            }
        }
    }
    Ok(killed.into_iter().collect())
}

fn remove_routes(store: &RouteStore, hostnames: &[String], owner: Option<i32>) {
    for hostname in hostnames {
        let _ = store.remove_route(hostname, owner);
    }
}

fn build_hostnames(name: &str, tlds: &[String]) -> Result<Vec<String>> {
    let mut base = name
        .trim()
        .strip_prefix("http://")
        .or_else(|| name.trim().strip_prefix("https://"))
        .unwrap_or(name.trim())
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    for tld in tlds {
        validate_tld(tld)?;
        if let Some(stripped) = base.strip_suffix(&format!(".{tld}")) {
            base = stripped.to_owned();
            break;
        }
    }
    validate_name(&base)?;
    Ok(tlds.iter().map(|tld| format!("{base}.{tld}")).collect())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("hostname cannot be empty");
    }
    if name.contains("..") {
        bail!("Invalid hostname \"{name}\": consecutive dots are not allowed");
    }
    for label in name.split('.') {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            bail!("Invalid hostname label \"{label}\"");
        }
    }
    Ok(())
}

fn validate_tld(tld: &str) -> Result<()> {
    if tld.is_empty() {
        bail!("TLD cannot be empty");
    }
    if !tld
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("Invalid TLD \"{tld}\": must contain only lowercase letters and digits");
    }
    Ok(())
}

fn parse_tlds(value: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    for value in value.split(',') {
        let value = value.trim().to_ascii_lowercase();
        validate_tld(&value)?;
        if !values.contains(&value) {
            values.push(value);
        }
    }
    if values.is_empty() {
        bail!("TLD list cannot be empty");
    }
    Ok(values)
}

fn risky_tld_reason(tld: &str) -> Option<&'static str> {
    match tld {
        "local" => Some("conflicts with mDNS/Bonjour on macOS"),
        "dev" => Some("is Google-owned; browsers force HTTPS via preloaded HSTS"),
        "com" | "org" | "net" | "io" | "app" | "edu" | "gov" | "mil" | "int" => {
            Some("is a public TLD; DNS requests will leak to the internet")
        }
        _ => None,
    }
}

fn env_tlds() -> Result<Vec<String>> {
    env::var("PORTLESS_TLD")
        .ok()
        .map(|value| parse_tlds(&value))
        .transpose()
        .map(|value| value.unwrap_or_else(|| vec![DEFAULT_TLD.to_owned()]))
}

fn format_allowed_hosts(tlds: &[String]) -> String {
    tlds.iter()
        .map(|tld| format!(".{tld}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_url(hostname: &str, port: u16, tls: bool) -> String {
    let protocol = if tls { "https" } else { "http" };
    let default = if tls { 443 } else { 80 };
    if port == default {
        format!("{protocol}://{hostname}")
    } else {
        format!("{protocol}://{hostname}:{port}")
    }
}

fn user_state_dir() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(".portless")
}

fn resolve_state_dir() -> PathBuf {
    env::var_os("PORTLESS_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(user_state_dir)
}

fn read_port(dir: &Path) -> Option<u16> {
    fs::read_to_string(dir.join("proxy.port"))
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn state_from_dir(dir: PathBuf, port: u16, infer_tls: bool) -> State {
    let tls = dir.join("proxy.tls").exists() || infer_tls;
    let custom_cert = dir.join("proxy.custom-cert").exists();
    let tlds = read_tlds(&dir).unwrap_or_else(|| vec![DEFAULT_TLD.into()]);
    let lan_ip = fs::read_to_string(dir.join("proxy.lan"))
        .ok()
        .and_then(|value| value.trim().parse().ok());
    State {
        dir,
        port,
        tls,
        tlds,
        lan_ip,
        custom_cert,
    }
}

async fn discover_state() -> State {
    if env::var_os("PORTLESS_STATE_DIR").is_some() {
        let dir = resolve_state_dir();
        let port = read_port(&dir).unwrap_or_else(configured_legacy_port);
        return state_from_dir(dir, port, false);
    }

    let user_dir = user_state_dir();
    if let Some(port) = read_port(&user_dir)
        && is_proxy_running(port).await
    {
        return state_from_dir(user_dir, port, false);
    }

    let legacy_dir = env::temp_dir().join("portless");
    if let Some(port) = read_port(&legacy_dir)
        && is_proxy_running(port).await
    {
        return state_from_dir(legacy_dir, port, false);
    }

    let configured = configured_legacy_port();
    for port in active_probe_ports(configured) {
        if is_proxy_running(port).await {
            return state_from_dir(user_state_dir(), port, port == 443);
        }
    }

    let persisted_port = read_port(&user_dir).unwrap_or(configured);
    state_from_dir(user_dir, persisted_port, false)
}

fn active_probe_ports(configured: u16) -> Vec<u16> {
    let mut ports = Vec::new();
    for port in [443, 80, FALLBACK_PROXY_PORT, configured] {
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports
}

fn configured_legacy_port() -> u16 {
    env::var("PORTLESS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(FALLBACK_PROXY_PORT)
}

fn read_tlds(dir: &Path) -> Option<Vec<String>> {
    fs::read_to_string(dir.join("proxy.tlds"))
        .ok()
        .and_then(|value| {
            let raw = value.trim();
            let values = if raw.starts_with('[') {
                serde_json::from_str::<Vec<String>>(raw).ok()?
            } else {
                raw.lines()
                    .flat_map(|line| line.split(','))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect()
            };
            let mut parsed = Vec::new();
            for value in values {
                parsed.extend(parse_tlds(&value).ok()?);
            }
            (!parsed.is_empty()).then(|| dedupe(parsed))
        })
        .or_else(|| {
            fs::read_to_string(dir.join("proxy.tld"))
                .ok()
                .map(|value| vec![value.trim().to_owned()])
                .filter(|values| values.first().is_some_and(|value| !value.is_empty()))
        })
}

fn write_marker(path: &Path, value: Option<&str>) -> io::Result<()> {
    if let Some(value) = value {
        fs::write(path, value)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn write_proxy_state(state: &State) -> Result<()> {
    fs::create_dir_all(&state.dir)?;
    fs::write(state.dir.join("proxy.pid"), std::process::id().to_string())?;
    fs::write(state.dir.join("proxy.port"), state.port.to_string())?;
    write_marker(&state.dir.join("proxy.tls"), state.tls.then_some("1"))?;
    write_marker(
        &state.dir.join("proxy.custom-cert"),
        state.custom_cert.then_some("1"),
    )?;
    if state.tlds == [DEFAULT_TLD] {
        write_marker(&state.dir.join("proxy.tlds"), None)?;
        write_marker(&state.dir.join("proxy.tld"), None)?;
    } else {
        fs::write(
            state.dir.join("proxy.tlds"),
            format!("{}\n", state.tlds.join("\n")),
        )?;
        fs::write(state.dir.join("proxy.tld"), state.primary_tld())?;
    }
    write_marker(
        &state.dir.join("proxy.lan"),
        state.lan_ip.map(|ip| ip.to_string()).as_deref(),
    )?;
    Ok(())
}

fn clear_proxy_runtime_state(dir: &Path) {
    for file in [
        "proxy.pid",
        "proxy.port",
        "proxy.tls",
        "proxy.custom-cert",
        "proxy.tld",
        "proxy.tlds",
        "proxy.lan",
    ] {
        let _ = fs::remove_file(dir.join(file));
    }
}

async fn is_proxy_running(port: u16) -> bool {
    let request = "GET / HTTP/1.1\r\nHost: __portless_health__.localhost\r\nConnection: \
                   close\r\n\r\n"
        .to_owned();
    tokio::time::timeout(Duration::from_millis(500), async move {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut bytes = vec![0; 1024];
        let read = stream.read(&mut bytes).await?;
        Ok::<_, io::Error>(
            String::from_utf8_lossy(&bytes[..read])
                .to_ascii_lowercase()
                .contains("x-portless: 1"),
        )
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false)
}

fn is_port_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(300),
    )
    .is_ok()
}

async fn wait_for_proxy(port: u16) -> bool {
    for _ in 0..WAIT_ATTEMPTS {
        if is_proxy_running(port).await {
            return true;
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }
    false
}

async fn ensure_proxy_running(desired: &State) -> Result<State> {
    let mut config = proxy_config_from_env()?;
    merge_persisted_proxy_config(&mut config, desired);
    if config.port < PRIVILEGED_PORT_THRESHOLD && !is_elevated() && !stdin_is_terminal() {
        bail!(
            "Proxy is not running and no TTY is available for sudo.\nStart it in a terminal: sudo \
             portless proxy start\nOr use: portless proxy start -p {FALLBACK_PROXY_PORT}"
        );
    }
    println!("Starting proxy...");
    let executable = env::current_exe()?;
    let mut args = vec!["proxy".into(), "start".into()];
    args.extend(proxy_args(&config, false));
    let status = Command::new(executable).args(args).status()?;
    if !status.success() || !wait_for_proxy(config.port).await {
        bail!("Failed to start proxy");
    }
    Ok(discover_state().await)
}

fn merge_persisted_proxy_config(config: &mut ProxyConfig, persisted: &State) {
    if !persisted.dir.join("proxy.port").exists() {
        return;
    }
    if !config.port_explicit {
        config.port = persisted.port;
    }
    if !config.tls_explicit {
        config.tls = persisted.tls;
    }
    if !config.lan_explicit {
        config.lan = persisted.lan_mode();
    }
    if !config.lan_ip_explicit {
        config.lan_ip = if config.lan { persisted.lan_ip } else { None };
    }
    if (config.lan || !config.lan_explicit) && !config.tlds_explicit {
        config.tlds = persisted.tlds.clone();
    }
    if config.lan {
        config.tlds = vec!["local".into()];
    } else {
        config.lan_ip = None;
    }
}

fn is_elevated() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::Uid::effective().is_root()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn stdin_is_terminal() -> bool {
    use std::io::IsTerminal as _;
    io::stdin().is_terminal() && env::var_os("CI").is_none()
}

fn proxy_config_from_env() -> Result<ProxyConfig> {
    let tls = !matches!(env::var("PORTLESS_HTTPS").as_deref(), Ok("0" | "false"));
    let lan =
        enabled(env::var_os("PORTLESS_LAN").as_deref()) || env::var_os("PORTLESS_LAN_IP").is_some();
    let lan_ip = env::var("PORTLESS_LAN_IP")
        .ok()
        .map(|value| value.parse::<Ipv4Addr>())
        .transpose()
        .context("PORTLESS_LAN_IP must be an IPv4 address")?;
    let tlds = if lan {
        vec!["local".into()]
    } else {
        env_tlds()?
    };
    Ok(ProxyConfig {
        port: env::var("PORTLESS_PORT")
            .ok()
            .map(|value| value.parse::<u16>())
            .transpose()
            .context("PORTLESS_PORT must be between 1 and 65535")?
            .unwrap_or(if tls { 443 } else { 80 }),
        port_explicit: env::var_os("PORTLESS_PORT").is_some(),
        tls,
        tls_explicit: env::var_os("PORTLESS_HTTPS").is_some(),
        foreground: false,
        skip_trust: false,
        cert: None,
        key: None,
        lan,
        lan_explicit: env::var_os("PORTLESS_LAN").is_some(),
        lan_ip,
        lan_ip_explicit: env::var_os("PORTLESS_LAN_IP").is_some(),
        tlds,
        tlds_explicit: env::var_os("PORTLESS_TLD").is_some(),
        wildcard: enabled(env::var_os("PORTLESS_WILDCARD").as_deref()),
    })
}

fn parse_proxy_start(args: &[String]) -> Result<ProxyConfig> {
    let mut config = proxy_config_from_env()?;
    let mut index = 2;
    let mut flag_tlds = Vec::new();
    while index < args.len() {
        match args[index].as_str() {
            "-p" | "--port" => {
                config.port = parse_port(args.get(index + 1), "--port requires a port number")?;
                config.port_explicit = true;
                index += 2;
            }
            "--https" => {
                config.tls = true;
                config.tls_explicit = true;
                index += 1;
            }
            "--no-tls" => {
                config.tls = false;
                config.tls_explicit = true;
                index += 1;
            }
            "--foreground" => {
                config.foreground = true;
                index += 1;
            }
            "--skip-trust" => {
                config.skip_trust = true;
                index += 1;
            }
            "--wildcard" => {
                config.wildcard = true;
                index += 1;
            }
            "--lan" => {
                config.lan = true;
                config.lan_explicit = true;
                index += 1;
            }
            "--ip" | INTERNAL_LAN_IP_FLAG => {
                let flag = &args[index];
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| anyhow!("{flag} requires an IP address"))?;
                config.lan_ip = Some(value.parse().context("invalid LAN IPv4 address")?);
                config.lan = true;
                config.lan_ip_explicit = flag == "--ip";
                index += 2;
            }
            "--cert" => {
                config.cert = Some(PathBuf::from(
                    args.get(index + 1)
                        .filter(|value| !value.starts_with('-'))
                        .ok_or_else(|| anyhow!("--cert requires a file path"))?,
                ));
                config.tls = true;
                config.tls_explicit = true;
                index += 2;
            }
            "--key" => {
                config.key = Some(PathBuf::from(
                    args.get(index + 1)
                        .filter(|value| !value.starts_with('-'))
                        .ok_or_else(|| anyhow!("--key requires a file path"))?,
                ));
                config.tls = true;
                config.tls_explicit = true;
                index += 2;
            }
            "--tld" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| anyhow!("--tld requires a TLD value"))?;
                flag_tlds.extend(parse_tlds(value)?);
                config.tlds_explicit = true;
                index += 2;
            }
            unknown => bail!("Unknown proxy start option \"{unknown}\""),
        }
    }
    if config.cert.is_some() != config.key.is_some() {
        bail!("--cert and --key must be used together");
    }
    if !flag_tlds.is_empty() {
        config.tlds = dedupe(flag_tlds);
    }
    if config.lan {
        config.tlds = vec!["local".into()];
    }
    if !config.port_explicit {
        config.port = if config.tls { 443 } else { 80 };
    }
    Ok(config)
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn proxy_args(config: &ProxyConfig, foreground: bool) -> Vec<String> {
    let mut args = vec![
        "--port".into(),
        config.port.to_string(),
        if config.tls { "--https" } else { "--no-tls" }.into(),
    ];
    if foreground {
        args.push("--foreground".into());
    }
    if config.skip_trust {
        args.push("--skip-trust".into());
    }
    if let (Some(cert), Some(key)) = (&config.cert, &config.key) {
        args.extend([
            "--cert".into(),
            cert.to_string_lossy().into_owned(),
            "--key".into(),
            key.to_string_lossy().into_owned(),
        ]);
    }
    if config.lan {
        args.push("--lan".into());
        if let Some(ip) = config.lan_ip {
            args.extend([
                if config.lan_ip_explicit {
                    "--ip"
                } else {
                    INTERNAL_LAN_IP_FLAG
                }
                .into(),
                ip.to_string(),
            ]);
        }
    } else {
        for tld in &config.tlds {
            args.extend(["--tld".into(), tld.clone()]);
        }
    }
    if config.wildcard {
        args.push("--wildcard".into());
    }
    args
}

async fn handle_proxy(args: &[String]) -> Result<i32> {
    match args.get(1).map(String::as_str) {
        Some("stop") => {
            let explicit = args
                .iter()
                .position(|arg| matches!(arg.as_str(), "-p" | "--port"))
                .map(|index| parse_port(args.get(index + 1), "--port requires a port number"))
                .transpose()?;
            stop_proxy(explicit).await?;
            Ok(0)
        }
        Some("start") => start_proxy(parse_proxy_start(args)?).await,
        None | Some("-h" | "--help") => {
            print_proxy_help();
            Ok(0)
        }
        Some(command) => {
            print_proxy_help();
            bail!("Unknown proxy subcommand \"{command}\"")
        }
    }
}

async fn start_proxy(mut config: ProxyConfig) -> Result<i32> {
    let discovered = discover_state().await;
    if is_proxy_running(discovered.port).await
        && (!config.port_explicit || config.port == discovered.port)
    {
        if let Some(mismatch) = active_proxy_mismatch(&config, &discovered) {
            bail!(
                "Proxy is already running with different {mismatch}. Stop it first with: portless \
                 proxy stop"
            );
        }
        println!("Proxy is already running on port {}.", discovered.port);
        return Ok(0);
    }
    merge_persisted_proxy_config(&mut config, &discovered);
    if is_proxy_running(config.port).await {
        println!("Proxy is already running on port {}.", config.port);
        return Ok(0);
    }

    if config.lan && config.lan_ip.is_none() {
        let support = mdns::is_mdns_supported();
        if !support.supported {
            bail!(
                "LAN mode requires mDNS publishing: {}",
                support
                    .reason
                    .unwrap_or_else(|| "unsupported platform".into())
            );
        }
        config.lan_ip = mdns::get_local_network_ip().await;
        if config.lan_ip.is_none() {
            bail!("Could not detect LAN IP. Specify --ip 192.168.1.42");
        }
    }
    for tld in &config.tlds {
        if let Some(reason) = risky_tld_reason(tld) {
            eprintln!("Warning: .{tld} {reason}.");
        }
    }

    if config.port < PRIVILEGED_PORT_THRESHOLD && !is_elevated() {
        println!(
            "Port {} requires elevated privileges. Requesting sudo...",
            config.port
        );
        let executable = env::current_exe()?;
        let mut sudo_args = collect_portless_env_args();
        sudo_args.push(executable.to_string_lossy().into_owned());
        sudo_args.extend(["proxy".into(), "start".into()]);
        sudo_args.extend(proxy_args(&config, config.foreground));
        let status = Command::new("sudo")
            .arg("env")
            .args(sudo_args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return Ok(0);
        }
        if config.port_explicit {
            bail!(
                "Port {} requires elevated privileges and sudo failed",
                config.port
            );
        }
        config.port = FALLBACK_PROXY_PORT;
        println!("Falling back to port {}.", config.port);
    }

    let state = State {
        dir: resolve_state_dir(),
        port: config.port,
        tls: config.tls,
        tlds: config.tlds.clone(),
        lan_ip: config.lan_ip,
        custom_cert: config.cert.is_some(),
    };
    if config.foreground {
        run_proxy_foreground(state, &config).await?;
        return Ok(0);
    }

    fs::create_dir_all(&state.dir)?;
    let log_path = state.dir.join("proxy.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = log.try_clone()?;
    let executable = env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .args(["proxy", "start"])
        .args(proxy_args(&config, true))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    process::configure_process_group(&mut command);
    command.spawn()?;
    if !wait_for_proxy(config.port).await {
        bail!(
            "Proxy failed to start (timed out). Logs: {}",
            log_path.display()
        );
    }
    println!(
        "{} proxy started on port {}",
        if config.tls { "HTTPS/2" } else { "HTTP" },
        config.port
    );
    Ok(0)
}

fn active_proxy_mismatch(config: &ProxyConfig, active: &State) -> Option<&'static str> {
    if config.port_explicit && config.port != active.port {
        return Some("port");
    }
    if config.tls_explicit && config.tls != active.tls {
        return Some("TLS mode");
    }
    if config.tlds_explicit && config.tlds != active.tlds {
        return Some("TLD configuration");
    }
    if config.lan_explicit && config.lan != active.lan_mode() {
        return Some("LAN mode");
    }
    if config.lan_ip_explicit && config.lan_ip != active.lan_ip {
        return Some("LAN IP");
    }
    if config.cert.is_some() && !active.custom_cert {
        return Some("certificate mode");
    }
    None
}

async fn run_proxy_foreground(state: State, config: &ProxyConfig) -> Result<()> {
    let store = Arc::new(RouteStore::with_warning(&state.dir, |warning| {
        eprintln!("Warning: {warning}");
    }));
    store.ensure_dir()?;
    if !store.routes_path().exists() {
        fs::write(store.routes_path(), "[]")?;
    }
    let routes = Arc::clone(&store);
    let mut options = ProxyOptions::new(state.port, move || routes.load_routes());
    options.tld = state.primary_tld().to_owned();
    options.tlds = state.tlds.clone();
    options.strict = !config.wildcard;
    if state.lan_mode() {
        options.bind_address = None;
    }
    if state.tls {
        let (cert, key, ca) = if let (Some(cert), Some(key)) = (&config.cert, &config.key) {
            (fs::read(cert)?, fs::read(key)?, None)
        } else {
            let generated = certs::ensure_certs(&state.dir)?;
            if !config.skip_trust && !certs::is_ca_trusted(&state.dir) {
                let trust = certs::trust_ca(&state.dir);
                if !trust.trusted {
                    eprintln!(
                        "Warning: could not trust local CA: {}",
                        trust.error.unwrap_or_else(|| "unknown error".into())
                    );
                }
            }
            (
                fs::read(generated.cert_path)?,
                fs::read(generated.key_path)?,
                Some(fs::read(generated.ca_path)?),
            )
        };
        options.tls = Some(TlsConfig { cert, key, ca });
        if generated_sni_enabled(config) {
            options.sni_state_dir = Some(state.dir.clone());
        }
    }

    let server = ProxyServer::bind(options).await?;
    write_proxy_state(&state)?;
    let auto_sync_hosts =
        hosts::should_auto_sync_hosts(env::var("PORTLESS_SYNC_HOSTS").ok().as_deref());
    let active_lan_ip = Arc::new(Mutex::new(state.lan_ip));
    sync_route_integrations(&store, state.port, state.lan_ip, auto_sync_hosts);
    let route_watch = start_route_watch(
        Arc::clone(&store),
        state.port,
        Arc::clone(&active_lan_ip),
        auto_sync_hosts,
    );
    let lan_monitor = if state.lan_mode() && !config.lan_ip_explicit {
        let monitor_store = Arc::clone(&store);
        let monitor_ip = Arc::clone(&active_lan_ip);
        let state_dir = state.dir.clone();
        Some(mdns::start_lan_ip_monitor(mdns::LanIpMonitorOptions {
            initial_ip: state.lan_ip,
            interval: Duration::from_secs(3),
            resolver: Arc::new(mdns::SystemLanIpResolver),
            on_change: Arc::new(move |next, _previous| {
                if let Ok(mut current) = monitor_ip.lock() {
                    *current = next;
                }
                let value = next.map(|ip| ip.to_string());
                let _ = write_marker(&state_dir.join("proxy.lan"), value.as_deref());
                mdns::cleanup_all();
                sync_route_integrations(&monitor_store, state.port, next, auto_sync_hosts);
            }),
            on_error: Some(Arc::new(|error| {
                eprintln!("Warning: LAN IP monitor failed: {error}");
            })),
        }))
    } else {
        None
    };
    println!(
        "{} proxy listening on port {}",
        if state.tls { "HTTPS/2" } else { "HTTP" },
        state.port
    );
    println!("\nProxy is running. Press Ctrl+C to stop.\n");
    server
        .run_until(async {
            #[cfg(unix)]
            {
                let terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
                let interrupt =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
                if let (Ok(mut terminate), Ok(mut interrupt)) = (terminate, interrupt) {
                    tokio::select! {
                        _ = terminate.recv() => {}
                        _ = interrupt.recv() => {}
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    route_watch.abort();
    if let Some(monitor) = lan_monitor {
        monitor.stop();
    }
    mdns::cleanup_all();
    clear_proxy_runtime_state(&state.dir);
    if auto_sync_hosts {
        let _ = hosts::clean_hosts_file();
    }
    Ok(())
}

fn generated_sni_enabled(config: &ProxyConfig) -> bool {
    config.tls && config.cert.is_none() && config.key.is_none()
}

fn sync_route_integrations(
    store: &RouteStore,
    proxy_port: u16,
    lan_ip: Option<Ipv4Addr>,
    auto_sync_hosts: bool,
) {
    let hostnames = store
        .load_routes()
        .into_iter()
        .map(|route| route.hostname)
        .collect::<Vec<_>>();
    if auto_sync_hosts {
        let _ = hosts::sync_hosts_file(&hostnames);
    }
    let desired = hostnames.iter().cloned().collect::<BTreeSet<_>>();
    for hostname in mdns::get_published() {
        if !desired.contains(&hostname) || lan_ip.is_none() {
            mdns::unpublish(&hostname);
        }
    }
    if let Some(ip) = lan_ip {
        for hostname in hostnames {
            let _ = mdns::publish(&hostname, proxy_port, ip, None);
        }
    }
}

fn route_file_signature(path: &Path) -> Option<(SystemTime, u64, u64)> {
    let metadata = fs::metadata(path).ok()?;
    let contents = fs::read(path).ok()?;
    let fingerprint = contents
        .iter()
        .fold(14_695_981_039_346_656_037, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
        });
    Some((metadata.modified().ok()?, metadata.len(), fingerprint))
}

fn start_route_watch(
    store: Arc<RouteStore>,
    proxy_port: u16,
    lan_ip: Arc<Mutex<Option<Ipv4Addr>>>,
    auto_sync_hosts: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let path = store.routes_path();
        let mut signature = route_file_signature(path);
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let next = route_file_signature(path);
            if next == signature {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            let current_ip = lan_ip.lock().ok().and_then(|value| *value);
            sync_route_integrations(&store, proxy_port, current_ip, auto_sync_hosts);
            signature = route_file_signature(path);
        }
    })
}

async fn stop_proxy(explicit_port: Option<u16>) -> Result<()> {
    let mut state = discover_state().await;
    let persisted_port = read_port(&state.dir);
    if let Some(port) = explicit_port {
        state.port = port;
    }
    let pid_path = state.dir.join("proxy.pid");
    let pid_contents = (explicit_port.is_none() || persisted_port == Some(state.port))
        .then(|| fs::read_to_string(&pid_path).ok())
        .flatten();
    let pid = pid_contents
        .as_deref()
        .and_then(|value| value.trim().parse::<i32>().ok());
    if pid_contents.is_some() && pid.is_none() {
        clear_proxy_runtime_state(&state.dir);
        println!("Corrupted PID file removed.");
        return Ok(());
    }
    if let Some(pid) = pid {
        if !pid_alive(pid) {
            clear_proxy_runtime_state(&state.dir);
            println!("Proxy process is no longer running. Cleaned stale files.");
            return Ok(());
        }
        if !is_proxy_running(state.port).await {
            clear_proxy_runtime_state(&state.dir);
            println!(
                "PID file exists but port {} is not running Portless. Removed stale state without \
                 signaling PID {pid}.",
                state.port
            );
            return Ok(());
        }
        match terminate_pid(pid, false) {
            Ok(()) => {
                clear_proxy_runtime_state(&state.dir);
                println!("Proxy stopped.");
                return Ok(());
            }
            Err(error) if is_permission_denied(&error) && !is_elevated() => {
                #[cfg(unix)]
                {
                    let executable = env::current_exe()?;
                    let status = Command::new("sudo")
                        .arg("env")
                        .args(collect_portless_env_args())
                        .arg(executable)
                        .args(["proxy", "stop", "--port", &state.port.to_string()])
                        .status()?;
                    if status.success() {
                        return Ok(());
                    }
                    bail!("Failed to stop proxy with sudo");
                }
                #[cfg(not(unix))]
                {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    if is_proxy_running(state.port).await {
        if let Some(pid) = pids_on_port(state.port).into_iter().next() {
            terminate_pid(pid, false)?;
            clear_proxy_runtime_state(&state.dir);
            println!("Killed process {pid}. Proxy stopped.");
            return Ok(());
        }
        bail!(
            "PID file is missing and Portless is responding on port {}, but the listener PID \
             could not be identified safely",
            state.port
        );
    }
    if is_port_listening(state.port) {
        println!(
            "Proxy is not running; port {} belongs to another service and was left untouched.",
            state.port
        );
        return Ok(());
    }
    println!("Proxy is not running.");
    Ok(())
}

#[cfg(unix)]
fn terminate_pid(pid: i32, force: bool) -> Result<()> {
    let signal = if force {
        Signal::SIGKILL
    } else {
        Signal::SIGTERM
    };
    kill(Pid::from_raw(pid), signal).map_err(Into::into)
}

#[cfg(windows)]
fn terminate_pid(pid: i32, force: bool) -> Result<()> {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("taskkill failed for PID {pid}")
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate_pid(_pid: i32, _force: bool) -> Result<()> {
    bail!("process signaling is not supported on this platform")
}

fn is_permission_denied(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
        || {
            #[cfg(unix)]
            {
                error
                    .downcast_ref::<nix::errno::Errno>()
                    .is_some_and(|error| *error == nix::errno::Errno::EPERM)
            }
            #[cfg(not(unix))]
            {
                false
            }
        }
}

fn collect_portless_env_args() -> Vec<String> {
    env::vars()
        .filter(|(key, value)| key.starts_with("PORTLESS_") && !value.is_empty())
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

async fn handle_get(args: &[String]) -> Result<i32> {
    if args
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: portless get <name> [--no-worktree]");
        return Ok(0);
    }
    let skip = args.iter().any(|arg| arg == "--no-worktree");
    let name = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .ok_or_else(|| anyhow!("Missing service name"))?;
    let cwd = env::current_dir()?;
    let name = if skip {
        name.clone()
    } else {
        detect_worktree_prefix(cwd).map_or_else(
            || name.clone(),
            |prefix| format!("{}.{}", prefix.prefix, name),
        )
    };
    let state = discover_state().await;
    let hostname = build_hostnames(&name, &state.tlds)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no hostname generated"))?;
    println!("{}", format_url(&hostname, state.port, state.tls));
    Ok(0)
}

async fn handle_alias(args: &[String]) -> Result<i32> {
    if args
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!(
            "Usage:\n  portless alias <name> <port> [--force]\n  portless alias --remove <name>"
        );
        return Ok(0);
    }
    let state = discover_state().await;
    let store = RouteStore::new(&state.dir);
    if args.get(1).is_some_and(|arg| arg == "--remove") {
        let name = args
            .get(2)
            .ok_or_else(|| anyhow!("No alias name provided"))?;
        let hostnames = build_hostnames(name, &state.tlds)?;
        let exists = store
            .load_routes()
            .iter()
            .any(|route| route.pid == 0 && hostnames.contains(&route.hostname));
        if !exists {
            bail!("No alias found for \"{}\"", hostnames.join(", "));
        }
        remove_routes(&store, &hostnames, None);
        println!("Removed alias: {}", hostnames.join(", "));
        return Ok(0);
    }
    let name = args.get(1).ok_or_else(|| anyhow!("Missing alias name"))?;
    let port = parse_port(args.get(2), "Missing alias port")?;
    let hostnames = build_hostnames(name, &state.tlds)?;
    add_routes(
        &store,
        &hostnames,
        port,
        0,
        args.iter().any(|arg| arg == "--force"),
    )?;
    println!(
        "Alias registered: {} -> 127.0.0.1:{port}",
        hostnames.join(", ")
    );
    Ok(0)
}

async fn handle_list() -> Result<i32> {
    let state = discover_state().await;
    let routes = RouteStore::new(&state.dir).load_routes();
    if routes.is_empty() {
        println!("No active routes.");
        return Ok(0);
    }
    println!("\nActive routes:\n");
    for route in routes {
        let owner = if route.pid == 0 {
            "(alias)".to_owned()
        } else {
            format!("(pid {})", route.pid)
        };
        println!(
            "  {}  ->  localhost:{}  {owner}",
            format_url(&route.hostname, state.port, state.tls),
            route.port
        );
        if let Some(url) = route.tailscale_url {
            println!(
                "    {}: {url}",
                if route.tailscale_funnel.unwrap_or(false) {
                    "funnel"
                } else {
                    "tailscale"
                }
            );
        }
        if let Some(url) = route.ngrok_url {
            println!("    ngrok: {url}");
        }
    }
    Ok(0)
}

async fn handle_hosts(args: &[String]) -> Result<i32> {
    match args.get(1).map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("Usage:\n  portless hosts sync\n  portless hosts clean");
            Ok(0)
        }
        Some("clean") => {
            if hosts::clean_hosts_file() {
                println!("Removed portless entries from hosts file.");
                Ok(0)
            } else {
                elevate_hosts("clean")
            }
        }
        Some("sync") => {
            let state = discover_state().await;
            let names = RouteStore::new(state.dir)
                .load_routes()
                .into_iter()
                .map(|route| route.hostname)
                .collect::<Vec<_>>();
            if names.is_empty() {
                println!("No active routes to sync.");
                return Ok(0);
            }
            if hosts::sync_hosts_file(&names) {
                println!("Synced {} hostname(s) to hosts file:", names.len());
                for name in names {
                    println!("  127.0.0.1 {name}");
                }
                Ok(0)
            } else {
                elevate_hosts("sync")
            }
        }
        Some(command) => bail!("Unknown hosts subcommand \"{command}\""),
    }
}

fn elevate_hosts(command: &str) -> Result<i32> {
    if cfg!(unix) && !is_elevated() {
        let executable = env::current_exe()?;
        let status = Command::new("sudo")
            .arg("env")
            .args(collect_portless_env_args())
            .arg(executable)
            .args(["hosts", command])
            .status()?;
        if status.success() {
            return Ok(0);
        }
    }
    bail!("Failed to update hosts file")
}

async fn handle_trust() -> Result<i32> {
    let state = discover_state().await;
    let certs = certs::ensure_certs(&state.dir)?;
    if certs.ca_generated {
        println!("Generated local CA certificate.");
    }
    let result = certs::trust_ca(&state.dir);
    if result.trusted {
        println!("Local CA added to system trust store.");
        return Ok(0);
    }
    let error = result.error.unwrap_or_else(|| "unknown error".into());
    if !is_elevated()
        && cfg!(unix)
        && (error.contains("Permission denied") || error.contains("EACCES"))
    {
        let executable = env::current_exe()?;
        let status = Command::new("sudo")
            .arg("env")
            .args(collect_portless_env_args())
            .arg(format!("PORTLESS_STATE_DIR={}", state.dir.display()))
            .arg(executable)
            .arg("trust")
            .status()?;
        if status.success() {
            return Ok(0);
        }
    }
    bail!("Failed to trust CA: {error}")
}

async fn handle_clean(args: &[String]) -> Result<i32> {
    if args
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: portless clean");
        return Ok(0);
    }
    if let Some(argument) = args.get(1) {
        bail!("Unknown argument \"{argument}\"");
    }
    let executable = env::current_exe()?;
    if let Ok(spec) = service::current_service_spec(&executable, None) {
        let result = ServiceManager::default().try_uninstall(&spec);
        if result.removed {
            println!("Removed startup service.");
        } else if result.needs_elevation && cfg!(unix) && !is_elevated() {
            let environment = env::vars().collect();
            let home = home_dir();
            let sudo_args = service::build_service_uninstall_sudo_args(
                &executable,
                Path::new(""),
                &home,
                spec.state_dir(),
                &environment,
            );
            if !Command::new("sudo")
                .args(sudo_args)
                .status()
                .is_ok_and(|status| status.success())
            {
                bail!("Failed to remove startup service with sudo");
            }
        } else if result.installed {
            bail!(
                "Could not remove startup service: {}",
                result.error.unwrap_or_else(|| "unknown error".into())
            );
        }
    }
    let state = discover_state().await;
    stop_proxy(Some(state.port)).await?;
    for route in RouteStore::new(&state.dir).load_routes_raw() {
        cleanup_route_integrations(&route);
    }
    let dirs = clean::collect_state_dirs_for_cleanup();
    for dir in &dirs {
        if certs::is_ca_trusted(dir) {
            let result = certs::untrust_ca(dir);
            if !result.removed {
                eprintln!(
                    "Warning: Could not remove CA: {}",
                    result.error.unwrap_or_else(|| "unknown error".into())
                );
            }
        }
    }
    for dir in dirs {
        clean::remove_portless_state_files(dir);
    }
    if !hosts::clean_hosts_file() && cfg!(unix) && !is_elevated() {
        return elevate_clean();
    }
    println!("Clean finished.");
    Ok(0)
}

fn elevate_clean() -> Result<i32> {
    let executable = env::current_exe()?;
    let status = Command::new("sudo")
        .arg("env")
        .args(collect_portless_env_args())
        .arg(format!("HOME={}", home_dir().display()))
        .arg(executable)
        .arg("clean")
        .status()?;
    if status.success() {
        Ok(0)
    } else {
        bail!("Failed to clean with sudo")
    }
}

fn cleanup_route_integrations(route: &Route) {
    tailscale::unregister_tailscale(TailscaleRoute {
        tailscale_https_port: route.tailscale_https_port,
        tailscale_funnel: route.tailscale_funnel.unwrap_or(false),
    });
    ngrok::stop_ngrok(route.ngrok_pid.and_then(|pid| u32::try_from(pid).ok()));
}

async fn handle_prune(args: &[String]) -> Result<i32> {
    if args
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: portless prune [--force]");
        return Ok(0);
    }
    if let Some(unknown) = args.iter().skip(1).find(|arg| arg.as_str() != "--force") {
        bail!("Unknown argument \"{unknown}\"");
    }
    let state = discover_state().await;
    let stale = RouteStore::new(state.dir).prune_stale_routes()?;
    if stale.is_empty() {
        println!("No orphaned routes found.");
        return Ok(0);
    }
    let force = args.iter().any(|arg| arg == "--force");
    let mut killed = 0;
    for route in &stale {
        cleanup_route_integrations(route);
        for pid in pids_on_port(route.port) {
            if terminate_pid(pid, force).is_ok() {
                killed += 1;
                println!(
                    "  {} :{} - killed PID {} ({})",
                    route.hostname,
                    route.port,
                    pid,
                    if force { "force" } else { "terminate" }
                );
            }
        }
    }
    println!(
        "\nPruned {} stale route{}, killed {killed} orphaned process{}.",
        stale.len(),
        if stale.len() == 1 { "" } else { "s" },
        if killed == 1 { "" } else { "es" }
    );
    Ok(0)
}

fn pids_on_port(port: u16) -> Vec<i32> {
    if cfg!(windows) {
        return Command::new("netstat")
            .arg("-ano")
            .output()
            .ok()
            .map(|output| {
                parse_windows_listening_pids(&String::from_utf8_lossy(&output.stdout), port)
            })
            .unwrap_or_default();
    }
    Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|pid| pid.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_windows_listening_pids(output: &str, port: u16) -> Vec<i32> {
    let mut pids = BTreeSet::new();
    for line in output.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5
            || !fields[0].eq_ignore_ascii_case("TCP")
            || !fields[3].eq_ignore_ascii_case("LISTENING")
        {
            continue;
        }
        let Some(local_port) = fields[1].rsplit(':').next() else {
            continue;
        };
        if local_port.parse::<u16>().ok() != Some(port) {
            continue;
        }
        if let Some(pid) = fields.last().and_then(|pid| pid.parse::<i32>().ok())
            && pid > 0
        {
            pids.insert(pid);
        }
    }
    pids.into_iter().collect()
}

async fn handle_doctor(args: &[String]) -> Result<i32> {
    if args
        .get(1)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: portless doctor");
        return Ok(0);
    }
    if let Some(argument) = args.get(1) {
        bail!("Unknown argument \"{argument}\"");
    }
    let state = discover_state().await;
    let store = RouteStore::new(&state.dir);
    let mut failures = 0;
    let mut warnings = 0;
    println!("\nportless doctor\n");
    println!("Version: {VERSION}");
    println!("Platform: {} {}", env::consts::OS, env::consts::ARCH);
    println!("State dir: {}", state.dir.display());
    println!(
        "Proxy target: {}",
        format_url("127.0.0.1", state.port, state.tls)
    );
    println!(
        "Mode: {}, .{}{}",
        if state.tls { "HTTPS" } else { "HTTP" },
        state.tlds.join(", ."),
        if state.lan_mode() { ", LAN" } else { "" }
    );
    println!();

    match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_start_matches('v')
                .to_owned();
            let supported = version
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                .is_some_and(|major| major >= 24);
            if supported {
                finding("ok", &format!("Node.js {version} is supported."));
            } else {
                finding(
                    "fail",
                    &format!("Node.js {version} is unsupported; install Node.js 24 or newer."),
                );
                failures += 1;
            }
        }
        _ => {
            finding("fail", "Node.js is not available on PATH.");
            failures += 1;
        }
    }
    if state.dir.exists() {
        if fs::metadata(&state.dir).is_ok_and(|metadata| metadata.is_dir()) {
            if path_effectively_writable(&state.dir) {
                finding(
                    "ok",
                    &format!("State directory is writable: {}", state.dir.display()),
                );
            } else {
                finding(
                    "fail",
                    &format!("State directory is not writable: {}", state.dir.display()),
                );
                failures += 1;
            }
        } else {
            finding("fail", "State path is not a directory");
            failures += 1;
        }
    } else {
        let ancestor = find_existing_ancestor(
            state
                .dir
                .parent()
                .unwrap_or_else(|| Path::new(std::path::MAIN_SEPARATOR_STR)),
        );
        match ancestor {
            Some(path)
                if fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir())
                    && path_effectively_writable(&path) =>
            {
                finding(
                    "info",
                    &format!(
                        "State directory has not been created yet; writable ancestor: {}",
                        path.display()
                    ),
                );
            }
            Some(path) => {
                finding(
                    "fail",
                    &format!(
                        "State directory does not exist and ancestor is not writable: {}",
                        path.display()
                    ),
                );
                failures += 1;
            }
            None => {
                finding(
                    "fail",
                    "State directory does not exist and no existing ancestor was found.",
                );
                failures += 1;
            }
        }
    }
    if is_proxy_running(state.port).await {
        finding(
            "ok",
            &format!("Proxy is responding on port {}.", state.port),
        );
    } else if is_port_listening(state.port) {
        finding(
            "fail",
            &format!(
                "Port {} is in use, but it is not a portless proxy.",
                state.port
            ),
        );
        failures += 1;
    } else {
        finding(
            "warn",
            &format!("Proxy is not running on port {}.", state.port),
        );
        warnings += 1;
    }
    let proxy_running = is_proxy_running(state.port).await;
    let pid_path = state.dir.join("proxy.pid");
    match fs::read_to_string(&pid_path) {
        Ok(raw) => match raw.trim().parse::<i32>() {
            Ok(pid) if pid > 0 && !pid_alive(pid) => {
                finding("warn", &format!("Proxy PID file is stale: {pid}."));
                warnings += 1;
            }
            Ok(pid) if pid > 0 && !proxy_running => {
                finding(
                    "warn",
                    &format!("Proxy PID file points to PID {pid}, but Portless is not responding."),
                );
                warnings += 1;
            }
            Ok(pid) if pid > 0 => {
                let listener = pids_on_port(state.port).into_iter().next();
                if listener.is_some_and(|listener| listener != pid) {
                    finding(
                        "warn",
                        &format!(
                            "Proxy PID file points to PID {pid}, but port {} has another owner.",
                            state.port
                        ),
                    );
                    warnings += 1;
                } else {
                    finding("ok", &format!("Proxy PID file is valid: {pid}."));
                }
            }
            _ => {
                finding("fail", "Proxy PID file is invalid.");
                failures += 1;
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound && proxy_running => {
            finding("warn", "Proxy is running but its PID file is missing.");
            warnings += 1;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            finding("fail", &format!("Could not read proxy PID file: {error}"));
            failures += 1;
        }
    }
    if state.tls && state.custom_cert {
        finding("ok", "Proxy is configured with a custom TLS certificate.");
        finding(
            "info",
            "Generated local CA is not required for custom TLS certificates.",
        );
    } else if state.tls {
        if command_available("openssl", &["version"]) {
            finding("ok", "OpenSSL is available for certificate generation.");
        } else {
            finding("fail", "OpenSSL is not available on PATH.");
            failures += 1;
        }
        if state.dir.join(certs::CA_CERT_FILE).exists() {
            if certs::is_ca_trusted(&state.dir) {
                finding("ok", "Local CA is trusted by the OS trust store.");
            } else {
                finding(
                    "warn",
                    "Local CA exists but is not trusted. Run: portless trust",
                );
                warnings += 1;
            }
        } else {
            finding("info", "Local CA has not been generated yet.");
        }
    }
    let raw = store.load_routes_raw();
    let no_routes = raw.is_empty();
    let (live, stale): (Vec<_>, Vec<_>) = raw
        .into_iter()
        .partition(|route| route.pid == 0 || pid_alive(route.pid));
    if no_routes {
        finding("info", "No routes are registered.");
    } else if stale.is_empty() {
        finding("ok", &format!("Routes: {} active route(s).", live.len()));
    } else {
        finding(
            "warn",
            &format!(
                "Routes: {} active route(s), {} stale route(s). Run: portless prune",
                live.len(),
                stale.len()
            ),
        );
        warnings += 1;
    }
    for route in stale.iter().take(5) {
        finding(
            "warn",
            &format!(
                "Stale route {} is owned by exited PID {}.",
                route.hostname, route.pid
            ),
        );
        warnings += 1;
    }
    for route in &live {
        if !is_port_listening(route.port) {
            finding(
                "warn",
                &format!(
                    "Route {} points to port {}, but nothing is listening there.",
                    route.hostname, route.port
                ),
            );
            warnings += 1;
        }
    }
    if state.lan_mode() {
        let support = mdns::is_mdns_supported();
        if support.supported {
            finding("ok", "mDNS publishing support is available.");
        } else {
            finding(
                "fail",
                &format!(
                    "LAN mode enabled but mDNS unavailable: {}",
                    support.reason.unwrap_or_default()
                ),
            );
            failures += 1;
        }
        if let Some(ip) = state.lan_ip {
            finding("ok", &format!("LAN IP is recorded: {ip}."));
        } else {
            finding("warn", "LAN mode is enabled but no LAN IP is recorded.");
            warnings += 1;
        }
    } else {
        let managed = hosts::get_managed_hostnames()
            .into_iter()
            .collect::<BTreeSet<_>>();
        for route in live {
            if !hosts::check_host_resolution(&route.hostname).await {
                finding(
                    "warn",
                    &format!(
                        "{} did not resolve ({})",
                        route.hostname,
                        if managed.contains(&route.hostname) {
                            "present in hosts block"
                        } else {
                            "run: portless hosts sync"
                        }
                    ),
                );
                warnings += 1;
            }
        }
    }
    println!();
    println!("Summary: {failures} failure(s), {warnings} warning(s).");
    Ok(if failures == 0 { 0 } else { 1 })
}

fn finding(status: &str, message: &str) {
    println!("{status:5} {message}");
}

fn find_existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.exists())
        .map(Path::to_path_buf)
}

#[cfg(unix)]
fn path_effectively_writable(path: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt as _};

    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a valid, NUL-terminated C string and no pointer escapes.
    unsafe {
        nix::libc::faccessat(
            nix::libc::AT_FDCWD,
            path.as_ptr(),
            nix::libc::W_OK,
            nix::libc::AT_EACCESS,
        ) == 0
    }
}

#[cfg(not(unix))]
fn path_effectively_writable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| !metadata.permissions().readonly())
}

fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            kill(Pid::from_raw(pid), None),
            Ok(()) | Err(nix::errno::Errno::EPERM)
        )
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .any(|field| field == pid.to_string())
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

fn command_available(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

async fn handle_service(args: &[String]) -> Result<i32> {
    match args.get(1).map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!(
                "Usage:\n  portless service install [proxy options]\n  portless service status\n  \
                 portless service uninstall"
            );
            Ok(0)
        }
        Some("install") => {
            let environment: BTreeMap<String, String> = env::vars().collect();
            let config = service::parse_service_install_config(args, &environment, false)?;
            validate_service_install_support(&config)?;
            let executable = env::current_exe()?;
            let spec = service::current_service_spec(&executable, Some(config))?;
            if !is_elevated() && cfg!(unix) {
                let sudo_args = service::build_install_elevation_args(
                    &executable,
                    Path::new(""),
                    args,
                    &home_dir(),
                    spec.state_dir(),
                    &environment,
                );
                let status = Command::new("sudo").args(sudo_args).status()?;
                if status.success() {
                    return Ok(0);
                }
                bail!("Failed to install service with sudo");
            }
            fs::create_dir_all(spec.state_dir())?;
            if service_needs_generated_ca(spec.config()) {
                certs::ensure_certs(spec.state_dir()).with_context(|| {
                    format!(
                        "Failed to generate certificates in {}. Ensure OpenSSL is installed",
                        spec.state_dir().display()
                    )
                })?;
                if !certs::is_ca_trusted(spec.state_dir()) {
                    println!("Trusting portless CA for service startup...");
                    let trust = certs::trust_ca(spec.state_dir());
                    if trust.trusted {
                        println!("CA added to the system trust store.");
                    } else {
                        eprintln!("Warning: could not add the CA to the system trust store.");
                        if let Some(error) = trust.error {
                            eprintln!("{error}");
                        }
                        eprintln!(
                            "Warning: run `portless trust` if browsers show certificate warnings."
                        );
                    }
                }
            }
            ServiceManager::default().install(&spec)?;
            println!("Portless startup service installed.");
            Ok(0)
        }
        Some("status") => {
            let executable = env::current_exe()?;
            let spec = service::current_service_spec(executable, None)?;
            let installed = service::read_installed_service_config(&spec)
                .unwrap_or_else(|| spec.config().clone());
            let running = is_proxy_running(service_status_probe_port(&installed)).await;
            let status = ServiceManager::default().status(&spec, running)?;
            println!("portless service");
            println!(" Manager state: {}", status.manager_state);
            println!(
                " Installed: {}",
                if status.installed { "yes" } else { "no" }
            );
            println!(
                " Proxy on {}: {}",
                status.config.proxy_port,
                if status.proxy_running {
                    "responding"
                } else {
                    "not responding"
                }
            );
            println!(
                " HTTPS: {}",
                if status.config.use_https { "yes" } else { "no" }
            );
            println!(
                " TLDs: {}",
                if status.config.lan_mode {
                    ".local".into()
                } else {
                    status
                        .config
                        .tlds
                        .iter()
                        .map(|tld| format!(".{tld}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            );
            println!(
                " LAN mode: {}",
                if status.config.lan_mode { "yes" } else { "no" }
            );
            if status.config.lan_ip_explicit
                && let Some(ip) = &status.config.lan_ip
            {
                println!(" LAN IP: {ip}");
            }
            println!(
                " Wildcard: {}",
                if status.config.use_wildcard {
                    "yes"
                } else {
                    "no"
                }
            );
            println!(" State directory: {}", status.config.state_dir.display());
            if let Some(details) = status.details {
                println!(" Service entry: {details}");
            }
            Ok(if status.installed { 0 } else { 1 })
        }
        Some("uninstall") => {
            let executable = env::current_exe()?;
            let spec = service::current_service_spec(&executable, None)?;
            let result = ServiceManager::default().try_uninstall(&spec);
            if result.removed {
                println!("Portless startup service uninstalled.");
                return Ok(0);
            }
            if !result.installed {
                println!("Portless startup service is not installed.");
                return Ok(0);
            }
            if result.needs_elevation && cfg!(unix) && !is_elevated() {
                let environment = env::vars().collect();
                let sudo_args = service::build_service_uninstall_sudo_args(
                    &executable,
                    Path::new(""),
                    &home_dir(),
                    spec.state_dir(),
                    &environment,
                );
                if Command::new("sudo")
                    .args(sudo_args)
                    .status()
                    .is_ok_and(|status| status.success())
                {
                    return Ok(0);
                }
            }
            bail!(
                "Could not uninstall service: {}",
                result.error.unwrap_or_else(|| "unknown error".into())
            )
        }
        Some(command) => bail!("Unknown service subcommand \"{command}\""),
    }
}

fn validate_service_install_support(config: &service::ServiceInstallConfig) -> Result<()> {
    validate_service_mdns_support(config.lan_mode, &mdns::is_mdns_supported())
}

fn validate_service_mdns_support(lan_mode: bool, support: &mdns::MdnsSupport) -> Result<()> {
    if !lan_mode {
        return Ok(());
    }
    if support.supported {
        return Ok(());
    }
    bail!(
        "LAN mode requires mDNS publishing, which is not supported on this platform.{}",
        support
            .reason
            .as_deref()
            .map_or_else(String::new, |reason| format!("\n{reason}"))
    )
}

fn service_needs_generated_ca(config: &service::NormalizedServiceConfig) -> bool {
    config.use_https && config.custom_cert_path.is_none()
}

fn service_status_probe_port(config: &service::NormalizedServiceConfig) -> u16 {
    config.proxy_port
}

fn home_dir() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

async fn handle_default(script: Option<&str>, extra: &[String]) -> Result<bool> {
    let cwd = env::current_dir()?;
    if workspace::find_workspace_root(&cwd).as_deref() == Some(&cwd) {
        let packages = workspace::discover_workspace_packages(&cwd);
        let loaded = config::load_config(&cwd)?;
        let script_name = script
            .or_else(|| {
                loaded
                    .as_ref()
                    .and_then(|loaded| loaded.config.script.as_deref())
            })
            .unwrap_or("dev")
            .to_owned();
        if packages
            .iter()
            .any(|package| package.scripts.contains_key(&script_name))
        {
            run_workspace(&cwd, packages, loaded, &script_name, extra).await?;
            return Ok(true);
        }
    }
    let app = load_app_config(&cwd)?;
    let script_name = script
        .or_else(|| app.as_ref().and_then(|app| app.script.as_deref()))
        .unwrap_or("dev");
    if !config::has_script(script_name, &cwd) {
        return Ok(false);
    }
    let parsed = ParsedApp {
        command: config::resolve_script_command(script_name, &cwd).unwrap_or_default(),
        app_port: app.as_ref().and_then(|app| app.app_port),
        ..ParsedApp::default()
    };
    dispatch_app(parsed, true, Some(script_name)).await?;
    Ok(true)
}

async fn run_workspace(
    root: &Path,
    packages: Vec<WorkspacePackage>,
    loaded: Option<config::LoadedConfig>,
    script: &str,
    extra: &[String],
) -> Result<i32> {
    let project = workspace::infer_monorepo_project_name(
        root,
        &packages,
        loaded
            .as_ref()
            .and_then(|loaded| loaded.config.name.as_deref()),
    )?;
    let mut apps = Vec::new();
    for package in packages {
        let root_override = loaded.as_ref().map(|loaded| {
            config::resolve_app_config(&loaded.config, &loaded.config_dir, &package.dir)
        });
        let package_override = config::load_package_portless_config(&package.dir)?;
        let app = root_override
            .unwrap_or_default()
            .merged_with(&package_override.unwrap_or_default());
        let effective_script = app.script.as_deref().unwrap_or(script);
        let Some(raw) = package.scripts.get(effective_script) else {
            continue;
        };
        let split = config::split_command(raw);
        if split.is_empty() {
            continue;
        }
        let name = app.name.as_deref().map_or_else(
            || workspace::infer_monorepo_hostname(&package, root, &project),
            normalize_name,
        );
        let label = package.scope.as_ref().map_or_else(
            || package.name.clone().unwrap_or_else(|| name.clone()),
            |scope| format!("@{scope}/{}", package.name.as_deref().unwrap_or(&name)),
        );
        apps.push(MultiApp {
            command: vec![
                config::detect_package_manager(&package.dir).to_string(),
                "run".into(),
                effective_script.into(),
            ],
            package,
            name,
            label,
            app_port: app.app_port,
            proxied: app
                .proxy
                .unwrap_or_else(|| config::is_server_command(&split)),
        });
    }
    if apps.is_empty() {
        bail!("No workspace packages have a \"{script}\" script");
    }
    apps.sort_by(|left, right| left.label.cmp(&right.label));
    let state = if apps.iter().any(|app| app.proxied) {
        let state = discover_state().await;
        if is_proxy_running(state.port).await {
            let desired = proxy_config_from_env()?;
            if let Some(mismatch) = active_proxy_mismatch(&desired, &state) {
                bail!(
                    "Proxy is already running with different {mismatch}. Stop it first with: \
                     portless proxy stop"
                );
            }
            state
        } else {
            ensure_proxy_running(&state).await?
        }
    } else {
        discover_state().await
    };
    let use_turbo = loaded.as_ref().and_then(|loaded| loaded.config.turbo) != Some(false)
        && turbo::has_turbo_config(root);
    if use_turbo {
        run_workspace_turbo(root, &state, apps, script, extra).await
    } else {
        run_workspace_direct(&state, apps).await
    }
}

async fn run_workspace_turbo(
    root: &Path,
    state: &State,
    apps: Vec<MultiApp>,
    script: &str,
    extra: &[String],
) -> Result<i32> {
    let store = RouteStore::new(&state.dir);
    let owner = i32::try_from(std::process::id()).context("process ID is too large")?;
    let mut manifest = turbo::Manifest::new();
    let mut routes = Vec::new();
    let inherited: ChildEnv = env::vars_os().collect();
    for app in apps.iter().filter(|app| app.proxied) {
        let port = match app.app_port {
            Some(port) => port,
            None => process::find_free_port().await?,
        };
        let hostnames = build_hostnames(&app.name, &state.tlds)?;
        add_routes(&store, &hostnames, port, owner, false)?;
        let hostname = hostnames
            .first()
            .ok_or_else(|| anyhow!("no hostname generated"))?;
        let url = format_url(hostname, state.port, state.tls);
        println!("  {:<24} {url}", app.label);
        manifest.insert(
            app.package.dir.to_string_lossy().into_owned(),
            turbo::ManifestEntry {
                port: port.to_string(),
                host: "127.0.0.1".into(),
                portless_url: url,
                vite_additional_server_allowed_hosts: Some(format_allowed_hosts(&state.tlds)),
                node_extra_ca_certs: generated_ca_for_child(state, &inherited)
                    .map(|path| path.to_string_lossy().into_owned()),
            },
        );
        routes.push(hostnames);
    }
    let base = turbo::user_state_dir()?;
    turbo::ensure_env_loader(&base)?;
    turbo::write_manifest(&manifest, &base)?;
    let manager = config::detect_package_manager(root);
    let mut command = if config::has_script(script, root) {
        vec![manager.to_string(), "run".into(), script.into()]
    } else {
        match manager {
            config::PackageManager::Npm => {
                vec!["npx".into(), "turbo".into(), "run".into(), script.into()]
            }
            config::PackageManager::Bun => {
                vec!["bunx".into(), "turbo".into(), "run".into(), script.into()]
            }
            _ => vec![
                manager.to_string(),
                "exec".into(),
                "turbo".into(),
                "run".into(),
                script.into(),
            ],
        }
    };
    command.extend_from_slice(extra);
    let mut environment = inherited;
    environment.insert(
        "NODE_OPTIONS".into(),
        turbo::build_node_options(&base).into(),
    );
    let code = run_child_with_signals(&command, root, environment).await;
    for hostnames in routes {
        remove_routes(&store, &hostnames, Some(owner));
    }
    let _ = turbo::remove_manifest(&base);
    code
}

async fn run_workspace_direct(state: &State, apps: Vec<MultiApp>) -> Result<i32> {
    let owner = i32::try_from(std::process::id()).context("process ID is too large")?;
    let store = RouteStore::new(&state.dir);
    let executable = env::current_exe()?;
    let mut children = Vec::new();
    let mut route_groups = Vec::new();
    for app in apps {
        let inherited: ChildEnv = env::vars_os().collect();
        let environment = if app.proxied {
            let port = match app.app_port {
                Some(port) => port,
                None => process::find_free_port().await?,
            };
            let hostnames = build_hostnames(&app.name, &state.tlds)?;
            add_routes(&store, &hostnames, port, owner, false)?;
            let url = format_url(
                hostnames
                    .first()
                    .ok_or_else(|| anyhow!("no hostname generated"))?,
                state.port,
                state.tls,
            );
            println!("  {:<24} {url}", app.label);
            route_groups.push(hostnames);
            process::build_portless_child_env(
                &inherited,
                &PortlessChildEnv {
                    port,
                    host: Some("127.0.0.1"),
                    portless_url: &url,
                    vite_allowed_hosts: &format_allowed_hosts(&state.tlds),
                    lan_mode: state.lan_mode(),
                    tailscale_url: None,
                    ngrok_url: None,
                    node_extra_ca_certs: generated_ca_for_child(state, &inherited).as_deref(),
                },
                &app.package.dir,
                &executable,
            )
        } else {
            process::build_child_env(&inherited, &ChildEnv::new(), &app.package.dir, &executable)
        };
        let mut command = tokio::process::Command::new(
            app.command
                .first()
                .ok_or_else(|| anyhow!("empty workspace command"))?,
        );
        command
            .args(app.command.get(1..).unwrap_or_default())
            .current_dir(&app.package.dir)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        process::configure_tokio_process_group(&mut command);
        match command.spawn() {
            Ok(child) => children.push(child),
            Err(error) => {
                terminate_workspace_children(&mut children, false).await;
                for hostnames in route_groups {
                    remove_routes(&store, &hostnames, Some(owner));
                }
                return Err(error.into());
            }
        }
    }
    let mut active = vec![true; children.len()];
    let mut failed = false;
    let mut signal_code = None;
    #[cfg(unix)]
    {
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        while active.iter().any(|active| *active) {
            for (index, child) in children.iter_mut().enumerate() {
                if active[index]
                    && let Some(status) = child.try_wait()?
                {
                    active[index] = false;
                    failed |= !status.success();
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                _ = interrupt.recv() => {
                    for (index, child) in children.iter().enumerate() {
                        if active[index] && let Some(pid) = child.id() {
                            let _ = process::signal_process_tree(pid, Signal::SIGINT);
                        }
                    }
                    signal_code = Some(process::signal_exit_code(Signal::SIGINT));
                    break;
                }
                _ = terminate.recv() => {
                    for (index, child) in children.iter().enumerate() {
                        if active[index] && let Some(pid) = child.id() {
                            let _ = process::signal_process_tree(pid, Signal::SIGTERM);
                        }
                    }
                    signal_code = Some(process::signal_exit_code(Signal::SIGTERM));
                    break;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        while active.iter().any(|active| *active) {
            for (index, child) in children.iter_mut().enumerate() {
                if active[index]
                    && let Some(status) = child.try_wait()?
                {
                    active[index] = false;
                    failed |= !status.success();
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                _ = tokio::signal::ctrl_c() => {
                    signal_code = Some(130);
                    break;
                }
            }
        }
    }
    if signal_code.is_some() {
        terminate_workspace_children(&mut children, false).await;
    }
    for hostnames in route_groups {
        remove_routes(&store, &hostnames, Some(owner));
    }
    Ok(signal_code.unwrap_or_else(|| i32::from(failed)))
}

async fn terminate_workspace_children(children: &mut [tokio::process::Child], force: bool) {
    for child in children.iter_mut() {
        let Some(pid) = child.id() else {
            continue;
        };
        #[cfg(unix)]
        {
            let signal = if force {
                Signal::SIGKILL
            } else {
                Signal::SIGTERM
            };
            let _ = process::signal_process_tree(pid, signal);
        }
        #[cfg(not(unix))]
        {
            let _ = force;
            let _ = child.kill().await;
        }
    }
    for child in children {
        let _ = child.wait().await;
    }
}

async fn run_direct_command(
    command: &[String],
    cwd: &Path,
    environment: Option<ChildEnv>,
) -> Result<i32> {
    let environment = environment.unwrap_or_else(|| env::vars_os().collect());
    run_child_with_signals(command, cwd, environment).await
}

fn print_help() {
    println!(
        r#"
portless - Replace port numbers with stable, named .localhost URLs. For humans and agents.

Usage:
  portless                         Run dev script through proxy
  portless run [options] [cmd...]  Infer project name and run through proxy
  portless <name> <cmd...>         Run with an explicit app name
  portless get <name>              Print URL for a service
  portless alias <name> <port>     Register a static route
  portless list                    Show active routes
  portless doctor                  Check local portless health
  portless trust                   Trust the local CA
  portless clean                   Remove Portless artifacts
  portless prune [--force]         Clean orphaned routes and processes
  portless hosts sync|clean        Manage hosts-file entries
  portless proxy start|stop        Manage the proxy
  portless service install|status|uninstall

App options:
  --name <name>  --force  --app-port <port>  --tailscale  --funnel  --ngrok
Global options:
  --script <name>  --lan  --ip <address>  --help  --version

Environment:
  PORTLESS_PORT, PORTLESS_APP_PORT, PORTLESS_HTTPS, PORTLESS_LAN,
  PORTLESS_LAN_IP, PORTLESS_TLD, PORTLESS_WILDCARD, PORTLESS_SYNC_HOSTS,
  PORTLESS_TAILSCALE, PORTLESS_FUNNEL, PORTLESS_NGROK, PORTLESS_STATE_DIR
  PORTLESS=0 runs the child command without proxy setup.
"#
    );
}

fn print_run_help() {
    println!(
        r#"
portless run - Infer project name and run through the proxy.

Usage:
  portless run [options] [command...]

Options:
  --name <name>       Override inferred name
  --force             Take over an existing route
  --app-port <port>   Use a fixed app port
  --tailscale         Share on the tailnet
  --funnel            Share publicly via Tailscale Funnel
  --ngrok             Share publicly via ngrok
  --help, -h          Show this help
"#
    );
}

fn print_proxy_help() {
    println!(
        r#"
portless proxy - Manage the proxy server.

Usage:
  portless proxy start [--port <port>] [--no-tls] [--foreground]
                       [--lan] [--ip <address>] [--tld <tld>] [--wildcard]
                       [--cert <path> --key <path>]
  portless proxy stop [--port <port>]
"#
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn run_parser_preserves_child_flags_after_command() {
        let parsed = parse_run_args(&strings(&[
            "--name", "web", "--force", "vite", "--host", "0.0.0.0",
        ]))
        .expect("parse");
        assert_eq!(parsed.name.as_deref(), Some("web"));
        assert!(parsed.force);
        assert_eq!(parsed.command, strings(&["vite", "--host", "0.0.0.0"]));
    }

    #[test]
    fn named_parser_accepts_flags_before_and_after_name() {
        let parsed = parse_named_args(&strings(&[
            "--force",
            "web",
            "--app-port",
            "4100",
            "--",
            "tool",
            "--force",
        ]))
        .expect("parse");
        assert_eq!(parsed.name.as_deref(), Some("web"));
        assert_eq!(parsed.app_port, Some(4100));
        assert!(parsed.force);
        assert_eq!(parsed.command, strings(&["tool", "--force"]));
    }

    #[test]
    fn global_flags_stop_at_separator() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { env::remove_var("PORTLESS_LAN") };
        let mut args = strings(&["run", "tool", "--", "--lan"]);
        let script = strip_global_flags(&mut args).expect("flags");
        assert_eq!(script, None);
        assert_eq!(args, strings(&["run", "tool", "--", "--lan"]));
        assert!(env::var_os("PORTLESS_LAN").is_none());
    }

    #[test]
    fn tld_and_hostname_validation_match_command_contract() {
        assert_eq!(
            parse_tlds("localhost,test,localhost").expect("tlds"),
            strings(&["localhost", "test"])
        );
        assert_eq!(
            build_hostnames("api.team", &strings(&["localhost", "test"])).expect("hostnames"),
            strings(&["api.team.localhost", "api.team.test"])
        );
        assert!(build_hostnames("-bad", &strings(&["localhost"])).is_err());
        assert!(parse_tlds("not_valid").is_err());
    }

    #[test]
    fn proxy_parser_applies_protocol_port_and_lan_tld() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        for key in [
            "PORTLESS_PORT",
            "PORTLESS_HTTPS",
            "PORTLESS_LAN",
            "PORTLESS_LAN_IP",
            "PORTLESS_TLD",
            "PORTLESS_WILDCARD",
        ] {
            // SAFETY: serialized by ENV_LOCK.
            unsafe { env::remove_var(key) };
        }
        let parsed = parse_proxy_start(&strings(&[
            "proxy",
            "start",
            "--no-tls",
            "--lan",
            "--ip",
            "192.168.1.42",
            "--tld",
            "test",
        ]))
        .expect("proxy args");
        assert_eq!(parsed.port, 80);
        assert!(!parsed.tls);
        assert_eq!(parsed.tlds, ["local"]);
        assert_eq!(
            parsed.lan_ip,
            Some("192.168.1.42".parse().expect("test IP"))
        );
    }

    #[test]
    fn proxy_args_round_trip_relevant_configuration() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        for key in [
            "PORTLESS_PORT",
            "PORTLESS_HTTPS",
            "PORTLESS_LAN",
            "PORTLESS_LAN_IP",
            "PORTLESS_TLD",
            "PORTLESS_WILDCARD",
        ] {
            // SAFETY: serialized by ENV_LOCK.
            unsafe { env::remove_var(key) };
        }
        let original = parse_proxy_start(&strings(&[
            "proxy",
            "start",
            "-p",
            "1355",
            "--no-tls",
            "--tld",
            "localhost,test",
            "--wildcard",
        ]))
        .expect("original");
        let mut args = strings(&["proxy", "start"]);
        args.extend(proxy_args(&original, true));
        let reparsed = parse_proxy_start(&args).expect("reparsed");
        assert_eq!(reparsed.port, original.port);
        assert_eq!(reparsed.tls, original.tls);
        assert_eq!(reparsed.tlds, original.tlds);
        assert_eq!(reparsed.wildcard, original.wildcard);
        assert!(reparsed.foreground);
    }

    #[test]
    fn hostname_normalization_preserves_labels_and_parses_inputs() {
        assert_eq!(normalize_name("local.MyApp"), "local.MyApp");
        assert_eq!(
            build_hostnames(
                "https://API.localhost/path",
                &strings(&["localhost", "test"])
            )
            .expect("hostnames"),
            strings(&["api.localhost", "api.test"])
        );
        assert!(build_hostnames("My_App", &strings(&["localhost"])).is_err());
    }

    #[test]
    fn tld_validation_and_risk_warnings_match_upstream() {
        assert!(parse_tlds(".localhost").is_err());
        assert!(parse_tlds("my-dev").is_err());
        assert_eq!(
            parse_tlds("DEV,localhost,dev").expect("valid TLD list"),
            strings(&["dev", "localhost"])
        );
        assert!(risky_tld_reason("dev").is_some());
        assert!(risky_tld_reason("test").is_none());
    }

    #[test]
    fn active_proxy_explicit_mismatches_are_rejected() {
        let active = State {
            dir: PathBuf::from("/tmp/portless-test"),
            port: 443,
            tls: true,
            tlds: strings(&["localhost"]),
            lan_ip: None,
            custom_cert: false,
        };
        let mut config = ProxyConfig {
            port: 443,
            port_explicit: false,
            tls: false,
            tls_explicit: true,
            foreground: false,
            skip_trust: false,
            cert: None,
            key: None,
            lan: false,
            lan_explicit: false,
            lan_ip: None,
            lan_ip_explicit: false,
            tlds: strings(&["localhost"]),
            tlds_explicit: false,
            wildcard: false,
        };
        assert_eq!(active_proxy_mismatch(&config, &active), Some("TLS mode"));
        config.tls_explicit = false;
        assert_eq!(active_proxy_mismatch(&config, &active), None);
    }

    #[test]
    fn custom_certificates_disable_generated_sni() {
        let mut config = ProxyConfig {
            port: 443,
            port_explicit: false,
            tls: true,
            tls_explicit: false,
            foreground: false,
            skip_trust: false,
            cert: None,
            key: None,
            lan: false,
            lan_explicit: false,
            lan_ip: None,
            lan_ip_explicit: false,
            tlds: strings(&["localhost"]),
            tlds_explicit: false,
            wildcard: false,
        };
        assert!(generated_sni_enabled(&config));
        config.cert = Some(PathBuf::from("cert.pem"));
        config.key = Some(PathBuf::from("key.pem"));
        assert!(!generated_sni_enabled(&config));
    }

    #[test]
    fn windows_netstat_parser_only_returns_exact_listeners() {
        let output = "\
TCP    0.0.0.0:443       0.0.0.0:0       LISTENING       123
TCP    [::]:443          [::]:0          LISTENING       456
TCP    127.0.0.1:4430    0.0.0.0:0       LISTENING       789
TCP    127.0.0.1:443     127.0.0.1:99    ESTABLISHED     999
UDP    0.0.0.0:443       *:*                             321";
        assert_eq!(parse_windows_listening_pids(output, 443), vec![123, 456]);
    }

    #[test]
    fn state_discovery_probes_standard_legacy_and_configured_ports() {
        assert_eq!(active_probe_ports(8443), vec![443, 80, 1355, 8443]);
        assert_eq!(active_probe_ports(443), vec![443, 80, 1355]);
    }

    #[test]
    fn single_app_proxy_false_runs_without_proxy() {
        let disabled = AppConfig {
            proxy: Some(false),
            ..AppConfig::default()
        };
        assert!(!single_app_proxy_enabled(Some(&disabled)));
        assert!(single_app_proxy_enabled(None));
    }

    #[test]
    fn proxy_markers_are_upstream_compatible_and_cleanup_is_complete() {
        let dir = env::temp_dir().join(format!(
            "portless-cli-marker-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let state = State {
            dir: dir.clone(),
            port: 8443,
            tls: true,
            tlds: strings(&["test"]),
            lan_ip: Some("192.168.1.2".parse().expect("IP")),
            custom_cert: true,
        };
        write_proxy_state(&state).expect("state");
        assert_eq!(fs::read_to_string(dir.join("proxy.tls")).expect("TLS"), "1");
        assert_eq!(
            fs::read_to_string(dir.join("proxy.custom-cert")).expect("custom"),
            "1"
        );
        assert_eq!(
            fs::read_to_string(dir.join("proxy.port")).expect("port"),
            "8443"
        );
        assert_eq!(
            fs::read_to_string(dir.join("proxy.tlds")).expect("TLDs"),
            "test\n"
        );
        clear_proxy_runtime_state(&dir);
        for marker in [
            "proxy.pid",
            "proxy.port",
            "proxy.tls",
            "proxy.custom-cert",
            "proxy.tld",
            "proxy.tlds",
            "proxy.lan",
        ] {
            assert!(!dir.join(marker).exists(), "{marker} should be removed");
        }
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn route_watch_signature_detects_rewrites() {
        let path =
            env::temp_dir().join(format!("portless-routes-signature-{}", std::process::id()));
        fs::write(&path, "[]").expect("initial");
        let initial = route_file_signature(&path).expect("signature");
        fs::write(&path, "[{\"changed\":true}]").expect("rewrite");
        let changed = route_file_signature(&path).expect("changed signature");
        assert_ne!(initial, changed);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn persisted_proxy_settings_only_fill_non_explicit_values() {
        let dir = env::temp_dir().join(format!("portless-persisted-config-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("state directory");
        fs::write(dir.join("proxy.port"), "8443").expect("port marker");
        let persisted = State {
            dir: dir.clone(),
            port: 8443,
            tls: false,
            tlds: strings(&["test"]),
            lan_ip: Some("192.168.1.8".parse().expect("IP")),
            custom_cert: false,
        };
        let mut explicit = ProxyConfig {
            port: 443,
            port_explicit: false,
            tls: true,
            tls_explicit: true,
            foreground: false,
            skip_trust: false,
            cert: None,
            key: None,
            lan: false,
            lan_explicit: true,
            lan_ip: None,
            lan_ip_explicit: false,
            tlds: strings(&["localhost"]),
            tlds_explicit: true,
            wildcard: false,
        };
        merge_persisted_proxy_config(&mut explicit, &persisted);
        assert_eq!(explicit.port, 8443, "custom persisted port is retained");
        assert!(explicit.tls, "explicit TLS mode wins");
        assert!(!explicit.lan, "explicit LAN opt-out wins");
        assert_eq!(explicit.tlds, strings(&["localhost"]));

        let mut inherited = explicit.clone();
        inherited.tls_explicit = false;
        inherited.lan_explicit = false;
        inherited.tlds_explicit = false;
        merge_persisted_proxy_config(&mut inherited, &persisted);
        assert!(!inherited.tls, "persisted TLS fills a non-explicit value");
        assert!(inherited.lan, "persisted LAN fills a non-explicit value");
        assert_eq!(inherited.tlds, strings(&["local"]));
        assert_eq!(inherited.lan_ip, persisted.lan_ip);

        explicit.port = 9443;
        explicit.port_explicit = true;
        merge_persisted_proxy_config(&mut explicit, &persisted);
        assert_eq!(explicit.port, 9443, "explicit port wins");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn active_proxy_mismatch_includes_app_explicit_port_and_tlds() {
        let active = State {
            dir: PathBuf::from("/tmp/portless-active"),
            port: 443,
            tls: true,
            tlds: strings(&["localhost"]),
            lan_ip: None,
            custom_cert: false,
        };
        let config = ProxyConfig {
            port: 8443,
            port_explicit: true,
            tls: true,
            tls_explicit: false,
            foreground: false,
            skip_trust: false,
            cert: None,
            key: None,
            lan: false,
            lan_explicit: false,
            lan_ip: None,
            lan_ip_explicit: false,
            tlds: strings(&["test"]),
            tlds_explicit: true,
            wildcard: false,
        };
        assert_eq!(active_proxy_mismatch(&config, &active), Some("port"));
    }

    #[test]
    fn workspace_ca_injection_respects_user_env_and_custom_certs() {
        let dir = env::temp_dir().join(format!("portless-ca-env-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("state directory");
        fs::write(dir.join(certs::CA_CERT_FILE), "test").expect("CA");
        let mut state = State {
            dir: dir.clone(),
            port: 443,
            tls: true,
            tlds: strings(&["localhost"]),
            lan_ip: None,
            custom_cert: false,
        };
        let mut inherited = ChildEnv::new();
        assert_eq!(
            generated_ca_for_child(&state, &inherited),
            Some(dir.join(certs::CA_CERT_FILE))
        );
        inherited.insert("NODE_EXTRA_CA_CERTS".into(), "user-ca.pem".into());
        assert_eq!(generated_ca_for_child(&state, &inherited), None);
        inherited.clear();
        state.custom_cert = true;
        assert_eq!(generated_ca_for_child(&state, &inherited), None);
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn ngrok_early_exit_prevents_stale_metadata_commit() {
        let dir = env::temp_dir().join(format!("portless-ngrok-race-{}", std::process::id()));
        let store = RouteStore::new(&dir);
        store.ensure_dir().expect("state directory");
        store
            .add_route("app.localhost", 4100, 1, false)
            .expect("route");
        let lifecycle = Mutex::new(true);
        assert!(
            !commit_ngrok_metadata(
                &store,
                "app.localhost",
                &lifecycle,
                "https://example.ngrok.app",
                Some(123),
            )
            .expect("guard")
        );
        let route = store.load_routes_raw().into_iter().next().expect("route");
        assert_eq!(route.ngrok_url, None);
        assert_eq!(route.ngrok_pid, None);
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn service_ca_and_status_use_effective_installed_configuration() {
        let mut config = service::NormalizedServiceConfig {
            state_dir: PathBuf::from("/tmp/portless-service"),
            proxy_port: 9443,
            use_https: true,
            custom_cert_path: None,
            custom_key_path: None,
            lan_mode: false,
            lan_ip: None,
            lan_ip_explicit: false,
            tld: "localhost".into(),
            tlds: strings(&["localhost"]),
            use_wildcard: false,
            extra_env: BTreeMap::new(),
        };
        assert!(service_needs_generated_ca(&config));
        assert_eq!(service_status_probe_port(&config), 9443);
        assert!(
            validate_service_mdns_support(
                true,
                &mdns::MdnsSupport {
                    supported: false,
                    reason: Some("publisher unavailable".into()),
                },
            )
            .is_err()
        );
        assert!(
            validate_service_mdns_support(
                false,
                &mdns::MdnsSupport {
                    supported: false,
                    reason: Some("publisher unavailable".into()),
                },
            )
            .is_ok()
        );
        config.custom_cert_path = Some(PathBuf::from("cert.pem"));
        config.custom_key_path = Some(PathBuf::from("key.pem"));
        assert!(!service_needs_generated_ca(&config));
    }

    #[test]
    fn doctor_finds_effectively_writable_existing_ancestor() {
        let dir = env::temp_dir().join(format!("portless-doctor-ancestor-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("ancestor");
        let missing = dir.join("one").join("two");
        assert_eq!(find_existing_ancestor(&missing), Some(dir.clone()));
        assert!(path_effectively_writable(&dir));
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
