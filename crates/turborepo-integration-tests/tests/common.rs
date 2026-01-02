//! Common test utilities for integration tests.
//!
//! Tests run in isolated temp directories with controlled environment variables,
//! matching the behavior of the existing prysk-based integration tests.

use anyhow::{Context, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::LazyLock;

/// Compiled regex for timing redaction.
/// Matches patterns like "Time: 1.23s" or "Time: 100ms".
static TIMING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Time:\s*[\d\.]+m?s").expect("Invalid timing regex"));

/// Compiled regex for hash redaction.
/// Matches 16-character lowercase hex strings (turbo cache hashes).
static HASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-f0-9]{16}").expect("Invalid hash regex"));

/// Apply redactions to make output deterministic for snapshots.
///
/// This function normalizes dynamic values in turbo output to enable
/// stable snapshot testing across runs.
///
/// # Redactions Applied
///
/// | Pattern | Example Input | Replacement |
/// |---------|---------------|-------------|
/// | Timing | `Time: 1.23s`, `Time: 100ms` | `Time: [TIME]` |
/// | Cache hashes | `0555ce94ca234049` | `[HASH]` |
///
/// # Known Limitations
///
/// - The hash regex `[a-f0-9]{16}` matches any 16-character lowercase
///   hex string, which could over-redact in edge cases (e.g., UUIDs).
///   This is intentional to catch all cache-related hashes.
///
/// # Example
///
/// ```ignore
/// let output = "my-app:build: cache miss, executing 0555ce94ca234049\nTime: 1.23s";
/// let redacted = redact_output(output);
/// assert_eq!(redacted, "my-app:build: cache miss, executing [HASH]\nTime: [TIME]");
/// ```
pub fn redact_output(output: &str) -> String {
    let output = TIMING_RE.replace_all(output, "Time: [TIME]").to_string();
    HASH_RE.replace_all(&output, "[HASH]").to_string()
}

/// Path to the turbo binary, discovered via cargo workspace layout.
///
/// # Assumptions
///
/// - Binary was built with `cargo build -p turbo` (debug profile)
/// - Workspace uses default target directory (`target/`)
///
/// # Panics
///
/// Panics if the manifest directory structure is unexpected.
///
/// # Limitations
///
/// - Does not support release builds (`target/release/turbo`)
/// - Does not support custom `CARGO_TARGET_DIR` settings
pub fn turbo_binary_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("CARGO_MANIFEST_DIR should have parent (crates/)")
        .parent()
        .expect("crates/ should have parent (workspace root)");
    workspace_root.join("target").join("debug").join("turbo")
}

/// Path to the fixtures directory
pub fn fixtures_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("CARGO_MANIFEST_DIR should have parent (crates/)")
        .parent()
        .expect("crates/ should have parent (workspace root)")
        .join("turborepo-tests")
        .join("integration")
        .join("fixtures")
}

/// A test environment that runs turbo commands in an isolated temp directory.
///
/// Each test gets its own temp directory, matching the isolation model of
/// the existing prysk-based integration tests.
///
/// # Example
///
/// ```ignore
/// let env = TurboTestEnv::new().await?;
/// env.copy_fixture("basic_monorepo").await?;
/// env.setup_git().await?;
///
/// let result = env.run_turbo(&["run", "build"]).await?;
/// result.assert_success();
/// ```
pub struct TurboTestEnv {
    workspace_path: PathBuf,
    turbo_binary: PathBuf,
    _temp_dir: tempfile::TempDir, // Keep temp dir alive for duration of test
}

impl TurboTestEnv {
    /// Create a new isolated test environment.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The turbo binary does not exist (run `cargo build -p turbo` first)
    /// - Failed to create a temp directory
    pub async fn new() -> Result<Self> {
        let turbo_binary = turbo_binary_path();
        if !turbo_binary.exists() {
            anyhow::bail!(
                "Turbo binary not found at {:?}. Run `cargo build -p turbo` first.",
                turbo_binary
            );
        }

        let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
        let workspace_path = temp_dir.path().to_path_buf();

        Ok(Self {
            workspace_path,
            turbo_binary,
            _temp_dir: temp_dir,
        })
    }

    /// Copy a fixture into the workspace.
    ///
    /// # Arguments
    ///
    /// * `fixture_name` - Name of a fixture directory within
    ///   `turborepo-tests/integration/fixtures/`. Must be a simple directory
    ///   name without path separators or traversal sequences.
    ///
    /// # Security
    ///
    /// This function validates that `fixture_name` does not contain path
    /// traversal sequences (`..`) or absolute paths to prevent accessing
    /// files outside the fixtures directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The fixture name contains path traversal sequences
    /// - The fixture directory does not exist
    /// - The resolved path escapes the fixtures directory
    /// - File copy operations fail
    pub async fn copy_fixture(&self, fixture_name: &str) -> Result<()> {
        // Validate fixture name doesn't contain path traversal sequences
        if fixture_name.contains("..") {
            anyhow::bail!(
                "Invalid fixture name '{}': path traversal sequences (..) are not allowed",
                fixture_name
            );
        }

        // Reject absolute paths
        if Path::new(fixture_name).is_absolute() {
            anyhow::bail!(
                "Invalid fixture name '{}': absolute paths are not allowed",
                fixture_name
            );
        }

        let fixtures_base = fixtures_path();
        let fixture_path = fixtures_base.join(fixture_name);

        // Verify the fixture exists
        if !fixture_path.exists() {
            anyhow::bail!("Fixture not found: {:?}", fixture_path);
        }

        // Canonicalize paths and verify the fixture is within the fixtures directory
        let canonical_fixture = fixture_path
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize fixture path: {:?}", fixture_path))?;
        let canonical_base = fixtures_base
            .canonicalize()
            .context("Failed to canonicalize fixtures base path")?;

        if !canonical_fixture.starts_with(&canonical_base) {
            anyhow::bail!(
                "Invalid fixture '{}': resolved path {:?} escapes fixtures directory {:?}",
                fixture_name,
                canonical_fixture,
                canonical_base
            );
        }

        // Use spawn_blocking to avoid blocking the async runtime during file I/O
        let workspace_path = self.workspace_path.clone();
        tokio::task::spawn_blocking(move || copy_dir_recursive(&canonical_fixture, &workspace_path))
            .await
            .context("File copy task panicked")??;

        Ok(())
    }

    /// Initialize git in the workspace (required for turbo).
    ///
    /// Creates a git repository with an initial commit containing all files.
    pub async fn setup_git(&self) -> Result<()> {
        self.exec(&["git", "init"]).await?;
        self.exec(&["git", "config", "user.email", "test@test.com"])
            .await?;
        self.exec(&["git", "config", "user.name", "Test User"])
            .await?;
        self.exec(&["git", "add", "."]).await?;
        self.exec(&["git", "commit", "-m", "Initial commit"])
            .await?;
        Ok(())
    }

    /// Install npm dependencies in the workspace.
    #[allow(dead_code)]
    pub async fn npm_install(&self) -> Result<ExecResult> {
        self.exec(&["npm", "install"]).await
    }

    /// Run turbo with the given arguments.
    ///
    /// # Environment
    ///
    /// The following environment variables are automatically set:
    /// - `TURBO_TELEMETRY_MESSAGE_DISABLED=1`
    /// - `TURBO_GLOBAL_WARNING_DISABLED=1`
    /// - `TURBO_PRINT_VERSION_DISABLED=1`
    pub async fn run_turbo(&self, args: &[&str]) -> Result<ExecResult> {
        let output = tokio::process::Command::new(&self.turbo_binary)
            .args(args)
            .current_dir(&self.workspace_path)
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .output()
            .await
            .context("Failed to execute turbo")?;

        Ok(ExecResult::from(output))
    }

    /// Run turbo with specific environment variables.
    ///
    /// Additional environment variables are merged with the defaults.
    pub async fn run_turbo_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExecResult> {
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args)
            .current_dir(&self.workspace_path)
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1");

        for (key, value) in env {
            cmd.env(key, value);
        }

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Execute a command in the workspace directory.
    pub async fn exec(&self, cmd: &[&str]) -> Result<ExecResult> {
        let (program, args) = cmd.split_first().context("Empty command")?;
        let output = tokio::process::Command::new(program)
            .args(args)
            .current_dir(&self.workspace_path)
            .output()
            .await
            .context("Failed to execute command")?;

        Ok(ExecResult::from(output))
    }
}

/// Result of executing a command.
#[derive(Debug)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl From<Output> for ExecResult {
    fn from(output: Output) -> Self {
        Self {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        }
    }
}

impl ExecResult {
    /// Get combined stdout and stderr.
    ///
    /// Note: This concatenates stdout followed by stderr, not interleaved
    /// in chronological order.
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}{}", self.stdout, self.stderr)
        }
    }

    /// Assert the command succeeded (exit code 0).
    pub fn assert_success(&self) -> &Self {
        assert_eq!(
            self.exit_code, 0,
            "Command failed with exit code {}.\nstdout: {}\nstderr: {}",
            self.exit_code, self.stdout, self.stderr
        );
        self
    }

    /// Assert the command failed (non-zero exit code).
    #[allow(dead_code)]
    pub fn assert_failure(&self) -> &Self {
        assert_ne!(
            self.exit_code, 0,
            "Command unexpectedly succeeded.\nstdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }
}

/// Recursively copy a directory.
///
/// This function is designed to be called within `spawn_blocking` to avoid
/// blocking the async runtime.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src).context("Failed to read source directory")? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Skip symlinks to prevent following links outside the fixture directory
        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
