//! Common test utilities for integration tests.
//!
//! Tests run in isolated temp directories with controlled environment
//! variables, matching the behavior of the existing prysk-based integration
//! tests.

use std::{
    path::{Path, PathBuf},
    process::Output,
    sync::{Arc, LazyLock, OnceLock},
};

use anyhow::{Context, Result};
use regex::Regex;
use which::which;

/// Compiled regex for timing redaction.
/// Matches patterns like "Time: 1.23s" or "Time: 100ms".
static TIMING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Time:\s*[\d\.]+m?s").expect("Invalid timing regex"));

/// Compiled regex for hash redaction.
/// Matches turbo cache hashes (16-character lowercase hex) in context.
///
/// Uses word boundaries (`\b`) to avoid matching hex strings that are part of
/// larger identifiers (e.g., UUIDs, git SHAs). This prevents over-redaction
/// while still catching all turbo-generated cache hashes.
static HASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-f0-9]{16}\b").expect("Invalid hash regex"));

/// Compiled regex for temp directory path redaction.
/// Matches various temp paths across platforms:
/// - macOS: `/private/var/folders/.../T/.tmpXXX/...` or
///   `/var/folders/.../T/.tmpXXX/...`
/// - Linux: `/tmp/.tmpXXXXXX/...`
/// - Windows: `C:\Users\...\AppData\Local\Temp\.tmpXXX\...` (after path
///   normalization)
///
/// These paths appear in:
/// - npm error messages with locations
/// - Lockfile warnings
static TEMP_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match either:
    // 1. macOS style: /private?/var/folders/.../T/.tmp.../...
    // 2. Linux style: /tmp/.tmp.../...
    // 3. Windows style: C:/Users/.../AppData/Local/Temp/.?tmp.../... (normalized)
    //    Note: tempfile crate may create "tmpXXX" or ".tmpXXX" directories
    Regex::new(r"(?:(?:/private)?/var/folders/[a-zA-Z0-9_]+/[a-zA-Z0-9_]+/T/\.tmp[a-zA-Z0-9_]+|/tmp/\.tmp[a-zA-Z0-9_]+|[A-Z]:/Users/[^/]+/AppData/Local/Temp/\.?tmp[a-zA-Z0-9_]+)(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid temp path regex")
});

/// Compiled regex for matching temp paths split across lines.
/// Error messages may split paths at various points like:
/// - `/private/var/folders/03/\n    bcr7.../T/.tmpXXX/...`
/// - `/private/var/\n    folders/.../T/.tmpXXX/...`
///
/// This regex matches the entire multi-line pattern.
static TEMP_PATH_MULTILINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match macOS temp path split across two lines.
    // The path is: /private?/var/folders/XX/HASH/T/.tmpXXX/file
    // It can break after XX/ (leaving HASH/T/... on next line)
    // or after /var/ (leaving folders/XX/HASH/T/... on next line)
    Regex::new(r"(?:/private)?/var/(?:folders/[a-zA-Z0-9_]+/)?\n\s*(?:folders/[a-zA-Z0-9_]+/)?[a-zA-Z0-9_]+/T/\.tmp[a-zA-Z0-9_]+(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid multiline temp path regex")
});

/// Compiled regex for matching Windows temp paths split across lines.
/// Windows error messages may split paths after the drive letter:
/// - `C:\n    \Users/runneradmin/AppData/Local/Temp/.tmpXXX/...`
/// - `C:\n      \Users/runneradmin/AppData/Local/Temp/.tmpXXX/...`
///
/// This regex matches the entire multi-line pattern.
static TEMP_PATH_WINDOWS_MULTILINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match Windows temp path split across two lines after drive letter.
    // The path is: C:\Users\...\AppData\Local\Temp\.tmpXXX\file (or with forward
    // slashes) It can break after C: leaving \Users/... on next line
    Regex::new(r"[A-Z]:\n\s*[/\\]Users/[^/\n]+/AppData/Local/Temp/\.?tmp[a-zA-Z0-9_]+(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid Windows multiline temp path regex")
});

/// Compiled regex for stripping ANSI escape codes from output.
/// Matches CSI sequences like `\x1b[31m` (color) and `\x1b[0m` (reset).
static ANSI_ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match ANSI escape sequences:
    // - CSI sequences: \x1b[ followed by parameters and a command letter
    // - OSC sequences: \x1b] followed by text and \x07 or \x1b\\
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*(?:\x07|\x1b\\)")
        .expect("Invalid ANSI escape regex")
});

/// Apply redactions to make output deterministic for snapshots.
///
/// This function normalizes dynamic values in turbo output to enable
/// stable snapshot testing across runs.
///
/// # Redactions Applied
///
/// | Pattern | Example Input | Replacement |
/// |---------|---------------|-------------|
/// | ANSI escape codes | `\x1b[31m` (red) | (removed) |
/// | CRLF line endings | `\r\n` | `\n` |
/// | Timing | `Time: 1.23s`, `Time: 100ms` | `Time: [TIME]` |
/// | Cache hashes | `0555ce94ca234049` | `[HASH]` |
/// | Temp paths | `/var/folders/.../T/.tmpXXX` | `[TEMP_DIR]` |
/// | Path separators | `packages\util` | `packages/util` |
///
/// # Known Limitations
///
/// - The hash regex `\b[a-f0-9]{16}\b` uses word boundaries to match standalone
///   16-character hex strings. This avoids over-redacting UUIDs or longer hex
///   identifiers while catching all turbo cache hashes.
///
/// # Example
///
/// ```ignore
/// let output = "my-app:build: cache miss, executing 0555ce94ca234049\nTime: 1.23s";
/// let redacted = redact_output(output);
/// assert_eq!(redacted, "my-app:build: cache miss, executing [HASH]\nTime: [TIME]");
/// ```
pub fn redact_output(output: &str) -> String {
    // Strip ANSI escape codes first (colors, cursor movements, etc.)
    let output = ANSI_ESCAPE_RE.replace_all(output, "");
    // Normalize CRLF to LF for cross-platform snapshot consistency
    let output = output.replace("\r\n", "\n");
    // Normalize Windows path separators to Unix style for consistent snapshots.
    // Only replace backslashes that appear in path-like contexts (after
    // packages, .turbo, etc.)
    let output = normalize_path_separators(&output);
    let output = TIMING_RE.replace_all(&output, "Time: [TIME]");
    let output = HASH_RE.replace_all(&output, "[HASH]");

    // First handle multiline temp paths (paths split across lines)
    let output = TEMP_PATH_MULTILINE_RE.replace_all(&output, "[TEMP_DIR]");
    // Handle Windows multiline temp paths (split after drive letter)
    let output = TEMP_PATH_WINDOWS_MULTILINE_RE.replace_all(&output, "[TEMP_DIR]");
    // Then handle single-line temp paths
    TEMP_PATH_RE.replace_all(&output, "[TEMP_DIR]").into_owned()
}

/// Normalize Windows path separators to Unix style.
///
/// Converts backslashes to forward slashes in common path patterns like:
/// - `packages\util` -> `packages/util`
/// - `packages\util\.turbo` -> `packages/util/.turbo`
/// - `C:\Users\...` -> `C:/Users/...`
fn normalize_path_separators(output: &str) -> String {
    // Replace backslash path separators with forward slashes.
    static PATH_SEP_RE: LazyLock<Regex> = LazyLock::new(|| {
        // Match backslash:
        // 1. After word char and before word char or dot (catches `util\.turbo`)
        // 2. After drive letter colon (catches `C:\Users`)
        Regex::new(r"(\w|:)\\([\w.])").expect("Invalid path separator regex")
    });

    // Iteratively replace until no more matches (handles `a\b\c` -> `a/b/c`)
    let mut result = output.to_string();
    loop {
        let new_result = PATH_SEP_RE.replace_all(&result, "$1/$2").to_string();
        if new_result == result {
            break;
        }
        result = new_result;
    }
    result
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

    #[cfg(windows)]
    let binary_name = "turbo.exe";
    #[cfg(not(windows))]
    let binary_name = "turbo";

    workspace_root
        .join("target")
        .join("debug")
        .join(binary_name)
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
    /// Path to use for turbo config directory (for telemetry config, etc.)
    /// This ensures telemetry can initialize even when HOME is not set.
    config_dir_path: PathBuf,
    /// Path for corepack shims (npm, yarn, pnpm binaries).
    /// When corepack is enabled, this directory contains shims that respect
    /// the `packageManager` field in package.json.
    corepack_install_dir: PathBuf,
    /// Whether corepack has been enabled for this environment.
    /// When true, commands will include the corepack shim directory in PATH.
    corepack_enabled: bool,
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
        // Use temp dir for turbo config to ensure telemetry can initialize
        // even when HOME is not set (common in CI environments)
        let config_dir_path = temp_dir.path().join(".turbo-config");
        // Create a directory for corepack shims to isolate from system
        let corepack_install_dir = temp_dir.path().join("corepack");

        Ok(Self {
            workspace_path,
            turbo_binary,
            config_dir_path,
            corepack_install_dir,
            corepack_enabled: false,
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
        tokio::task::spawn_blocking(move || {
            copy_dir_recursive(&canonical_fixture, &workspace_path)
        })
        .await
        .context("File copy task panicked")??;

        Ok(())
    }

    /// Initialize git in the workspace (required for turbo).
    ///
    /// Creates a git repository with an initial commit containing all files.
    /// Also creates `.npmrc` with `script-shell=bash` for cross-platform
    /// consistency, matching the behavior of the prysk-based integration tests.
    pub async fn setup_git(&self) -> Result<()> {
        self.exec(&["git", "init"]).await?;
        self.exec(&["git", "config", "user.email", "test@test.com"])
            .await?;
        self.exec(&["git", "config", "user.name", "Test User"])
            .await?;
        // Disable autocrlf to ensure consistent line endings across platforms.
        // This prevents Windows from converting LF to CRLF, which would cause
        // different file hashes and input counts between platforms.
        self.exec(&["git", "config", "core.autocrlf", "false"])
            .await?;

        // Set script-shell=bash for cross-platform consistency.
        // This ensures npm scripts using bash syntax (;, &&, etc.) work on Windows.
        // See: https://docs.npmjs.com/cli/v9/using-npm/config#script-shell
        self.write_file(".npmrc", "script-shell=bash\n").await?;

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

    /// Configure turbo-specific test environment on a command.
    ///
    /// This sets environment variables for deterministic test execution:
    /// - `TURBO_CONFIG_DIR_PATH` - Set to temp dir for telemetry config
    /// - `TURBO_TELEMETRY_MESSAGE_DISABLED=1`
    /// - `TURBO_GLOBAL_WARNING_DISABLED=1`
    /// - `TURBO_PRINT_VERSION_DISABLED=1`
    /// - `NO_COLOR=1` - For consistent output formatting
    /// - Removes `GITHUB_ACTIONS` - Prevents CI-specific output formatting
    /// - Removes `CI` - Prevents CI-specific behavior
    /// - If corepack is enabled, prepends the shim directory to PATH
    fn configure_turbo_env(&self, cmd: &mut tokio::process::Command) {
        // If corepack is enabled, prepend the shim directory to PATH
        // This ensures turbo spawns the correct npm/yarn/pnpm version
        if self.corepack_enabled {
            self.configure_corepack_path(cmd);
        }

        cmd.env("TURBO_CONFIG_DIR_PATH", &self.config_dir_path)
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            // Disable colored output for consistent snapshot testing
            .env("NO_COLOR", "1")
            // Remove CI-specific environment variables to ensure consistent output
            // format across local development and CI environments
            .env_remove("GITHUB_ACTIONS")
            .env_remove("CI");
    }

    /// Run turbo with the given arguments.
    ///
    /// # Environment
    ///
    /// This method inherits the current environment but overrides specific
    /// variables for deterministic test execution. See
    /// [`Self::configure_turbo_env`] for the full list of environment
    /// modifications.
    ///
    /// Inheriting the environment (rather than clearing it) ensures that
    /// npm, node, git, and other tools work correctly across all platforms.
    pub async fn run_turbo(&self, args: &[&str]) -> Result<ExecResult> {
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&self.workspace_path);
        self.configure_turbo_env(&mut cmd);

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Run turbo from a subdirectory within the workspace.
    ///
    /// This is useful for testing package inference behavior, where turbo
    /// infers the target package from the current working directory.
    ///
    /// # Arguments
    ///
    /// * `subdir` - Relative path from workspace root to run turbo from
    /// * `args` - Arguments to pass to turbo
    pub async fn run_turbo_from_dir(&self, subdir: &str, args: &[&str]) -> Result<ExecResult> {
        let dir = self.workspace_path.join(subdir);
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&dir);
        self.configure_turbo_env(&mut cmd);

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Run turbo with specific environment variables.
    ///
    /// Additional environment variables are merged with the inherited
    /// environment. Test-specific variables override any inherited values.
    pub async fn run_turbo_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExecResult> {
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&self.workspace_path);
        self.configure_turbo_env(&mut cmd);

        // Add test-specific environment variables (these override defaults)
        for (key, value) in env {
            cmd.env(key, value);
        }

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Execute a command in the workspace directory.
    ///
    /// If corepack has been enabled via [`Self::enable_corepack`], the corepack
    /// shim directory is prepended to PATH so that npm/yarn/pnpm commands use
    /// the version specified in package.json's `packageManager` field.
    pub async fn exec(&self, cmd: &[&str]) -> Result<ExecResult> {
        let (program, args) = cmd.split_first().context("Empty command")?;
        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(&self.workspace_path);

        // If corepack is enabled, prepend the shim directory to PATH
        if self.corepack_enabled {
            self.configure_corepack_path(&mut command);
        }

        let output = command
            .output()
            .await
            .context("Failed to execute command")?;

        Ok(ExecResult::from(output))
    }

    /// Configure PATH to include corepack shim directory.
    fn configure_corepack_path(&self, cmd: &mut tokio::process::Command) {
        let current_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!(
            "{}{}{}",
            self.corepack_install_dir.display(),
            std::path::MAIN_SEPARATOR,
            current_path
        );
        cmd.env("PATH", new_path);
    }

    /// Enable corepack for the specified package manager.
    ///
    /// This runs `corepack enable <manager>` with `--install-directory`
    /// pointing to the test environment's isolated corepack directory.
    /// After this, npm/yarn/pnpm commands will use the version specified in
    /// package.json's `packageManager` field.
    ///
    /// This mimics the behavior of `setup_package_manager.sh` in the prysk
    /// tests.
    ///
    /// # Arguments
    ///
    /// * `package_manager` - The package manager name (e.g., "npm", "yarn",
    ///   "pnpm")
    ///
    /// # Example
    ///
    /// ```ignore
    /// env.set_package_manager("npm@10.5.0").await?;
    /// env.enable_corepack("npm").await?;
    /// // Now npm commands will use npm@10.5.0
    /// ```
    pub async fn enable_corepack(&mut self, package_manager_name: &str) -> Result<()> {
        // Create the corepack install directory
        tokio::fs::create_dir_all(&self.corepack_install_dir)
            .await
            .context("Failed to create corepack install directory")?;

        // Use `which` to find corepack binary - this handles Windows .cmd extensions
        let corepack_binary =
            which("corepack").context("corepack not found in PATH. Is Node.js installed?")?;

        let install_dir_arg = format!(
            "--install-directory={}",
            self.corepack_install_dir.display()
        );

        let output = tokio::process::Command::new(&corepack_binary)
            .args(["enable", package_manager_name, &install_dir_arg])
            .current_dir(&self.workspace_path)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to execute corepack enable (binary: {})",
                    corepack_binary.display()
                )
            })?;

        if !output.status.success() {
            anyhow::bail!(
                "corepack enable failed (exit code {:?}):\nstdout: {}\nstderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        self.corepack_enabled = true;
        Ok(())
    }

    /// Write content to a file in the workspace.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let full_path = self.workspace_path.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, content).await?;
        Ok(())
    }

    /// Read content from a file in the workspace.
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let full_path = self.workspace_path.join(path);
        let content = tokio::fs::read_to_string(&full_path).await?;
        Ok(content)
    }

    /// Set the packageManager field in the root package.json and enable
    /// corepack.
    ///
    /// This mimics the behavior of setup_package_manager.sh in the prysk tests:
    /// 1. Sets the `packageManager` field in package.json
    /// 2. Commits the change (if git is initialized)
    /// 3. Runs `corepack enable` to create shims that respect the version
    ///
    /// **Important**: This function should be called AFTER `setup_git()` to
    /// match the prysk test behavior. The prysk tests commit the initial
    /// files first, then modify and commit package.json separately.
    ///
    /// # Arguments
    ///
    /// * `package_manager` - Full package manager spec (e.g., "npm@10.5.0")
    pub async fn set_package_manager(&mut self, package_manager: &str) -> Result<()> {
        let package_json_path = self.workspace_path.join("package.json");
        let content = tokio::fs::read_to_string(&package_json_path).await?;
        let mut json: serde_json::Value =
            serde_json::from_str(&content).context("Failed to parse package.json")?;

        json["packageManager"] = serde_json::Value::String(package_manager.to_string());

        // serde_json::to_string_pretty uses \n line endings on all platforms
        let mut updated = serde_json::to_string_pretty(&json)?;
        // Ensure file ends with newline (POSIX convention)
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        tokio::fs::write(&package_json_path, updated).await?;

        // Commit the package.json change if git is initialized.
        // This matches the behavior of setup_package_manager.sh which commits
        // after modifying package.json.
        if self.workspace_path.join(".git").exists() {
            self.exec(&["git", "add", "package.json"]).await?;
            let commit_msg = format!("Set packageManager to {}", package_manager);
            self.exec(&["git", "commit", "-m", &commit_msg]).await?;
        }

        // Extract the package manager name (e.g., "npm" from "npm@10.5.0")
        let package_manager_name = package_manager.split('@').next().unwrap_or(package_manager);

        // Enable corepack so the packageManager field is respected
        self.enable_corepack(package_manager_name).await?;

        Ok(())
    }

    /// Delete a file in the workspace.
    pub async fn remove_file(&self, path: &str) -> Result<()> {
        let full_path = self.workspace_path.join(path);
        tokio::fs::remove_file(&full_path).await?;
        Ok(())
    }

    /// Check if a file exists in the workspace.
    pub async fn file_exists(&self, path: &str) -> bool {
        let full_path = self.workspace_path.join(path);
        tokio::fs::metadata(&full_path).await.is_ok()
    }

    /// Check if a directory exists in the workspace.
    pub async fn dir_exists(&self, path: &str) -> bool {
        let full_path = self.workspace_path.join(path);
        match tokio::fs::metadata(&full_path).await {
            Ok(metadata) => metadata.is_dir(),
            Err(_) => false,
        }
    }

    /// Rename/move a file in the workspace.
    pub async fn rename_file(&self, from: &str, to: &str) -> Result<()> {
        let from_path = self.workspace_path.join(from);
        let to_path = self.workspace_path.join(to);
        tokio::fs::rename(&from_path, &to_path).await?;
        Ok(())
    }

    /// Touch a file (create empty or update mtime).
    pub async fn touch_file(&self, path: &str) -> Result<()> {
        let full_path = self.workspace_path.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Create or truncate to update mtime
        tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&full_path)
            .await?;
        Ok(())
    }

    /// Stage and commit a git change.
    pub async fn git_commit(&self, message: &str) -> Result<()> {
        self.exec(&["git", "add", "."]).await?;
        self.exec(&["git", "commit", "-m", message]).await?;
        Ok(())
    }

    /// Get the workspace path.
    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
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
    pub fn assert_failure(&self) -> &Self {
        assert_ne!(
            self.exit_code, 0,
            "Command unexpectedly succeeded.\nstdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }

    /// Assert a specific exit code.
    pub fn assert_exit_code(&self, expected: i32) -> &Self {
        assert_eq!(
            self.exit_code, expected,
            "Expected exit code {}, got {}.\nstdout: {}\nstderr: {}",
            expected, self.exit_code, self.stdout, self.stderr
        );
        self
    }

    /// Check if stdout contains a pattern.
    pub fn stdout_contains(&self, pattern: &str) -> bool {
        self.stdout.contains(pattern)
    }

    /// Check if stderr contains a pattern.
    pub fn stderr_contains(&self, pattern: &str) -> bool {
        self.stderr.contains(pattern)
    }

    /// Check if combined output contains a pattern.
    pub fn output_contains(&self, pattern: &str) -> bool {
        self.combined_output().contains(pattern)
    }
}

/// Recursively copy a directory, normalizing line endings for text files.
///
/// This function is designed to be called within `spawn_blocking` to avoid
/// blocking the async runtime.
///
/// Text files (determined by extension) have their line endings normalized to
/// LF. This ensures consistent file hashes across platforms, particularly
/// important on Windows where git's autocrlf setting might have converted LF to
/// CRLF when cloning the repo.
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
            // For text files, normalize line endings to LF for cross-platform consistency
            if is_text_file(&src_path) {
                copy_with_normalized_line_endings(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
    }
    Ok(())
}

/// Check if a file is likely a text file based on its extension.
fn is_text_file(path: &Path) -> bool {
    let text_extensions = [
        "json",
        "txt",
        "md",
        "js",
        "ts",
        "jsx",
        "tsx",
        "css",
        "html",
        "yml",
        "yaml",
        "toml",
        "lock",
        "gitignore",
        "npmrc",
        "sh",
        "bash",
        "zsh",
    ];

    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| text_extensions.contains(&ext.to_lowercase().as_str()))
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with('.') && !name.contains('.') // dotfiles like
                // .gitignore
            })
}

/// Copy a file while normalizing CRLF line endings to LF.
fn copy_with_normalized_line_endings(src: &Path, dst: &Path) -> Result<()> {
    let content =
        std::fs::read_to_string(src).with_context(|| format!("Failed to read file: {:?}", src))?;
    let normalized = content.replace("\r\n", "\n");
    std::fs::write(dst, normalized).with_context(|| format!("Failed to write file: {:?}", dst))?;
    Ok(())
}

// =============================================================================
// Shared Fixture Cache for Test Performance
// =============================================================================

/// A cached, pre-warmed test environment that can be copied for fast test
/// setup.
///
/// This structure stores a prepared fixture directory with git initialized and
/// cache primed, allowing tests to copy from this cached state instead of
/// repeating expensive setup operations.
struct CachedFixtureEnv {
    /// Path to the cached fixture directory
    path: PathBuf,
    /// Keep the temp dir alive
    _temp_dir: tempfile::TempDir,
}

/// Global cache for the basic_monorepo fixture with pre-warmed turbo cache.
static BASIC_MONOREPO_CACHE: OnceLock<Arc<CachedFixtureEnv>> = OnceLock::new();

impl CachedFixtureEnv {
    /// Create a new cached fixture environment.
    async fn new(fixture_name: &str, prime_args: &[&str]) -> Result<Self> {
        let turbo_binary = turbo_binary_path();
        if !turbo_binary.exists() {
            anyhow::bail!(
                "Turbo binary not found at {:?}. Run `cargo build -p turbo` first.",
                turbo_binary
            );
        }

        let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
        let workspace_path = temp_dir.path().to_path_buf();

        // Copy fixture
        let fixtures_base = fixtures_path();
        let fixture_path = fixtures_base.join(fixture_name);
        let canonical_fixture = fixture_path.canonicalize()?;

        let workspace_clone = workspace_path.clone();
        tokio::task::spawn_blocking(move || {
            copy_dir_recursive(&canonical_fixture, &workspace_clone)
        })
        .await
        .context("File copy task panicked")??;

        // Initialize git
        let git_init_commands = [
            vec!["git", "init"],
            vec!["git", "config", "user.email", "test@test.com"],
            vec!["git", "config", "user.name", "Test User"],
        ];

        for cmd in &git_init_commands {
            let (program, args) = cmd.split_first().unwrap();
            let output = tokio::process::Command::new(program)
                .args(args)
                .current_dir(&workspace_path)
                .output()
                .await
                .context("Failed to execute git command")?;
            if !output.status.success() {
                anyhow::bail!(
                    "Git command {:?} failed: {}",
                    cmd,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }

        // Set script-shell=bash for cross-platform consistency.
        // This ensures npm scripts using bash syntax (;, &&, etc.) work on Windows.
        // See: https://docs.npmjs.com/cli/v9/using-npm/config#script-shell
        tokio::fs::write(workspace_path.join(".npmrc"), "script-shell=bash\n")
            .await
            .context("Failed to write .npmrc")?;

        // Stage and commit all files
        let git_commit_commands = [
            vec!["git", "add", "."],
            vec!["git", "commit", "-m", "Initial commit"],
        ];

        for cmd in &git_commit_commands {
            let (program, args) = cmd.split_first().unwrap();
            let output = tokio::process::Command::new(program)
                .args(args)
                .current_dir(&workspace_path)
                .output()
                .await
                .context("Failed to execute git command")?;
            if !output.status.success() {
                anyhow::bail!(
                    "Git command {:?} failed: {}",
                    cmd,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }

        // Prime the cache with turbo run.
        // We inherit the environment (rather than clearing it) to ensure that
        // npm, node, git, and other tools work correctly across all platforms.
        // On Windows, this is critical for PATHEXT, COMSPEC, and other system
        // variables that are required for process spawning to work correctly.
        let mut cmd = tokio::process::Command::new(&turbo_binary);
        cmd.args(prime_args).current_dir(&workspace_path);

        // Set turbo-specific test environment (overrides any inherited values)
        cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            // Disable colored output for consistent snapshot testing
            .env("NO_COLOR", "1")
            // Remove CI-specific environment variables to ensure consistent output
            // format across local development and CI environments
            .env_remove("GITHUB_ACTIONS")
            .env_remove("CI");

        let output = cmd.output().await.context("Failed to prime cache")?;
        if !output.status.success() {
            anyhow::bail!(
                "Failed to prime turbo cache: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(Self {
            path: workspace_path,
            _temp_dir: temp_dir,
        })
    }
}

/// Get or initialize the shared basic_monorepo fixture cache.
///
/// This function ensures the fixture is only set up once, even when called
/// from multiple tests running in parallel. Subsequent calls return a
/// reference to the cached environment.
async fn get_basic_monorepo_cache(prime_args: &[&str]) -> Result<Arc<CachedFixtureEnv>> {
    // Fast path: cache already initialized
    if let Some(cache) = BASIC_MONOREPO_CACHE.get() {
        return Ok(Arc::clone(cache));
    }

    // Slow path: initialize the cache
    // Note: In parallel test execution, multiple tests might try to initialize.
    // OnceLock ensures only one succeeds, others will get the cached value.
    let cache = Arc::new(CachedFixtureEnv::new("basic_monorepo", prime_args).await?);

    // Try to set the cache, if another thread beat us, use their value
    match BASIC_MONOREPO_CACHE.set(Arc::clone(&cache)) {
        Ok(()) => Ok(cache),
        Err(_) => Ok(Arc::clone(BASIC_MONOREPO_CACHE.get().unwrap())),
    }
}

/// Create a test environment by copying from the shared cache.
///
/// This is significantly faster than `setup_env_with_cache()` because:
/// 1. Fixture copying happens once per test run, not per test
/// 2. Git initialization happens once
/// 3. Cache priming happens once
///
/// The returned environment has its own temp directory with a copy of the
/// cached fixture, so tests can safely modify it without affecting other tests.
pub async fn create_env_from_cache(prime_args: &[&str]) -> Result<TurboTestEnv> {
    let cache = get_basic_monorepo_cache(prime_args).await?;

    let turbo_binary = turbo_binary_path();
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let workspace_path = temp_dir.path().to_path_buf();

    // Copy from cached fixture (includes .git and .turbo cache)
    let cache_path = cache.path.clone();
    let workspace_clone = workspace_path.clone();
    tokio::task::spawn_blocking(move || copy_dir_recursive(&cache_path, &workspace_clone))
        .await
        .context("File copy task panicked")??;

    // Use temp dir for turbo config to ensure telemetry can initialize
    let config_dir_path = temp_dir.path().join(".turbo-config");
    // Create a directory for corepack shims
    let corepack_install_dir = temp_dir.path().join("corepack");

    Ok(TurboTestEnv {
        workspace_path,
        turbo_binary,
        config_dir_path,
        corepack_install_dir,
        corepack_enabled: false,
        _temp_dir: temp_dir,
    })
}

// =============================================================================
// Unit Tests for Security and Core Functionality
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // P0: Path Traversal Security Tests
    // =========================================================================

    #[tokio::test]
    async fn test_copy_fixture_rejects_path_traversal_dotdot() {
        let env = TurboTestEnv::new().await.unwrap();
        let result = env.copy_fixture("../../../etc/passwd").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("path traversal"),
            "Expected path traversal error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_copy_fixture_rejects_nested_path_traversal() {
        let env = TurboTestEnv::new().await.unwrap();
        let result = env.copy_fixture("foo/../../../etc/passwd").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("path traversal"),
            "Expected path traversal error, got: {}",
            err
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn test_copy_fixture_rejects_absolute_path_unix() {
        let env = TurboTestEnv::new().await.unwrap();
        let result = env.copy_fixture("/etc/passwd").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("absolute paths"),
            "Expected absolute path error, got: {}",
            err
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_copy_fixture_rejects_absolute_path_windows() {
        let env = TurboTestEnv::new().await.unwrap();
        let result = env.copy_fixture("C:\\Windows\\System32").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("absolute paths"),
            "Expected absolute path error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_copy_fixture_rejects_nonexistent_fixture() {
        let env = TurboTestEnv::new().await.unwrap();
        let result = env.copy_fixture("nonexistent_fixture_12345").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Fixture not found"),
            "Expected fixture not found error, got: {}",
            err
        );
    }

    // =========================================================================
    // Redaction Tests
    // =========================================================================

    #[test]
    fn test_redact_output_normalizes_crlf() {
        let input = "line1\r\nline2\r\nline3";
        let output = redact_output(input);
        assert!(!output.contains('\r'), "CRLF should be normalized to LF");
        assert_eq!(output, "line1\nline2\nline3");
    }

    #[test]
    fn test_redact_output_preserves_lf() {
        let input = "line1\nline2\nline3";
        let output = redact_output(input);
        assert_eq!(output, "line1\nline2\nline3");
    }

    #[test]
    fn test_redact_output_handles_mixed_line_endings() {
        let input = "line1\r\nline2\nline3\r\n";
        let output = redact_output(input);
        assert!(!output.contains('\r'));
        assert_eq!(output, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_redact_output_timing() {
        assert_eq!(redact_output("Time: 1.23s"), "Time: [TIME]");
        assert_eq!(redact_output("Time: 100ms"), "Time: [TIME]");
        assert_eq!(redact_output("Time:  42.5s"), "Time: [TIME]");
    }

    #[test]
    fn test_redact_output_hash() {
        let input = "cache miss, executing 0555ce94ca234049";
        let output = redact_output(input);
        assert_eq!(output, "cache miss, executing [HASH]");
    }

    #[test]
    fn test_redact_output_combined() {
        let input = "my-app:build: cache miss, executing 0555ce94ca234049\r\nTime: 1.23s";
        let output = redact_output(input);
        assert_eq!(
            output,
            "my-app:build: cache miss, executing [HASH]\nTime: [TIME]"
        );
    }

    #[test]
    fn test_redact_output_multiline_temp_path_after_var() {
        // Test multiline temp paths split after /var/
        let input = "Lockfile not found at /private/var/\n      \
                     folders/0r/90dc16493lx7gw025k4z8sw40000gn/T/.tmpE7t2eW/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("/private/var"),
            "Multiline temp path should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_redact_output_multiline_temp_path_after_folders_xx() {
        // Test multiline temp paths split after /folders/XX/ (CI pattern)
        let input = "Lockfile not found at /private/var/folders/03/\n      \
                     bcr7nd0x5lz0x5lkgq6vrh5w0000gn/T/.tmpQcamWa/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("/private/var"),
            "Multiline temp path should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_redact_output_multiline_temp_path() {
        // Test multiline temp paths split after /var/
        let input = "Lockfile not found at /private/var/\n      \
                     folders/0r/90dc16493lx7gw025k4z8sw40000gn/T/.tmpE7t2eW/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("/private/var"),
            "Multiline temp path should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_redact_output_windows_temp_path() {
        // Test Windows temp paths (after path normalization from backslash to forward
        // slash)
        let input = "Lockfile not found at \
                     C:/Users/runneradmin/AppData/Local/Temp/.tmpAbC123/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("runneradmin"),
            "Windows temp path should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_redact_output_windows_temp_path_no_dot() {
        // Test Windows temp paths without leading dot (tempfile crate variation)
        let input =
            "Lockfile not found at C:/Users/runner/AppData/Local/Temp/tmpXYZ789/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("runner"),
            "Windows temp path (no dot) should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_redact_output_windows_temp_path_with_backslashes() {
        // Test Windows temp paths with native backslash separators
        // (simulates raw Windows output before normalization in combined function)
        let input =
            r"Lockfile not found at C:\Users\runner\AppData\Local\Temp\tmpXYZ789\package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("runner"),
            "Windows temp path with backslashes should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    #[test]
    fn test_normalize_drive_letter_path() {
        // Verify that drive letter paths are normalized correctly
        let input = r"C:\Users\test";
        let output = normalize_path_separators(input);
        assert_eq!(
            output, "C:/Users/test",
            "Drive letter paths should be normalized"
        );
    }

    #[test]
    fn test_redact_output_windows_multiline_temp_path() {
        // Test Windows temp path split across lines after drive letter (CI pattern)
        // This happens when npm error messages wrap the path
        let input = "Lockfile not found at C:\n      \
                     /Users/runneradmin/AppData/Local/Temp/.tmpLaTNdD/package-lock.json";
        let output = redact_output(input);
        assert!(
            !output.contains("runneradmin"),
            "Windows multiline temp path should be redacted. Got: {}",
            output
        );
        assert!(output.contains("[TEMP_DIR]"), "Should contain [TEMP_DIR]");
    }

    // =========================================================================
    // ExecResult Tests
    // =========================================================================

    #[test]
    fn test_combined_output_stdout_only() {
        let result = ExecResult {
            stdout: "output".into(),
            stderr: "".into(),
            exit_code: 0,
        };
        assert_eq!(result.combined_output(), "output");
    }

    #[test]
    fn test_combined_output_stderr_only() {
        let result = ExecResult {
            stdout: "".into(),
            stderr: "error".into(),
            exit_code: 1,
        };
        assert_eq!(result.combined_output(), "error");
    }

    #[test]
    fn test_combined_output_both() {
        let result = ExecResult {
            stdout: "out".into(),
            stderr: "err".into(),
            exit_code: 0,
        };
        // Note: concatenated without separator
        assert_eq!(result.combined_output(), "outerr");
    }
}
