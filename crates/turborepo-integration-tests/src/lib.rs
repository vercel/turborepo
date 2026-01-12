//! Integration test setup library for turborepo.
//!
//! This library provides utilities for setting up isolated test environments
//! for turborepo integration tests. It can be used either as a Rust library
//! for Rust-based tests or via the `turbo-test-setup` binary for shell-based
//! tests (like prysk).
//!
//! # Usage
//!
//! ## From Rust tests
//!
//! ```ignore
//! use turborepo_integration_tests::{TurboTestEnv, redact_output};
//!
//! let env = TurboTestEnv::new().await?;
//! env.copy_fixture("basic_monorepo").await?;
//! env.setup_git().await?;
//! ```
//!
//! ## From shell scripts
//!
//! ```sh
//! # Set up a test environment
//! eval "$(turbo-test-setup init basic_monorepo --package-manager npm@10.5.0)"
//! ```

use std::{
    path::{Path, PathBuf},
    process::Output,
    sync::LazyLock,
};

use anyhow::{Context, Result};
use regex::Regex;
use which::which;

// =============================================================================
// Output Redaction
// =============================================================================

/// Compiled regex for timing redaction.
static TIMING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Time:\s*[\d\.]+m?s").expect("Invalid timing regex"));

/// Compiled regex for hash redaction.
static HASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-f0-9]{16}\b").expect("Invalid hash regex"));

/// Compiled regex for temp directory path redaction.
static TEMP_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:(?:/private)?/var/folders/[a-zA-Z0-9_]+/[a-zA-Z0-9_]+/T/\.tmp[a-zA-Z0-9_]+|/tmp/\.tmp[a-zA-Z0-9_]+|[A-Z]:/Users/[^/]+/AppData/Local/Temp/\.?tmp[a-zA-Z0-9_]+)(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid temp path regex")
});

/// Compiled regex for matching temp paths split across lines.
static TEMP_PATH_MULTILINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:/private)?/var/(?:folders/[a-zA-Z0-9_]+/)?\n\s*(?:folders/[a-zA-Z0-9_]+/)?[a-zA-Z0-9_]+/T/\.tmp[a-zA-Z0-9_]+(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid multiline temp path regex")
});

/// Compiled regex for matching Windows temp paths split across lines.
static TEMP_PATH_WINDOWS_MULTILINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Z]:\n\s*[/\\]Users/[^/\n]+/AppData/Local/Temp/\.?tmp[a-zA-Z0-9_]+(?:/[a-zA-Z0-9._-]+)*")
        .expect("Invalid Windows multiline temp path regex")
});

/// Compiled regex for stripping ANSI escape codes from output.
static ANSI_ESCAPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*(?:\x07|\x1b\\)")
        .expect("Invalid ANSI escape regex")
});

/// Apply redactions to make output deterministic for snapshots.
pub fn redact_output(output: &str) -> String {
    let output = ANSI_ESCAPE_RE.replace_all(output, "");
    let output = output.replace("\r\n", "\n");
    let output = normalize_path_separators(&output);
    let output = TIMING_RE.replace_all(&output, "Time: [TIME]");
    let output = HASH_RE.replace_all(&output, "[HASH]");
    let output = TEMP_PATH_MULTILINE_RE.replace_all(&output, "[TEMP_DIR]");
    let output = TEMP_PATH_WINDOWS_MULTILINE_RE.replace_all(&output, "[TEMP_DIR]");
    TEMP_PATH_RE.replace_all(&output, "[TEMP_DIR]").into_owned()
}

/// Normalize Windows path separators to Unix style.
fn normalize_path_separators(output: &str) -> String {
    static PATH_SEP_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\w|:)\\([\w.])").expect("Invalid path separator regex"));

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

// =============================================================================
// Path Utilities
// =============================================================================

/// Path to the turbo binary, discovered via cargo workspace layout.
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

// =============================================================================
// Test Environment
// =============================================================================

/// A test environment that runs turbo commands in an isolated temp directory.
pub struct TurboTestEnv {
    workspace_path: PathBuf,
    turbo_binary: PathBuf,
    config_dir_path: PathBuf,
    corepack_install_dir: PathBuf,
    corepack_enabled: bool,
    _temp_dir: tempfile::TempDir,
}

impl TurboTestEnv {
    /// Create a new isolated test environment.
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
        let config_dir_path = temp_dir.path().join(".turbo-config");
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

    /// Create a test environment at a specific path (for CLI use).
    pub fn at_path(path: PathBuf, turbo_binary: PathBuf) -> Self {
        let config_dir_path = path.join(".turbo-config");
        let corepack_install_dir = path.join("corepack");

        Self {
            workspace_path: path,
            turbo_binary,
            config_dir_path,
            corepack_install_dir,
            corepack_enabled: false,
            _temp_dir: tempfile::tempdir().expect("Failed to create temp dir"),
        }
    }

    /// Copy a fixture into the workspace.
    pub async fn copy_fixture(&self, fixture_name: &str) -> Result<()> {
        if fixture_name.contains("..") {
            anyhow::bail!(
                "Invalid fixture name '{}': path traversal sequences (..) are not allowed",
                fixture_name
            );
        }

        if Path::new(fixture_name).is_absolute() {
            anyhow::bail!(
                "Invalid fixture name '{}': absolute paths are not allowed",
                fixture_name
            );
        }

        let fixtures_base = fixtures_path();
        let fixture_path = fixtures_base.join(fixture_name);

        if !fixture_path.exists() {
            anyhow::bail!("Fixture not found: {:?}", fixture_path);
        }

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

        let workspace_path = self.workspace_path.clone();
        tokio::task::spawn_blocking(move || {
            copy_dir_recursive(&canonical_fixture, &workspace_path)
        })
        .await
        .context("File copy task panicked")??;

        Ok(())
    }

    /// Initialize git in the workspace.
    pub async fn setup_git(&self) -> Result<()> {
        self.exec(&["git", "init"]).await?;
        self.exec(&["git", "config", "user.email", "turbo-test@example.com"])
            .await?;
        self.exec(&["git", "config", "user.name", "Turbo Test"])
            .await?;
        self.exec(&["git", "config", "core.autocrlf", "false"])
            .await?;

        self.write_file(".npmrc", "script-shell=bash\n").await?;

        self.exec(&["git", "add", "."]).await?;
        self.exec(&["git", "commit", "-m", "Initial"]).await?;
        Ok(())
    }

    /// Install npm dependencies in the workspace.
    #[allow(dead_code)]
    pub async fn npm_install(&self) -> Result<ExecResult> {
        self.exec(&["npm", "install"]).await
    }

    fn configure_turbo_env(&self, cmd: &mut tokio::process::Command) {
        if self.corepack_enabled {
            self.configure_corepack_path(cmd);
        }

        cmd.env("TURBO_CONFIG_DIR_PATH", &self.config_dir_path)
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env_remove("GITHUB_ACTIONS")
            .env_remove("CI");
    }

    /// Run turbo with the given arguments.
    pub async fn run_turbo(&self, args: &[&str]) -> Result<ExecResult> {
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&self.workspace_path);
        self.configure_turbo_env(&mut cmd);

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Run turbo from a subdirectory within the workspace.
    pub async fn run_turbo_from_dir(&self, subdir: &str, args: &[&str]) -> Result<ExecResult> {
        let dir = self.workspace_path.join(subdir);
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&dir);
        self.configure_turbo_env(&mut cmd);

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Run turbo with specific environment variables.
    pub async fn run_turbo_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExecResult> {
        let mut cmd = tokio::process::Command::new(&self.turbo_binary);
        cmd.args(args).current_dir(&self.workspace_path);
        self.configure_turbo_env(&mut cmd);

        for (key, value) in env {
            cmd.env(key, value);
        }

        let output = cmd.output().await.context("Failed to execute turbo")?;
        Ok(ExecResult::from(output))
    }

    /// Execute a command in the workspace directory.
    pub async fn exec(&self, cmd: &[&str]) -> Result<ExecResult> {
        let (program, args) = cmd.split_first().context("Empty command")?;
        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(&self.workspace_path);

        if self.corepack_enabled {
            self.configure_corepack_path(&mut command);
        }

        let output = command
            .output()
            .await
            .context("Failed to execute command")?;

        Ok(ExecResult::from(output))
    }

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
    pub async fn enable_corepack(&mut self, package_manager_name: &str) -> Result<()> {
        tokio::fs::create_dir_all(&self.corepack_install_dir)
            .await
            .context("Failed to create corepack install directory")?;

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
    pub async fn set_package_manager(&mut self, package_manager: &str) -> Result<()> {
        let package_json_path = self.workspace_path.join("package.json");
        let content = tokio::fs::read_to_string(&package_json_path).await?;
        let mut json: serde_json::Value =
            serde_json::from_str(&content).context("Failed to parse package.json")?;

        json["packageManager"] = serde_json::Value::String(package_manager.to_string());

        let mut updated = serde_json::to_string_pretty(&json)?;
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        tokio::fs::write(&package_json_path, updated).await?;

        if self.workspace_path.join(".git").exists() {
            self.exec(&["git", "add", "package.json"]).await?;
            let commit_msg = format!("Updated package manager to {}", package_manager);
            self.exec(&["git", "commit", "-m", &commit_msg]).await?;
        }

        let package_manager_name = package_manager.split('@').next().unwrap_or(package_manager);
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

    /// Get the corepack install directory path.
    pub fn corepack_install_dir(&self) -> &Path {
        &self.corepack_install_dir
    }

    /// Check if corepack is enabled.
    pub fn corepack_enabled(&self) -> bool {
        self.corepack_enabled
    }

    /// Get the turbo binary path.
    pub fn turbo_binary(&self) -> &Path {
        &self.turbo_binary
    }
}

// =============================================================================
// ExecResult
// =============================================================================

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

// =============================================================================
// File Utilities
// =============================================================================

/// Recursively copy a directory, normalizing line endings for text files.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src).context("Failed to read source directory")? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
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
            .is_some_and(|name| name.starts_with('.') && !name.contains('.'))
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
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_output_timing() {
        assert_eq!(redact_output("Time: 1.23s"), "Time: [TIME]");
        assert_eq!(redact_output("Time: 100ms"), "Time: [TIME]");
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
}
