//! Tailscale Serve and Funnel integration.
//!
//! This module intentionally keeps command execution behind
//! [`TailscaleCommandRunner`] so callers can test policy without a Tailscale
//! installation.

use std::{
    collections::HashSet,
    fmt,
    io::{self, Read},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

const TAILSCALE_BINARY: &str = "tailscale";
const TAILSCALE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PREFERRED_SERVE_PORTS: &[u16] = &[443, 8443, 8444, 8445, 8446, 8447, 8448, 8449, 8450];
const FUNNEL_PORTS: &[u16] = &[443, 8443, 10000];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandErrorKind {
    NotFound,
    TimedOut,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TailscaleCommandResult {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<CommandError>,
}

pub trait TailscaleCommandRunner: Send + Sync {
    fn run(&self, args: &[String]) -> TailscaleCommandResult;
}

impl<F> TailscaleCommandRunner for F
where
    F: Fn(&[String]) -> TailscaleCommandResult + Send + Sync,
{
    fn run(&self, args: &[String]) -> TailscaleCommandResult {
        self(args)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemTailscaleCommandRunner;

impl TailscaleCommandRunner for SystemTailscaleCommandRunner {
    fn run(&self, args: &[String]) -> TailscaleCommandResult {
        run_command_with_timeout(TAILSCALE_BINARY, args, TAILSCALE_COMMAND_TIMEOUT)
    }
}

fn run_command_with_timeout(
    binary: &str,
    args: &[String],
    timeout: Duration,
) -> TailscaleCommandResult {
    let spawned = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return TailscaleCommandResult {
                error: Some(command_error(error)),
                ..TailscaleCommandResult::default()
            };
        }
    };
    let stdout = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        })
    });
    let stderr = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        })
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return TailscaleCommandResult {
                    status: status.code(),
                    stdout: join_output(stdout),
                    stderr: join_output(stderr),
                    error: None,
                };
            }
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return TailscaleCommandResult {
                    error: Some(CommandError {
                        kind: CommandErrorKind::TimedOut,
                        message: format!("{binary} command timed out"),
                    }),
                    ..TailscaleCommandResult::default()
                };
            }
            Err(error) => {
                return TailscaleCommandResult {
                    error: Some(command_error(error)),
                    ..TailscaleCommandResult::default()
                };
            }
        }
    }
}

fn join_output(reader: Option<thread::JoinHandle<Vec<u8>>>) -> String {
    let bytes = reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn command_error(error: io::Error) -> CommandError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => CommandErrorKind::NotFound,
        io::ErrorKind::TimedOut => CommandErrorKind::TimedOut,
        _ => CommandErrorKind::Other,
    };
    CommandError {
        kind,
        message: error.to_string(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleError(pub String);

impl fmt::Display for TailscaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TailscaleError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleReady {
    pub dns_name: String,
    pub base_url: String,
}

#[derive(Clone, Copy, Default)]
pub struct EnsureTailscaleReadyOptions<'a> {
    pub require_funnel: bool,
    pub require_https: bool,
    pub runner: Option<&'a dyn TailscaleCommandRunner>,
}

pub fn ensure_tailscale_ready(
    options: EnsureTailscaleReadyOptions<'_>,
) -> Result<TailscaleReady, TailscaleError> {
    let system_runner = SystemTailscaleCommandRunner;
    let runner = options.runner.unwrap_or(&system_runner);
    run_or_error(&["version"], "check tailscale version", runner)?;
    let result = run_or_error(&["status", "--json"], "read tailscale status", runner)?;
    let status: Value = serde_json::from_str(&result.stdout)
        .map_err(|_| TailscaleError("Failed to parse `tailscale status --json` output.".into()))?;
    let dns_name = status_to_dns_name(&status)?;
    if options.require_https && !has_capability(&status, is_https_capability) {
        return Err(TailscaleError(
            "Tailscale HTTPS is not enabled on your tailnet. Enable HTTPS certificates in \
             Tailscale DNS settings, then run portless again."
                .into(),
        ));
    }
    if options.require_funnel && !has_capability(&status, is_funnel_capability) {
        let enable_url = status
            .pointer("/Self/ID")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| {
                format!(" Visit https://login.tailscale.com/f/funnel?node={id} to enable it.")
            })
            .unwrap_or_default();
        return Err(TailscaleError(format!(
            "Tailscale Funnel is not enabled on your tailnet. Enable Funnel for this node, then \
             run portless again.{enable_url}"
        )));
    }
    Ok(TailscaleReady {
        base_url: format!("https://{dns_name}"),
        dns_name,
    })
}

fn run_or_error(
    args: &[&str],
    action: &str,
    runner: &dyn TailscaleCommandRunner,
) -> Result<TailscaleCommandResult, TailscaleError> {
    let args = strings(args);
    let result = runner.run(&args);
    if let Some(error) = &result.error {
        if error.kind == CommandErrorKind::NotFound {
            return Err(TailscaleError(
                "Tailscale CLI not found. Install Tailscale (https://tailscale.com/download) and \
                 ensure `tailscale` is on PATH."
                    .into(),
            ));
        }
        return Err(TailscaleError(format!(
            "Failed to {action}: {}",
            error.message
        )));
    }
    if result.status != Some(0) {
        let details = normalize_space(if result.stderr.is_empty() {
            &result.stdout
        } else {
            &result.stderr
        });
        return Err(TailscaleError(format!(
            "Failed to {action}: {}",
            if details.is_empty() {
                "unknown tailscale error"
            } else {
                &details
            }
        )));
    }
    Ok(result)
}

fn status_to_dns_name(status: &Value) -> Result<String, TailscaleError> {
    if let Some(name) = status
        .pointer("/Self/DNSName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        return Ok(trim_dot(name).to_owned());
    }
    let host = status
        .pointer("/Self/HostName")
        .and_then(Value::as_str)
        .filter(|host| !host.is_empty());
    let suffix = status
        .pointer("/CurrentTailnet/MagicDNSSuffix")
        .and_then(Value::as_str)
        .filter(|suffix| !suffix.is_empty());
    match (host, suffix) {
        (Some(host), Some(suffix)) => Ok(format!("{host}.{}", trim_dot(suffix))),
        _ => Err(TailscaleError(
            "Could not determine Tailscale node DNS name from `tailscale status --json`. Is \
             Tailscale connected?"
                .into(),
        )),
    }
}

fn trim_dot(value: &str) -> &str {
    value.strip_suffix('.').unwrap_or(value)
}

fn has_capability(status: &Value, predicate: fn(&str) -> bool) -> bool {
    status
        .pointer("/Self/Capabilities")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().filter_map(Value::as_str).any(predicate))
        || status
            .pointer("/Self/CapMap")
            .and_then(Value::as_object)
            .is_some_and(|map| map.keys().any(|key| predicate(key)))
}

fn is_funnel_capability(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    normalized == "funnel" || normalized.ends_with("/funnel")
}

fn is_https_capability(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    normalized == "https" || normalized.ends_with("/https")
}

pub fn get_used_serve_ports() -> HashSet<u16> {
    get_used_serve_ports_with_runner(&SystemTailscaleCommandRunner)
}

pub fn get_used_serve_ports_with_runner(runner: &dyn TailscaleCommandRunner) -> HashSet<u16> {
    let result = runner.run(&strings(&["serve", "status", "--json"]));
    if result.error.is_some() || result.status != Some(0) {
        return HashSet::new();
    }
    let Ok(config) = serde_json::from_str::<Value>(&result.stdout) else {
        return HashSet::new();
    };
    let mut ports = HashSet::new();
    if let Some(web) = config.get("Web").and_then(Value::as_object) {
        for host_port in web.keys() {
            if let Some(port) = host_port
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse::<u16>().ok())
            {
                ports.insert(port);
            }
        }
    }
    if let Some(tcp) = config.get("TCP").and_then(Value::as_object) {
        ports.extend(tcp.keys().filter_map(|port| port.parse::<u16>().ok()));
    }
    ports
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TailscaleMode {
    #[default]
    Serve,
    Funnel,
}

pub fn find_available_serve_port(
    used_ports: &HashSet<u16>,
    mode: TailscaleMode,
) -> Result<u16, TailscaleError> {
    let pool = match mode {
        TailscaleMode::Serve => PREFERRED_SERVE_PORTS,
        TailscaleMode::Funnel => FUNNEL_PORTS,
    };
    if let Some(port) = pool.iter().find(|port| !used_ports.contains(port)) {
        return Ok(*port);
    }
    if mode == TailscaleMode::Funnel {
        return Err(TailscaleError(
            "All Tailscale Funnel ports are in use (443, 8443, 10000). Stop an existing funnel to \
             free a port."
                .into(),
        ));
    }
    let mut port = PREFERRED_SERVE_PORTS.last().copied().unwrap_or(8450) + 1;
    while used_ports.contains(&port) {
        port = port.checked_add(1).ok_or_else(|| {
            TailscaleError("No valid Tailscale Serve HTTPS port is available.".into())
        })?;
    }
    Ok(port)
}

pub fn register_serve(local_port: u16, https_port: u16) -> Result<(), TailscaleError> {
    register_with_runner(
        TailscaleMode::Serve,
        local_port,
        https_port,
        &SystemTailscaleCommandRunner,
    )
}

pub fn register_funnel(local_port: u16, https_port: u16) -> Result<(), TailscaleError> {
    register_with_runner(
        TailscaleMode::Funnel,
        local_port,
        https_port,
        &SystemTailscaleCommandRunner,
    )
}

pub fn register_with_runner(
    mode: TailscaleMode,
    local_port: u16,
    https_port: u16,
    runner: &dyn TailscaleCommandRunner,
) -> Result<(), TailscaleError> {
    let command = mode.command();
    let args = vec![
        command.into(),
        "--bg".into(),
        "--yes".into(),
        format!("--https={https_port}"),
        format!("http://127.0.0.1:{local_port}"),
    ];
    let result = runner.run(&args);
    if let Some(error) = &result.error {
        if error.kind == CommandErrorKind::NotFound {
            return Err(cli_not_found());
        }
        if mode == TailscaleMode::Funnel && funnel_not_enabled(&result) {
            return Err(funnel_not_enabled_error(&result));
        }
        if mode == TailscaleMode::Funnel && error.kind == CommandErrorKind::TimedOut {
            return Err(TailscaleError(
                "Tailscale Funnel registration timed out. Make sure Funnel is enabled on your \
                 tailnet, then run portless again."
                    .into(),
            ));
        }
        return Err(TailscaleError(format!(
            "Failed to register tailscale {command}: {}",
            error.message
        )));
    }
    if result.status != Some(0) {
        if mode == TailscaleMode::Funnel && funnel_not_enabled(&result) {
            return Err(funnel_not_enabled_error(&result));
        }
        if conflict_error(&result) {
            let advice = match mode {
                TailscaleMode::Serve => {
                    "Stop the existing serve or let portless auto-assign a different port."
                }
                TailscaleMode::Funnel => "Tailscale Funnel supports ports 443, 8443, and 10000.",
            };
            let label = if mode == TailscaleMode::Funnel {
                "Funnel "
            } else {
                ""
            };
            return Err(TailscaleError(format!(
                "Tailscale {label}HTTPS port {https_port} is already in use. {advice}"
            )));
        }
        let details = preferred_output(&result);
        return Err(TailscaleError(format!(
            "Failed to register tailscale {command} on port {https_port}: {}",
            if details.is_empty() {
                "unknown tailscale error"
            } else {
                &details
            }
        )));
    }
    Ok(())
}

pub fn unregister_serve(https_port: u16, ignore_missing: bool) -> Result<(), TailscaleError> {
    unregister_with_runner(
        TailscaleMode::Serve,
        https_port,
        ignore_missing,
        &SystemTailscaleCommandRunner,
    )
}

pub fn unregister_funnel(https_port: u16, ignore_missing: bool) -> Result<(), TailscaleError> {
    unregister_with_runner(
        TailscaleMode::Funnel,
        https_port,
        ignore_missing,
        &SystemTailscaleCommandRunner,
    )
}

pub fn unregister_with_runner(
    mode: TailscaleMode,
    https_port: u16,
    ignore_missing: bool,
    runner: &dyn TailscaleCommandRunner,
) -> Result<(), TailscaleError> {
    let command = mode.command();
    let result = runner.run(&[
        command.into(),
        "--yes".into(),
        format!("--https={https_port}"),
        "off".into(),
    ]);
    if let Some(error) = &result.error {
        if error.kind == CommandErrorKind::NotFound {
            return Ok(());
        }
        return Err(TailscaleError(format!(
            "Failed to remove tailscale {command}: {}",
            error.message
        )));
    }
    if result.status != Some(0) {
        let all_output = format!("{}\n{}", result.stderr, result.stdout).to_ascii_lowercase();
        let missing = [
            "not found",
            "no serve config",
            "nothing to remove",
            "does not exist",
        ]
        .iter()
        .any(|needle| all_output.contains(needle));
        if ignore_missing && missing {
            return Ok(());
        }
        let details = preferred_output(&result);
        return Err(TailscaleError(format!(
            "Failed to remove tailscale {command} on port {https_port}: {}",
            if details.is_empty() {
                "unknown tailscale error"
            } else {
                &details
            }
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TailscaleRoute {
    pub tailscale_https_port: Option<u16>,
    pub tailscale_funnel: bool,
}

pub fn unregister_tailscale(route: TailscaleRoute) {
    unregister_tailscale_with_runner(route, &SystemTailscaleCommandRunner);
}

pub fn unregister_tailscale_with_runner(
    route: TailscaleRoute,
    runner: &dyn TailscaleCommandRunner,
) {
    let Some(port) = route.tailscale_https_port else {
        return;
    };
    let mode = if route.tailscale_funnel {
        TailscaleMode::Funnel
    } else {
        TailscaleMode::Serve
    };
    let _ = unregister_with_runner(mode, port, true, runner);
}

pub fn format_tailscale_url(base_url: &str, https_port: u16) -> String {
    let base_url = base_url.strip_suffix('/').unwrap_or(base_url);
    if https_port == 443 {
        base_url.into()
    } else {
        format!("{base_url}:{https_port}")
    }
}

impl TailscaleMode {
    fn command(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Funnel => "funnel",
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn preferred_output(result: &TailscaleCommandResult) -> String {
    normalize_space(if result.stderr.is_empty() {
        &result.stdout
    } else {
        &result.stderr
    })
}

fn combined_output(result: &TailscaleCommandResult) -> String {
    format!("{}\n{}", result.stderr, result.stdout).to_ascii_lowercase()
}

fn conflict_error(result: &TailscaleCommandResult) -> bool {
    let text = combined_output(result);
    [
        "already in use",
        "already exists",
        "port conflict",
        "address already",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn funnel_not_enabled(result: &TailscaleCommandResult) -> bool {
    combined_output(result).contains("funnel is not enabled on your tailnet")
}

fn funnel_not_enabled_error(result: &TailscaleCommandResult) -> TailscaleError {
    let details = normalize_space(&format!("{}\n{}", result.stderr, result.stdout));
    let suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" Tailscale said: {details}")
    };
    TailscaleError(format!(
        "Tailscale Funnel is not enabled on your tailnet. Enable Funnel for this node, then run \
         portless again.{suffix}"
    ))
}

fn cli_not_found() -> TailscaleError {
    TailscaleError(
        "Tailscale CLI not found. Install Tailscale (https://tailscale.com/download) and ensure \
         `tailscale` is on PATH."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn success(stdout: &str) -> TailscaleCommandResult {
        TailscaleCommandResult {
            status: Some(0),
            stdout: stdout.into(),
            ..TailscaleCommandResult::default()
        }
    }

    #[test]
    fn readiness_uses_dns_name_and_capabilities() {
        let runner = |args: &[String]| match args.join(" ").as_str() {
            "version" => success("1.0"),
            "status --json" => success(
                r#"{"Self":{"DNSName":"devbox.example.ts.net.","Capabilities":["https","https://tailscale.com/cap/funnel"]}}"#,
            ),
            other => panic!("unexpected command: {other}"),
        };
        let ready = ensure_tailscale_ready(EnsureTailscaleReadyOptions {
            require_funnel: true,
            require_https: true,
            runner: Some(&runner),
        })
        .expect("readiness should pass");
        assert_eq!(ready.dns_name, "devbox.example.ts.net");
        assert_eq!(ready.base_url, "https://devbox.example.ts.net");
    }

    #[test]
    fn readiness_falls_back_and_reports_missing_funnel() {
        let runner = |args: &[String]| match args.join(" ").as_str() {
            "version" => success("1.0"),
            "status --json" => success(
                r#"{"Self":{"HostName":"devbox","ID":"node-1"},"CurrentTailnet":{"MagicDNSSuffix":"example.ts.net."}}"#,
            ),
            other => panic!("unexpected command: {other}"),
        };
        let error = ensure_tailscale_ready(EnsureTailscaleReadyOptions {
            require_funnel: true,
            runner: Some(&runner),
            ..EnsureTailscaleReadyOptions::default()
        })
        .expect_err("funnel should be rejected");
        assert!(error.0.contains("f/funnel?node=node-1"));
    }

    #[test]
    fn used_ports_include_web_and_tcp() {
        let runner = |_args: &[String]| {
            success(
                r#"{"Web":{"dev.ts.net:443":{},"dev.ts.net:8443":{}},"TCP":{"22":{},"invalid":{}}}"#,
            )
        };
        assert_eq!(
            get_used_serve_ports_with_runner(&runner),
            HashSet::from([22, 443, 8443])
        );
    }

    #[test]
    fn allocates_serve_and_funnel_ports() {
        let serve_used = HashSet::from([443, 8443, 8444, 8445, 8446, 8447, 8448, 8449, 8450]);
        assert_eq!(
            find_available_serve_port(&serve_used, TailscaleMode::Serve)
                .expect("port should be available"),
            8451
        );
        let funnel_used = HashSet::from([443, 8443]);
        assert_eq!(
            find_available_serve_port(&funnel_used, TailscaleMode::Funnel)
                .expect("port should be available"),
            10000
        );
        assert!(
            find_available_serve_port(&HashSet::from([443, 8443, 10000]), TailscaleMode::Funnel)
                .is_err()
        );
    }

    #[test]
    fn register_and_unregister_build_faithful_commands() {
        let calls = Mutex::new(Vec::new());
        let runner = |args: &[String]| {
            calls.lock().expect("calls lock").push(args.to_vec());
            success("")
        };
        register_with_runner(TailscaleMode::Serve, 4123, 443, &runner)
            .expect("registration should pass");
        unregister_with_runner(TailscaleMode::Funnel, 8443, false, &runner)
            .expect("cleanup should pass");
        assert_eq!(
            *calls.lock().expect("calls lock"),
            vec![
                strings(&[
                    "serve",
                    "--bg",
                    "--yes",
                    "--https=443",
                    "http://127.0.0.1:4123"
                ]),
                strings(&["funnel", "--yes", "--https=8443", "off"])
            ]
        );
    }

    #[test]
    fn funnel_failures_are_actionable() {
        let runner = |_args: &[String]| TailscaleCommandResult {
            status: Some(1),
            stderr: "Funnel is not enabled on your tailnet.".into(),
            ..TailscaleCommandResult::default()
        };
        let error = register_with_runner(TailscaleMode::Funnel, 4123, 443, &runner)
            .expect_err("registration should fail");
        assert!(error.0.contains("Enable Funnel"));
    }

    #[test]
    fn formats_urls() {
        assert_eq!(
            format_tailscale_url("https://dev.example.ts.net/", 443),
            "https://dev.example.ts.net"
        );
        assert_eq!(
            format_tailscale_url("https://dev.example.ts.net", 8443),
            "https://dev.example.ts.net:8443"
        );
    }
}
