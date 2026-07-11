//! Local certificate authority and server certificate management.
//!
//! This is a command-compatible port of portless 0.15.1's `certs.ts`.  OpenSSL
//! remains the source of truth, which keeps certificates produced by the Rust
//! and JavaScript implementations interchangeable.

use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

pub const CA_KEY_FILE: &str = "ca-key.pem";
pub const CA_CERT_FILE: &str = "ca.pem";
pub const SERVER_KEY_FILE: &str = "server-key.pem";
pub const SERVER_CERT_FILE: &str = "server.pem";
pub const CA_TRUST_MARKER: &str = "ca.trusted";
pub const HOST_CERTS_DIR: &str = "host-certs";
pub const CA_COMMON_NAME: &str = "portless Local CA";

const CA_VALIDITY_DAYS: &str = "3650";
const SERVER_VALIDITY_DAYS: &str = "365";
const EXPIRY_BUFFER_SECONDS: &str = "604800";
const MAX_CN_LENGTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Linux,
    Windows,
    Unsupported,
}

impl Platform {
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Unsupported
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub input: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl CommandRequest {
    fn new(program: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: BTreeMap::new(),
            input: None,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

pub trait CommandRunner {
    fn run(&self, request: &CommandRequest) -> io::Result<CommandOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdCommandRunner;

impl CommandRunner for StdCommandRunner {
    fn run(&self, request: &CommandRequest) -> io::Result<CommandOutput> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .envs(&request.env)
            .stdin(if request.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if let (Some(input), Some(mut stdin)) = (&request.input, child.stdin.take()) {
            use std::io::Write;
            stdin.write_all(input)?;
        }
        let stdout = child.stdout.take().map(read_pipe);
        let stderr = child.stderr.take().map(read_pipe);
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= request.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} timed out after {:?}", request.program, request.timeout),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        Ok(CommandOutput {
            status: status.code(),
            stdout: join_pipe(stdout)?,
            stderr: join_pipe(stderr)?,
        })
    }
}

#[derive(Debug, Error)]
pub enum CertError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("{program} failed: {detail}")]
    Command { program: String, detail: String },
    #[error("unsupported platform")]
    UnsupportedPlatform,
    #[error("CA certificate not found. Run portless trust to generate it.")]
    CaNotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPaths {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
    pub ca_generated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCertPaths {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustResult {
    pub trusted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrustResult {
    pub removed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxCaTrustConfig {
    cert_dir: PathBuf,
    update_command: &'static str,
}

/// Repairs ownership of files created while portless is running under sudo.
///
/// Implementations must not follow symbolic links. Errors are intentionally
/// ignored by [`CertManager`], matching upstream's best-effort behavior.
pub trait OwnershipRepair {
    fn repair(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdOwnershipRepair;

impl OwnershipRepair for StdOwnershipRepair {
    fn repair(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
        repair_ownership_without_following_symlinks(path, uid, gid)
    }
}

pub struct CertManager<R = StdCommandRunner, O = StdOwnershipRepair> {
    runner: R,
    ownership: O,
    platform: Platform,
    is_root: bool,
    env: BTreeMap<String, String>,
}

impl Default for CertManager<StdCommandRunner, StdOwnershipRepair> {
    fn default() -> Self {
        Self::new(StdCommandRunner)
    }
}

impl<R: CommandRunner> CertManager<R, StdOwnershipRepair> {
    #[must_use]
    pub fn new(runner: R) -> Self {
        let is_root = {
            #[cfg(unix)]
            {
                nix::unistd::Uid::effective().is_root()
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        Self {
            runner,
            ownership: StdOwnershipRepair,
            platform: Platform::current(),
            is_root,
            env: env::vars().collect(),
        }
    }

    #[must_use]
    pub fn with_context(
        runner: R,
        platform: Platform,
        is_root: bool,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            runner,
            ownership: StdOwnershipRepair,
            platform,
            is_root,
            env,
        }
    }
}

impl<R: CommandRunner, O: OwnershipRepair> CertManager<R, O> {
    #[must_use]
    pub fn with_context_and_ownership(
        runner: R,
        ownership: O,
        platform: Platform,
        is_root: bool,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            runner,
            ownership,
            platform,
            is_root,
            env,
        }
    }

    pub fn ensure_certs(&self, state_dir: &Path) -> Result<CertPaths, CertError> {
        fs::create_dir_all(state_dir)?;
        let ca_cert = state_dir.join(CA_CERT_FILE);
        let ca_key = state_dir.join(CA_KEY_FILE);
        let server_cert = state_dir.join(SERVER_CERT_FILE);
        let server_key = state_dir.join(SERVER_KEY_FILE);

        let ca_missing = !readable(&ca_cert)
            || !readable(&ca_key)
            || !self.cert_valid(&ca_cert)
            || !self.cert_signature_strong(&ca_cert);
        if ca_missing {
            self.generate_ca(state_dir)?;
        }
        if ca_missing
            || !readable(&server_cert)
            || !readable(&server_key)
            || !self.cert_valid(&server_cert)
            || !self.cert_signature_strong(&server_cert)
            || !self.cert_sans_complete(&server_cert)
        {
            self.generate_server_cert(state_dir)?;
        }
        Ok(CertPaths {
            cert_path: server_cert,
            key_path: server_key,
            ca_path: ca_cert,
            ca_generated: ca_missing,
        })
    }

    pub fn generate_host_cert(
        &self,
        state_dir: &Path,
        hostname: &str,
    ) -> Result<HostCertPaths, CertError> {
        let host_dir = state_dir.join(HOST_CERTS_DIR);
        fs::create_dir_all(&host_dir)?;
        set_mode(&host_dir, 0o755)?;
        self.repair_ownership([host_dir.as_path()]);
        let safe_name = sanitize_host_for_filename(hostname);
        let key = host_dir.join(format!("{safe_name}-key.pem"));
        let cert = host_dir.join(format!("{safe_name}.pem"));
        let csr = host_dir.join(format!("{safe_name}.csr"));
        let ext = host_dir.join(format!("{safe_name}-ext.cnf"));
        self.openssl([
            "ecparam",
            "-genkey",
            "-name",
            "prime256v1",
            "-noout",
            "-out",
            &display(&key),
        ])?;
        let cn: String = hostname.chars().take(MAX_CN_LENGTH).collect();
        self.openssl([
            "req",
            "-new",
            "-key",
            &display(&key),
            "-out",
            &display(&csr),
            "-subj",
            &format!("/CN={cn}"),
        ])?;
        let mut sans = vec![format!("DNS:{hostname}")];
        if let Some((_, parent)) = hostname.split_once('.') {
            sans.push(format!("DNS:*.{parent}"));
        }
        write_extensions(&ext, &sans)?;
        let serial = ensure_serial(state_dir)?;
        self.repair_ownership([serial.as_path()]);
        let result = self.openssl([
            "x509",
            "-req",
            "-sha256",
            "-in",
            &display(&csr),
            "-CA",
            &display(&state_dir.join(CA_CERT_FILE)),
            "-CAkey",
            &display(&state_dir.join(CA_KEY_FILE)),
            "-CAserial",
            &display(&serial),
            "-out",
            &display(&cert),
            "-days",
            SERVER_VALIDITY_DAYS,
            "-extfile",
            &display(&ext),
        ]);
        remove_if_present(&csr);
        remove_if_present(&ext);
        result?;
        set_mode(&key, 0o600)?;
        set_mode(&cert, 0o644)?;
        self.repair_ownership([key.as_path(), cert.as_path(), serial.as_path()]);
        Ok(HostCertPaths {
            cert_path: cert,
            key_path: key,
        })
    }

    #[must_use]
    pub fn is_ca_trusted(&self, state_dir: &Path) -> bool {
        let ca = state_dir.join(CA_CERT_FILE);
        if !readable(&ca) {
            return false;
        }
        if read_trimmed(&state_dir.join(CA_TRUST_MARKER))
            .zip(ca_fingerprint(state_dir))
            .is_some_and(|(marker, fingerprint)| marker == fingerprint)
        {
            return true;
        }
        match self.platform {
            Platform::MacOs => self.is_ca_trusted_macos(&ca),
            Platform::Linux => self.is_ca_trusted_linux(state_dir),
            Platform::Windows => self.is_ca_trusted_windows(&ca),
            Platform::Unsupported => false,
        }
    }

    pub fn trust_ca(&self, state_dir: &Path) -> TrustResult {
        let ca = state_dir.join(CA_CERT_FILE);
        if !readable(&ca) {
            return TrustResult {
                trusted: false,
                error: Some(CertError::CaNotFound.to_string()),
            };
        }
        let result = match self.platform {
            Platform::MacOs if self.is_root => self.run_checked(request(
                "security",
                [
                    "add-trusted-cert",
                    "-d",
                    "-r",
                    "trustRoot",
                    "-k",
                    "/Library/Keychains/System.keychain",
                    &display(&ca),
                ],
                Duration::from_secs(60),
            )),
            Platform::MacOs => {
                let keychain = self.login_keychain_path();
                self.run_checked(request(
                    "security",
                    [
                        "add-trusted-cert",
                        "-r",
                        "trustRoot",
                        "-k",
                        &display(&keychain),
                        &display(&ca),
                    ],
                    Duration::from_secs(120),
                ))
            }
            Platform::Linux => self.trust_ca_linux(state_dir),
            Platform::Windows => self.run_checked(request(
                "certutil",
                ["-addstore", "-user", "Root", &display(&ca)],
                Duration::from_secs(30),
            )),
            Platform::Unsupported => Err(CertError::UnsupportedPlatform),
        };
        match result.and_then(|_| self.write_trust_marker(state_dir)) {
            Ok(()) => TrustResult {
                trusted: true,
                error: None,
            },
            Err(error) => TrustResult {
                trusted: false,
                error: Some(trust_error_message(self.platform, &error.to_string())),
            },
        }
    }

    pub fn untrust_ca(&self, state_dir: &Path) -> UntrustResult {
        let ca = state_dir.join(CA_CERT_FILE);
        if !readable(&ca) || !self.is_ca_trusted(state_dir) {
            clear_trust_marker(state_dir);
            return UntrustResult {
                removed: true,
                error: None,
            };
        }
        let result = match self.platform {
            Platform::MacOs => self.untrust_ca_macos(&ca),
            Platform::Linux => self.untrust_ca_linux(state_dir),
            Platform::Windows => self.untrust_ca_windows(&ca),
            Platform::Unsupported => UntrustResult {
                removed: false,
                error: Some("unsupported platform".into()),
            },
        };
        if result.removed {
            clear_trust_marker(state_dir);
        }
        result
    }

    fn generate_ca(&self, state_dir: &Path) -> Result<(), CertError> {
        let key = state_dir.join(CA_KEY_FILE);
        let cert = state_dir.join(CA_CERT_FILE);
        self.openssl([
            "ecparam",
            "-genkey",
            "-name",
            "prime256v1",
            "-noout",
            "-out",
            &display(&key),
        ])?;
        self.openssl([
            "req",
            "-new",
            "-x509",
            "-sha256",
            "-key",
            &display(&key),
            "-out",
            &display(&cert),
            "-days",
            CA_VALIDITY_DAYS,
            "-subj",
            "/CN=portless Local CA",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
        ])?;
        set_mode(&key, 0o600)?;
        set_mode(&cert, 0o644)?;
        self.repair_ownership([key.as_path(), cert.as_path()]);
        clear_trust_marker(state_dir);
        Ok(())
    }

    fn generate_server_cert(&self, state_dir: &Path) -> Result<(), CertError> {
        let key = state_dir.join(SERVER_KEY_FILE);
        let cert = state_dir.join(SERVER_CERT_FILE);
        let csr = state_dir.join("server.csr");
        let ext = state_dir.join("server-ext.cnf");
        self.openssl([
            "ecparam",
            "-genkey",
            "-name",
            "prime256v1",
            "-noout",
            "-out",
            &display(&key),
        ])?;
        self.openssl([
            "req",
            "-new",
            "-key",
            &display(&key),
            "-out",
            &display(&csr),
            "-subj",
            "/CN=localhost",
        ])?;
        write_extensions(
            &ext,
            &[
                "DNS:localhost".into(),
                "DNS:*.localhost".into(),
                "DNS:*.local".into(),
            ],
        )?;
        let serial = ensure_serial(state_dir)?;
        self.repair_ownership([serial.as_path()]);
        let result = self.openssl([
            "x509",
            "-req",
            "-sha256",
            "-in",
            &display(&csr),
            "-CA",
            &display(&state_dir.join(CA_CERT_FILE)),
            "-CAkey",
            &display(&state_dir.join(CA_KEY_FILE)),
            "-CAserial",
            &display(&serial),
            "-out",
            &display(&cert),
            "-days",
            SERVER_VALIDITY_DAYS,
            "-extfile",
            &display(&ext),
        ]);
        remove_if_present(&csr);
        remove_if_present(&ext);
        result?;
        set_mode(&key, 0o600)?;
        set_mode(&cert, 0o644)?;
        self.repair_ownership([key.as_path(), cert.as_path(), serial.as_path()]);
        Ok(())
    }

    fn write_trust_marker(&self, state_dir: &Path) -> Result<(), CertError> {
        write_trust_marker(state_dir)?;
        self.repair_ownership([state_dir.join(CA_TRUST_MARKER).as_path()]);
        Ok(())
    }

    fn repair_ownership<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let Some((uid, gid)) = ownership_ids(self.is_root, &self.env) else {
            return;
        };
        for path in paths {
            let _ = self.ownership.repair(path, uid, gid);
        }
    }

    fn cert_valid(&self, cert: &Path) -> bool {
        self.openssl([
            "x509",
            "-checkend",
            EXPIRY_BUFFER_SECONDS,
            "-noout",
            "-in",
            &display(cert),
        ])
        .is_ok()
    }

    fn cert_signature_strong(&self, cert: &Path) -> bool {
        self.cert_text(cert)
            .and_then(|text| {
                text.lines()
                    .find(|line| line.to_ascii_lowercase().contains("signature algorithm:"))
                    .map(str::to_ascii_lowercase)
            })
            .is_some_and(|algorithm| !algorithm.contains("sha1"))
    }

    fn cert_sans_complete(&self, cert: &Path) -> bool {
        self.cert_text(cert)
            .is_some_and(|text| text.contains("DNS:*.local"))
    }

    fn cert_text(&self, cert: &Path) -> Option<String> {
        self.openssl(["x509", "-in", &display(cert), "-noout", "-text"])
            .ok()
    }

    fn openssl(
        &self,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<String, CertError> {
        let mut req = request("openssl", args, Duration::from_secs(15));
        if self.platform == Platform::Windows
            && let Some(config) = openssl_config(&self.env)
        {
            req.env.insert("OPENSSL_CONF".into(), display(&config));
        }
        self.run_checked(req).map_err(|error| {
            let hint = if self.platform == Platform::Windows {
                "Make sure openssl is installed and working.\nInstall via: winget install -e --id \
                 ShiningLight.OpenSSL.Dev\nIf already installed, set OPENSSL_CONF to the path of \
                 your openssl.cnf file."
            } else {
                "Make sure openssl is installed (ships with macOS and most Linux distributions)."
            };
            CertError::Command {
                program: "openssl".into(),
                detail: format!("{error}\n\n{hint}"),
            }
        })
    }

    fn run_checked(&self, req: CommandRequest) -> Result<String, CertError> {
        let output = self.runner.run(&req).map_err(CertError::Io)?;
        if output.success() {
            Ok(output.stdout_string())
        } else {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(CertError::Command {
                program: req.program,
                detail: if detail.is_empty() {
                    format!("exit status {:?}", output.status)
                } else {
                    detail
                },
            })
        }
    }

    fn login_keychain_path(&self) -> PathBuf {
        if let Ok(value) = self.run_checked(request(
            "security",
            ["default-keychain"],
            Duration::from_secs(15),
        )) {
            let trimmed = value.trim();
            if let Some(path) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return PathBuf::from(path);
            }
        }
        let home = self
            .env
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| format!("/Users/{}", env_value(&self.env, "USER", "unknown")));
        PathBuf::from(home)
            .join("Library")
            .join("Keychains")
            .join("login.keychain-db")
    }

    fn macos_verify_request(&self, ca: &Path) -> CommandRequest {
        if self.is_root
            && let Some(user) = self.env.get("SUDO_USER")
        {
            return request(
                "sudo",
                [
                    "-u",
                    user,
                    "security",
                    "verify-cert",
                    "-c",
                    &display(ca),
                    "-L",
                    "-p",
                    "ssl",
                ],
                Duration::from_secs(15),
            );
        }
        request(
            "security",
            ["verify-cert", "-c", &display(ca), "-L", "-p", "ssl"],
            Duration::from_secs(15),
        )
    }

    fn is_ca_trusted_macos(&self, ca: &Path) -> bool {
        self.run_checked(self.macos_verify_request(ca)).is_ok()
    }

    fn is_ca_trusted_windows(&self, ca: &Path) -> bool {
        let Ok(fingerprint) = self.sha1_fingerprint(ca) else {
            return false;
        };
        self.run_checked(request(
            "certutil",
            ["-store", "-user", "Root"],
            Duration::from_secs(10),
        ))
        .is_ok_and(|listing| normalize_whitespace(&listing).contains(&fingerprint))
    }

    fn sha1_fingerprint(&self, ca: &Path) -> Result<String, CertError> {
        self.openssl([
            "x509",
            "-in",
            &display(ca),
            "-noout",
            "-fingerprint",
            "-sha1",
        ])
        .map(|value| {
            value
                .trim()
                .split('=')
                .next_back()
                .unwrap_or_default()
                .replace(':', "")
                .to_ascii_lowercase()
        })
    }

    fn linux_config(&self) -> LinuxCaTrustConfig {
        let release = fs::read_to_string("/etc/os-release")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if release.contains("arch") {
            linux_configs()[1].clone()
        } else if ["fedora", "rhel", "centos"]
            .iter()
            .any(|name| release.contains(name))
        {
            linux_configs()[2].clone()
        } else if release.contains("suse") {
            linux_configs()[3].clone()
        } else {
            linux_configs()[0].clone()
        }
    }

    fn is_ca_trusted_linux(&self, state_dir: &Path) -> bool {
        let installed = self.linux_config().cert_dir.join("portless-ca.crt");
        same_trimmed_file(&state_dir.join(CA_CERT_FILE), &installed)
    }

    fn trust_ca_linux(&self, state_dir: &Path) -> Result<String, CertError> {
        let config = self.linux_config();
        fs::create_dir_all(&config.cert_dir)?;
        fs::copy(
            state_dir.join(CA_CERT_FILE),
            config.cert_dir.join("portless-ca.crt"),
        )?;
        self.run_checked(request(
            config.update_command,
            std::iter::empty::<String>(),
            Duration::from_secs(30),
        ))
    }

    fn untrust_ca_macos(&self, ca: &Path) -> UntrustResult {
        let mut errors = Vec::new();
        if let Err(error) = self.run_checked(request(
            "security",
            ["remove-trusted-cert", &display(ca)],
            Duration::from_secs(60),
        )) {
            errors.push(error.to_string());
        }
        for keychain in [
            self.login_keychain_path(),
            PathBuf::from("/Library/Keychains/System.keychain"),
        ] {
            for _ in 0..20 {
                let deleted = self.run_checked(request(
                    "security",
                    [
                        "delete-certificate",
                        "-c",
                        CA_COMMON_NAME,
                        &display(&keychain),
                    ],
                    Duration::from_secs(60),
                ));
                if let Err(error) = deleted {
                    errors.push(error.to_string());
                    break;
                }
            }
        }
        if self.is_ca_trusted_macos(ca) {
            UntrustResult {
                removed: false,
                error: Some(if errors.is_empty() {
                    "Could not remove CA from keychain (try sudo)".into()
                } else {
                    errors.join("; ")
                }),
            }
        } else {
            UntrustResult {
                removed: true,
                error: None,
            }
        }
    }

    fn untrust_ca_linux(&self, state_dir: &Path) -> UntrustResult {
        let ours = state_dir.join(CA_CERT_FILE);
        let mut errors = Vec::new();
        let mut deleted = false;
        for config in linux_configs() {
            let installed = config.cert_dir.join("portless-ca.crt");
            if same_trimmed_file(&ours, &installed) {
                match fs::remove_file(&installed) {
                    Ok(()) => deleted = true,
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        if deleted {
            let config = self.linux_config();
            if let Err(error) = self.run_checked(request(
                config.update_command,
                std::iter::empty::<String>(),
                Duration::from_secs(30),
            )) {
                errors.push(error.to_string());
            }
        }
        if self.is_ca_trusted_linux(state_dir) {
            UntrustResult {
                removed: false,
                error: Some(if errors.is_empty() {
                    "CA still trusted (remove portless-ca.crt and run the distro CA update \
                     command, often with sudo)"
                        .into()
                } else {
                    errors.join("; ")
                }),
            }
        } else {
            UntrustResult {
                removed: true,
                error: None,
            }
        }
    }

    fn untrust_ca_windows(&self, ca: &Path) -> UntrustResult {
        if !self.is_ca_trusted_windows(ca) {
            return UntrustResult {
                removed: true,
                error: None,
            };
        }
        if let Err(error) = self.run_checked(request(
            "certutil",
            ["-delstore", "-user", "Root", CA_COMMON_NAME],
            Duration::from_secs(30),
        )) {
            return UntrustResult {
                removed: false,
                error: Some(error.to_string()),
            };
        }
        if self.is_ca_trusted_windows(ca) {
            UntrustResult {
                removed: false,
                error: Some("certutil could not remove the portless CA from Root".into()),
            }
        } else {
            UntrustResult {
                removed: true,
                error: None,
            }
        }
    }
}

pub fn ensure_certs(state_dir: impl AsRef<Path>) -> Result<CertPaths, CertError> {
    CertManager::default().ensure_certs(state_dir.as_ref())
}

#[must_use]
pub fn is_ca_trusted(state_dir: impl AsRef<Path>) -> bool {
    CertManager::default().is_ca_trusted(state_dir.as_ref())
}

#[must_use]
pub fn trust_ca(state_dir: impl AsRef<Path>) -> TrustResult {
    CertManager::default().trust_ca(state_dir.as_ref())
}

#[must_use]
pub fn untrust_ca(state_dir: impl AsRef<Path>) -> UntrustResult {
    CertManager::default().untrust_ca(state_dir.as_ref())
}

pub fn generate_host_cert(
    state_dir: impl AsRef<Path>,
    hostname: &str,
) -> Result<HostCertPaths, CertError> {
    CertManager::default().generate_host_cert(state_dir.as_ref(), hostname)
}

#[must_use]
pub fn sanitize_host_for_filename(hostname: &str) -> String {
    hostname
        .chars()
        .filter_map(|character| {
            if character == '.' {
                Some('_')
            } else if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                Some(character)
            } else {
                None
            }
        })
        .collect()
}

fn request(
    program: impl Into<String>,
    args: impl IntoIterator<Item = impl Into<String>>,
    timeout: Duration,
) -> CommandRequest {
    let mut request = CommandRequest::new(program, args);
    request.timeout = timeout;
    request
}

fn read_pipe(mut pipe: impl io::Read + Send + 'static) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_pipe(handle: Option<thread::JoinHandle<io::Result<Vec<u8>>>>) -> io::Result<Vec<u8>> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_| io::Error::other("command output reader panicked"))?,
        None => Ok(Vec::new()),
    }
}

fn readable(path: &Path) -> bool {
    fs::File::open(path).is_ok()
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|contents| contents.trim().to_owned())
        .filter(|contents| !contents.is_empty())
}

fn same_trimmed_file(left: &Path, right: &Path) -> bool {
    read_trimmed(left)
        .zip(read_trimmed(right))
        .is_some_and(|(left, right)| left == right)
}

fn ca_fingerprint(state_dir: &Path) -> Option<String> {
    let bytes = fs::read(state_dir.join(CA_CERT_FILE)).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

fn write_trust_marker(state_dir: &Path) -> Result<(), CertError> {
    if let Some(fingerprint) = ca_fingerprint(state_dir) {
        fs::write(state_dir.join(CA_TRUST_MARKER), format!("{fingerprint}\n"))?;
    }
    Ok(())
}

fn clear_trust_marker(state_dir: &Path) {
    let _ = fs::remove_file(state_dir.join(CA_TRUST_MARKER));
}

fn ensure_serial(state_dir: &Path) -> Result<PathBuf, CertError> {
    let path = state_dir.join("ca.srl");
    if !path.exists() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let material = format!("{now}:{}:{}", std::process::id(), display(state_dir));
        let digest = format!("{:X}", Sha256::digest(material.as_bytes()));
        let serial = digest.get(..16).unwrap_or(&digest);
        fs::write(&path, format!("{serial}\n"))?;
    }
    Ok(path)
}

fn write_extensions(path: &Path, sans: &[String]) -> Result<(), CertError> {
    fs::write(
        path,
        format!(
            concat!(
                "authorityKeyIdentifier=keyid,issuer\n",
                "basicConstraints=CA:FALSE\n",
                "keyUsage=digitalSignature,keyEncipherment\n",
                "extendedKeyUsage=serverAuth\n",
                "subjectAltName={}\n"
            ),
            sans.join(",")
        ),
    )?;
    Ok(())
}

fn remove_if_present(path: &Path) {
    let _ = fs::remove_file(path);
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

fn ownership_ids(is_root: bool, environment: &BTreeMap<String, String>) -> Option<(u32, u32)> {
    if !is_root {
        return None;
    }
    let uid = environment.get("SUDO_UID")?.parse().ok()?;
    let gid = match environment.get("SUDO_GID") {
        Some(value) => value.parse().ok()?,
        None => uid,
    };
    Some((uid, gid))
}

#[cfg(unix)]
fn repair_ownership_without_following_symlinks(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(());
    }
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))
}

#[cfg(not(unix))]
fn repair_ownership_without_following_symlinks(
    _path: &Path,
    _uid: u32,
    _gid: u32,
) -> io::Result<()> {
    Ok(())
}

fn linux_configs() -> [LinuxCaTrustConfig; 4] {
    [
        LinuxCaTrustConfig {
            cert_dir: "/usr/local/share/ca-certificates".into(),
            update_command: "update-ca-certificates",
        },
        LinuxCaTrustConfig {
            cert_dir: "/etc/ca-certificates/trust-source/anchors".into(),
            update_command: "update-ca-trust",
        },
        LinuxCaTrustConfig {
            cert_dir: "/etc/pki/ca-trust/source/anchors".into(),
            update_command: "update-ca-trust",
        },
        LinuxCaTrustConfig {
            cert_dir: "/etc/pki/trust/anchors".into(),
            update_command: "update-ca-certificates",
        },
    ]
}

fn openssl_config(environment: &BTreeMap<String, String>) -> Option<PathBuf> {
    if environment
        .get("OPENSSL_CONF")
        .is_some_and(|path| readable(Path::new(path)))
    {
        return None;
    }
    [
        r"C:\Program Files\Git\mingw64\etc\ssl\openssl.cnf",
        r"C:\Program Files\Git\usr\ssl\openssl.cnf",
        r"C:\Program Files\OpenSSL-Win64\bin\cnf\openssl.cnf",
        r"C:\Program Files\OpenSSL-Win64\openssl.cnf",
        r"C:\Program Files (x86)\OpenSSL-Win32\bin\cnf\openssl.cnf",
        r"C:\Program Files\OpenSSL\bin\cnf\openssl.cnf",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| readable(path))
}

fn env_value(environment: &BTreeMap<String, String>, key: &str, fallback: &str) -> String {
    environment
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn normalize_whitespace(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn trust_error_message(platform: Platform, message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("etimedout") {
        if platform == Platform::MacOs {
            return "The macOS security command timed out. This can happen when the Keychain \
                    Services daemon is unresponsive or a system authorization dialog was not \
                    dismissed in time. Try restarting Keychain Access (or run: sudo killall \
                    securityd) and then: portless trust"
                .into();
        }
        return "The trust command timed out. Try: portless trust".into();
    }
    if ["authorization", "permission", "eacces"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return "Permission denied. Try: portless trust".into();
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
    };

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingOwnership {
        calls: Arc<Mutex<Vec<(PathBuf, u32, u32)>>>,
    }

    impl OwnershipRepair for RecordingOwnership {
        fn repair(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
            self.calls
                .lock()
                .expect("ownership call lock")
                .push((path.to_path_buf(), uid, gid));
            Ok(())
        }
    }

    #[test]
    fn sanitizes_hostnames_like_typescript() {
        assert_eq!(
            sanitize_host_for_filename("Chat.foo.localhost!"),
            "Chat_foo_localhost"
        );
    }

    #[test]
    fn extension_file_has_expected_server_constraints() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ext.cnf");
        write_extensions(&path, &["DNS:localhost".into(), "DNS:*.localhost".into()])
            .expect("write extensions");
        let contents = fs::read_to_string(path).expect("read extensions");
        assert!(contents.contains("basicConstraints=CA:FALSE"));
        assert!(contents.contains("subjectAltName=DNS:localhost,DNS:*.localhost"));
    }

    #[test]
    fn fingerprint_marker_tracks_exact_pem_bytes() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(CA_CERT_FILE), "pem\n").expect("write pem");
        write_trust_marker(dir.path()).expect("write marker");
        assert_eq!(
            read_trimmed(&dir.path().join(CA_TRUST_MARKER)),
            ca_fingerprint(dir.path())
        );
    }

    #[test]
    fn generates_openssl_ca_server_and_exact_host_certificates() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manager = CertManager::default();
        let paths = manager.ensure_certs(dir.path()).expect("generate certs");
        assert!(paths.ca_generated);
        assert!(manager.cert_valid(&paths.ca_path));
        assert!(manager.cert_sans_complete(&paths.cert_path));

        let host = manager
            .generate_host_cert(dir.path(), "chat.myapp.localhost")
            .expect("generate host cert");
        let text = manager.cert_text(&host.cert_path).expect("host cert text");
        assert!(text.contains("DNS:chat.myapp.localhost"));
        assert!(text.contains("DNS:*.myapp.localhost"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(paths.key_path)
                    .expect("key metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(host.cert_path)
                    .expect("certificate metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }
    }

    #[test]
    fn sudo_generation_repairs_all_persistent_certificate_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ownership = RecordingOwnership::default();
        let observer = ownership.calls.clone();
        let environment = BTreeMap::from([("SUDO_UID".into(), "1234".into())]);
        let manager = CertManager::with_context_and_ownership(
            StdCommandRunner,
            ownership,
            Platform::Linux,
            true,
            environment,
        );

        manager.ensure_certs(dir.path()).expect("generate certs");
        manager
            .generate_host_cert(dir.path(), "app.localhost")
            .expect("generate host cert");
        manager
            .write_trust_marker(dir.path())
            .expect("write trust marker");

        let calls = observer.lock().expect("ownership calls");
        let paths = calls
            .iter()
            .map(|(path, uid, gid)| {
                assert_eq!((*uid, *gid), (1234, 1234));
                path.strip_prefix(dir.path())
                    .expect("path in state directory")
                    .to_path_buf()
            })
            .collect::<BTreeSet<_>>();
        for expected in [
            CA_KEY_FILE,
            CA_CERT_FILE,
            SERVER_KEY_FILE,
            SERVER_CERT_FILE,
            "ca.srl",
            HOST_CERTS_DIR,
            "host-certs/app_localhost-key.pem",
            "host-certs/app_localhost.pem",
            CA_TRUST_MARKER,
        ] {
            assert!(
                paths.contains(Path::new(expected)),
                "missing ownership repair for {expected}"
            );
        }
    }

    #[test]
    fn ownership_context_requires_root_and_valid_sudo_uid() {
        let environment = BTreeMap::from([
            ("SUDO_UID".into(), "501".into()),
            ("SUDO_GID".into(), "20".into()),
        ]);
        assert_eq!(ownership_ids(true, &environment), Some((501, 20)));
        assert_eq!(ownership_ids(false, &environment), None);
        assert_eq!(
            ownership_ids(
                true,
                &BTreeMap::from([("SUDO_UID".into(), "invalid".into())])
            ),
            None
        );
        assert_eq!(
            ownership_ids(
                true,
                &BTreeMap::from([
                    ("SUDO_UID".into(), "501".into()),
                    ("SUDO_GID".into(), "invalid".into()),
                ])
            ),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn standard_ownership_repair_skips_symbolic_links() {
        use std::os::unix::fs::{MetadataExt, symlink};

        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, "contents").expect("write target");
        symlink(&target, &link).expect("create symlink");
        let before = fs::metadata(&target).expect("target metadata");

        StdOwnershipRepair
            .repair(&link, before.uid(), before.gid())
            .expect("skip symlink");

        let after = fs::metadata(&target).expect("target metadata");
        assert_eq!((before.uid(), before.gid()), (after.uid(), after.gid()));
    }
}
