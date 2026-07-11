//! Process and framework helpers ported from Portless 0.15.1.
//!
//! Ports are probed and released before the child binds them, so callers must
//! account for the same unavoidable TOCTOU race as the TypeScript CLI.

use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use rand::Rng;
use thiserror::Error;

pub const MIN_APP_PORT: u16 = 4000;
pub const MAX_APP_PORT: u16 = 4999;
pub const RANDOM_PORT_ATTEMPTS: usize = 50;

/// WHATWG Fetch "bad port" list used by Portless 0.15.1.
pub const BLOCKED_PORTS: &[u16] = &[
    0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101,
    102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427,
    465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990,
    993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667,
    6668, 6669, 6679, 6697, 10080,
];

const BUILD_ONLY_COMMANDS: &[&str] = &[
    "tsup",
    "tsc",
    "esbuild",
    "rollup",
    "babel",
    "swc",
    "unbuild",
    "pkgroll",
    "ncc",
    "microbundle",
];

pub type ChildEnv = BTreeMap<OsString, OsString>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortlessChildEnv<'a> {
    pub port: u16,
    pub host: Option<&'a str>,
    pub portless_url: &'a str,
    pub vite_allowed_hosts: &'a str,
    pub lan_mode: bool,
    pub tailscale_url: Option<&'a str>,
    pub ngrok_url: Option<&'a str>,
    pub node_extra_ca_certs: Option<&'a Path>,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("min_port ({min}) must be <= max_port ({max})")]
    InvalidPortRange { min: u16, max: u16 },
    #[error("no free port found in range {min}-{max}")]
    NoFreePort { min: u16, max: u16 },
    #[error("could not determine the current executable: {0}")]
    CurrentExecutable(#[source] io::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Framework {
    Vite,
    VitePress,
    ReactRouter,
    Rsbuild,
    Astro,
    Angular,
    ReactNative,
    Expo,
}

impl Framework {
    pub const fn command(self) -> &'static str {
        match self {
            Self::Vite => "vite",
            Self::VitePress => "vp",
            Self::ReactRouter => "react-router",
            Self::Rsbuild => "rsbuild",
            Self::Astro => "astro",
            Self::Angular => "ng",
            Self::ReactNative => "react-native",
            Self::Expo => "expo",
        }
    }

    const fn strict_port(self) -> bool {
        matches!(self, Self::Vite | Self::VitePress | Self::ReactRouter)
    }

    fn from_command(command: &str) -> Option<Self> {
        match command {
            "vite" => Some(Self::Vite),
            "vp" => Some(Self::VitePress),
            "react-router" => Some(Self::ReactRouter),
            "rsbuild" => Some(Self::Rsbuild),
            "astro" => Some(Self::Astro),
            "ng" => Some(Self::Angular),
            "react-native" => Some(Self::ReactNative),
            "expo" => Some(Self::Expo),
            _ => None,
        }
    }
}

pub const fn is_blocked_port(port: u16) -> bool {
    let mut index = 0;
    while index < BLOCKED_PORTS.len() {
        if BLOCKED_PORTS[index] == port {
            return true;
        }
        index += 1;
    }
    false
}

fn port_is_available(port: u16) -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    TcpListener::bind(address).is_ok()
}

/// Find a free, browser-safe port in Portless's default 4000-4999 range.
pub async fn find_free_port() -> Result<u16, ProcessError> {
    find_free_port_in_range(MIN_APP_PORT, MAX_APP_PORT).await
}

/// Try 50 random ports first, then scan the range in ascending order.
pub async fn find_free_port_in_range(min_port: u16, max_port: u16) -> Result<u16, ProcessError> {
    if min_port > max_port {
        return Err(ProcessError::InvalidPortRange {
            min: min_port,
            max: max_port,
        });
    }

    for _ in 0..RANDOM_PORT_ATTEMPTS {
        let port = rand::rng().random_range(min_port..=max_port);
        if !is_blocked_port(port) && port_is_available(port) {
            return Ok(port);
        }
    }

    for port in min_port..=max_port {
        if !is_blocked_port(port) && port_is_available(port) {
            return Ok(port);
        }
    }

    Err(ProcessError::NoFreePort {
        min: min_port,
        max: max_port,
    })
}

fn basename(value: &str) -> &str {
    value
        .rsplit(['/', '\\'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(value)
}

/// Locate a framework command, including through npx/bunx/pnpx and
/// yarn/pnpm `dlx` or `exec` wrappers.
pub fn find_framework(args: &[String]) -> Option<Framework> {
    let first = basename(args.first()?.as_str());
    if let Some(framework) = Framework::from_command(first) {
        return Some(framework);
    }

    let subcommands: &[&str] = match first {
        "npx" | "bunx" | "pnpx" => &[],
        "yarn" | "pnpm" => &["dlx", "exec"],
        _ => return None,
    };
    let mut index = 1;

    if !subcommands.is_empty() {
        while args.get(index).is_some_and(|arg| arg.starts_with('-')) {
            index += 1;
        }
        let candidate = args.get(index)?;
        if !subcommands.contains(&candidate.as_str()) {
            return Framework::from_command(basename(candidate));
        }
        index += 1;
    }

    while args.get(index).is_some_and(|arg| arg.starts_with('-')) {
        index += 1;
    }
    Framework::from_command(basename(args.get(index)?))
}

/// Inject framework-specific bind flags using `PORTLESS_LAN` for Expo mode.
pub fn inject_framework_flags(args: &mut Vec<String>, port: u16) {
    let lan_mode = env::var_os("PORTLESS_LAN")
        .as_deref()
        .is_some_and(|value| matches!(value.to_str(), Some("1" | "true")));
    inject_framework_flags_with_lan_mode(args, port, lan_mode);
}

/// Deterministic variant for callers that already resolved proxy LAN state.
/// Existing `--port` and `--host` flags are preserved.
pub fn inject_framework_flags_with_lan_mode(args: &mut Vec<String>, port: u16, lan_mode: bool) {
    let Some(framework) = find_framework(args) else {
        return;
    };

    if !args.iter().any(|arg| arg == "--port") {
        args.extend(["--port".to_owned(), port.to_string()]);
        if framework.strict_port() {
            args.push("--strictPort".to_owned());
        }
    }

    if !args.iter().any(|arg| arg == "--host") {
        if framework == Framework::Expo && lan_mode {
            return;
        }
        let host = if framework == Framework::Expo {
            "localhost"
        } else {
            "127.0.0.1"
        };
        args.extend(["--host".to_owned(), host.to_owned()]);
    }
}

pub fn is_build_only_command(args: &[String]) -> bool {
    args.first()
        .map(|command| BUILD_ONLY_COMMANDS.contains(&basename(command)))
        .unwrap_or(false)
}

/// Portless uses a denylist: unknown, non-empty commands are assumed servers.
pub fn is_server_command(args: &[String]) -> bool {
    !args.is_empty() && !is_build_only_command(args)
}

/// Apply an explicit `proxy` setting when present, otherwise auto-detect.
pub fn should_proxy(args: &[String], proxy_override: Option<bool>) -> bool {
    proxy_override.unwrap_or_else(|| is_server_command(args))
}

/// Whether a command should bypass proxy setup entirely.
pub fn should_bypass_proxy(args: &[String], proxy_override: Option<bool>) -> bool {
    !should_proxy(args, proxy_override)
}

/// `PORTLESS=0`, `false`, or `skip` bypasses the proxy in the JavaScript CLI.
pub fn portless_bypass_requested(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| matches!(value.to_str(), Some("0" | "false" | "skip")))
}

fn collect_bin_paths(cwd: &Path) -> Vec<PathBuf> {
    cwd.ancestors()
        .map(|directory| directory.join("node_modules").join(".bin"))
        .filter(|directory| directory.exists())
        .collect()
}

fn path_value(env: &ChildEnv) -> Option<&OsStr> {
    env.get(OsStr::new("PATH"))
        .or_else(|| env.get(OsStr::new("Path")))
        .map(OsString::as_os_str)
}

/// Build the child PATH with nearest `node_modules/.bin` directories first,
/// followed by the current executable's directory and the inherited PATH.
pub fn augmented_path_with_executable(env: &ChildEnv, cwd: &Path, executable: &Path) -> OsString {
    let mut paths = collect_bin_paths(cwd);
    if let Some(parent) = executable.parent() {
        paths.push(parent.to_path_buf());
    }
    if let Some(base) = path_value(env) {
        paths.extend(env::split_paths(base));
    }
    let mut result = OsString::new();
    let separator = if cfg!(windows) { ";" } else { ":" };
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            result.push(separator);
        }
        result.push(path.as_os_str());
    }
    result
}

pub fn augmented_path(env: &ChildEnv, cwd: &Path) -> Result<OsString, ProcessError> {
    let executable = env::current_exe().map_err(ProcessError::CurrentExecutable)?;
    Ok(augmented_path_with_executable(env, cwd, &executable))
}

/// Clone an environment, apply overrides, and normalize PATH casing on
/// Windows after augmenting it.
pub fn build_child_env(
    base: &ChildEnv,
    overrides: &ChildEnv,
    cwd: &Path,
    executable: &Path,
) -> ChildEnv {
    let mut child = base.clone();
    child.extend(overrides.clone());
    let path = augmented_path_with_executable(&child, cwd, executable);

    if cfg!(windows) {
        child.retain(|key, _| key == "PATH" || !key.to_string_lossy().eq_ignore_ascii_case("PATH"));
    }
    child.insert(OsString::from("PATH"), path);
    child
}

/// Construct the Portless-specific child environment on top of inherited
/// variables while preserving an existing CA setting unless explicitly
/// overridden by the caller.
pub fn build_portless_child_env(
    base: &ChildEnv,
    options: &PortlessChildEnv<'_>,
    cwd: &Path,
    executable: &Path,
) -> ChildEnv {
    let mut overrides = ChildEnv::from([
        (
            OsString::from("PORT"),
            OsString::from(options.port.to_string()),
        ),
        (
            OsString::from("PORTLESS_URL"),
            OsString::from(options.portless_url),
        ),
        (
            OsString::from("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS"),
            OsString::from(options.vite_allowed_hosts),
        ),
    ]);
    if let Some(host) = options.host {
        overrides.insert(OsString::from("HOST"), OsString::from(host));
    }
    if options.lan_mode {
        overrides.insert(OsString::from("PORTLESS_LAN"), OsString::from("1"));
    }
    if let Some(url) = options.tailscale_url {
        overrides.insert(
            OsString::from("PORTLESS_TAILSCALE_URL"),
            OsString::from(url),
        );
    }
    if let Some(url) = options.ngrok_url {
        overrides.insert(OsString::from("PORTLESS_NGROK_URL"), OsString::from(url));
    }
    if let Some(ca) = options.node_extra_ca_certs {
        overrides.insert(
            OsString::from("NODE_EXTRA_CA_CERTS"),
            ca.as_os_str().to_owned(),
        );
    }
    build_child_env(base, &overrides, cwd, executable)
}

/// Escape one argument for `/bin/sh -c` using single-quote semantics.
pub fn shell_escape(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\\''"))
}

/// Build the shell program and arguments used by Portless.
pub fn shell_invocation(args: &[String]) -> (OsString, Vec<OsString>) {
    if cfg!(windows) {
        (
            OsString::from("cmd.exe"),
            vec![
                OsString::from("/d"),
                OsString::from("/s"),
                OsString::from("/c"),
                OsString::from(args.join(" ")),
            ],
        )
    } else {
        (
            OsString::from("/bin/sh"),
            vec![
                OsString::from("-c"),
                OsString::from(
                    args.iter()
                        .map(|arg| shell_escape(arg))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            ],
        )
    }
}

/// Construct the inherited-stdio shell command used for single-app mode.
pub fn shell_command(args: &[String], child_env: &ChildEnv) -> Command {
    let (program, shell_args) = shell_invocation(args);
    let mut command = Command::new(program);
    command
        .args(shell_args)
        .env_clear()
        .envs(child_env)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    configure_process_group(&mut command);
    command
}

/// Configure a child as the leader of a new process group on Unix.
pub fn configure_process_group(_command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        _command.process_group(0);
    }
}

pub fn configure_tokio_process_group(_command: &mut tokio::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        _command.as_std_mut().process_group(0);
    }
}

/// Signal a child's process group, falling back to the child itself if the
/// group is already gone. Windows has no Unix process-group equivalent.
#[cfg(unix)]
pub fn signal_process_tree(pid: u32, signal: Signal) -> nix::Result<()> {
    let pid = i32::try_from(pid)
        .map(Pid::from_raw)
        .map_err(|_| nix::errno::Errno::EINVAL)?;
    if signal::kill(Pid::from_raw(-pid.as_raw()), signal).is_ok() {
        return Ok(());
    }
    signal::kill(pid, signal)
}

#[cfg(unix)]
pub const fn signal_exit_code(signal: Signal) -> i32 {
    128 + signal as i32
}

#[cfg(test)]
mod tests {
    use std::{fs, net::TcpListener};

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[tokio::test]
    async fn selects_only_free_unblocked_ports_in_requested_range() {
        let occupied = TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        let selected = find_free_port_in_range(occupied_port, occupied_port).await;
        assert!(matches!(selected, Err(ProcessError::NoFreePort { .. })));

        assert!(matches!(
            find_free_port_in_range(4045, 4045).await,
            Err(ProcessError::NoFreePort { .. })
        ));
        let selected = find_free_port().await.unwrap();
        assert!((MIN_APP_PORT..=MAX_APP_PORT).contains(&selected));
        assert!(!is_blocked_port(selected));
    }

    #[tokio::test]
    async fn rejects_reversed_port_ranges() {
        assert!(matches!(
            find_free_port_in_range(4999, 4000).await,
            Err(ProcessError::InvalidPortRange { .. })
        ));
    }

    #[test]
    fn injects_direct_and_wrapped_framework_flags() {
        let mut vite = strings(&["vite", "dev"]);
        inject_framework_flags_with_lan_mode(&mut vite, 4321, false);
        assert_eq!(
            vite,
            strings(&[
                "vite",
                "dev",
                "--port",
                "4321",
                "--strictPort",
                "--host",
                "127.0.0.1"
            ])
        );

        let mut wrapped = strings(&["bunx", "--bun", "astro", "dev"]);
        inject_framework_flags_with_lan_mode(&mut wrapped, 4322, false);
        assert_eq!(
            wrapped,
            strings(&[
                "bunx",
                "--bun",
                "astro",
                "dev",
                "--port",
                "4322",
                "--host",
                "127.0.0.1"
            ])
        );

        assert_eq!(
            find_framework(&strings(&["yarn", "--silent", "dlx", "--quiet", "ng"])),
            Some(Framework::Angular)
        );
        assert_eq!(
            find_framework(&strings(&["pnpm", "vite"])),
            Some(Framework::Vite)
        );
    }

    #[test]
    fn preserves_user_flags_and_handles_expo_lan() {
        let mut existing = strings(&["vite", "--port", "9000", "--host", "0.0.0.0"]);
        inject_framework_flags_with_lan_mode(&mut existing, 4321, false);
        assert_eq!(
            existing,
            strings(&["vite", "--port", "9000", "--host", "0.0.0.0"])
        );

        let mut expo = strings(&["npx", "expo", "start"]);
        inject_framework_flags_with_lan_mode(&mut expo, 4321, true);
        assert_eq!(expo, strings(&["npx", "expo", "start", "--port", "4321"]));

        let mut local_expo = strings(&["expo", "start"]);
        inject_framework_flags_with_lan_mode(&mut local_expo, 4321, false);
        assert_eq!(
            local_expo,
            strings(&["expo", "start", "--port", "4321", "--host", "localhost"])
        );
    }

    #[test]
    fn detects_build_only_tools_and_proxy_overrides() {
        assert!(!is_server_command(&[]));
        assert!(!is_server_command(&strings(&[
            "/repo/node_modules/.bin/tsup",
            "--watch"
        ])));
        assert!(is_server_command(&strings(&["next", "dev"])));
        assert!(should_proxy(&strings(&["tsc"]), Some(true)));
        assert!(should_bypass_proxy(&strings(&["next"]), Some(false)));
        assert!(portless_bypass_requested(Some(OsStr::new("skip"))));
        assert!(!portless_bypass_requested(Some(OsStr::new("1"))));
    }

    #[test]
    fn augments_path_nearest_first_and_builds_child_environment() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        let package = project.join("packages").join("web");
        fs::create_dir_all(project.join("node_modules/.bin")).unwrap();
        fs::create_dir_all(package.join("node_modules/.bin")).unwrap();

        let base = ChildEnv::from([
            (OsString::from("PATH"), OsString::from("/base/bin")),
            (OsString::from("KEEP"), OsString::from("yes")),
        ]);
        let overrides = ChildEnv::from([(OsString::from("PORT"), OsString::from("4321"))]);
        let executable = Path::new("/runtime/bin/portless");
        let child = build_child_env(&base, &overrides, &package, executable);
        let paths: Vec<_> = env::split_paths(child.get(OsStr::new("PATH")).unwrap()).collect();

        assert_eq!(paths[0], package.join("node_modules/.bin"));
        assert_eq!(paths[1], project.join("node_modules/.bin"));
        assert_eq!(paths[2], PathBuf::from("/runtime/bin"));
        assert_eq!(paths[3], PathBuf::from("/base/bin"));
        assert_eq!(child.get(OsStr::new("KEEP")), Some(&OsString::from("yes")));
        assert_eq!(child.get(OsStr::new("PORT")), Some(&OsString::from("4321")));
    }

    #[test]
    fn constructs_portless_child_environment() {
        let directory = tempdir().unwrap();
        let base = ChildEnv::from([(
            OsString::from("NODE_EXTRA_CA_CERTS"),
            OsString::from("/existing.pem"),
        )]);
        let options = PortlessChildEnv {
            port: 4321,
            host: None,
            portless_url: "https://app.localhost",
            vite_allowed_hosts: ".localhost",
            lan_mode: true,
            tailscale_url: Some("https://app.tailnet.ts.net"),
            ngrok_url: None,
            node_extra_ca_certs: None,
        };
        let child = build_portless_child_env(
            &base,
            &options,
            directory.path(),
            Path::new("/runtime/bin/portless"),
        );

        assert_eq!(child.get(OsStr::new("PORT")), Some(&OsString::from("4321")));
        assert_eq!(
            child.get(OsStr::new("PORTLESS_URL")),
            Some(&OsString::from("https://app.localhost"))
        );
        assert_eq!(
            child.get(OsStr::new("NODE_EXTRA_CA_CERTS")),
            Some(&OsString::from("/existing.pem"))
        );
        assert_eq!(
            child.get(OsStr::new("PORTLESS_LAN")),
            Some(&OsString::from("1"))
        );
        assert!(!child.contains_key(OsStr::new("HOST")));
    }

    #[test]
    fn builds_platform_shell_invocation() {
        assert_eq!(shell_escape("it's ready"), "'it'\\''s ready'");
        let (program, args) = shell_invocation(&strings(&["echo", "it's ready"]));
        #[cfg(windows)]
        assert_eq!(
            (program, args),
            (
                OsString::from("cmd.exe"),
                vec![
                    OsString::from("/d"),
                    OsString::from("/s"),
                    OsString::from("/c"),
                    OsString::from("echo it's ready"),
                ],
            )
        );
        #[cfg(unix)]
        assert_eq!(
            (program, args),
            (
                OsString::from("/bin/sh"),
                vec![
                    OsString::from("-c"),
                    OsString::from("'echo' 'it'\\''s ready'"),
                ],
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn maps_unix_signals_to_shell_exit_codes() {
        assert_eq!(signal_exit_code(Signal::SIGINT), 130);
        assert_eq!(signal_exit_code(Signal::SIGTERM), 143);
    }
}
