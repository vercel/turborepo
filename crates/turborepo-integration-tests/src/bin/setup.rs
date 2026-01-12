//! CLI binary for setting up turborepo integration test environments.
//!
//! This binary is intended to be called from shell scripts (like prysk test
//! helpers) to set up test environments using the same Rust infrastructure
//! as native Rust tests.
//!
//! # Usage
//!
//! ```sh
//! # Initialize a test environment (outputs shell commands to eval)
//! eval "$(turbo-test-setup init basic_monorepo)"
//!
//! # With package manager
//! eval "$(turbo-test-setup init basic_monorepo --package-manager npm@10.5.0)"
//!
//! # Skip dependency installation
//! eval "$(turbo-test-setup init basic_monorepo --no-install)"
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use turborepo_integration_tests::{copy_dir_recursive, fixtures_path, turbo_binary_path};
use which::which;

#[derive(Parser)]
#[command(name = "turbo-test-setup")]
#[command(about = "Set up turborepo integration test environments")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a test environment by copying a fixture and setting up git.
    /// Outputs shell commands to set environment variables.
    Init {
        /// Name of the fixture to copy (from
        /// turborepo-tests/integration/fixtures/)
        fixture: String,

        /// Package manager to use (e.g., "npm@10.5.0")
        #[arg(long, default_value = "npm@10.5.0")]
        package_manager: String,

        /// Skip installing dependencies
        #[arg(long)]
        no_install: bool,

        /// Target directory (defaults to current directory)
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            fixture,
            package_manager,
            no_install,
            target_dir,
        } => {
            init_test_env(&fixture, &package_manager, no_install, target_dir).await?;
        }
    }

    Ok(())
}

async fn init_test_env(
    fixture: &str,
    package_manager: &str,
    no_install: bool,
    target_dir: Option<PathBuf>,
) -> Result<()> {
    let target_dir = target_dir.unwrap_or_else(|| std::env::current_dir().unwrap());

    // Copy fixture
    let fixtures_base = fixtures_path();
    let fixture_path = fixtures_base.join(fixture);

    if !fixture_path.exists() {
        anyhow::bail!("Fixture not found: {:?}", fixture_path);
    }

    let canonical_fixture = fixture_path
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize fixture path: {:?}", fixture_path))?;

    copy_dir_recursive(&canonical_fixture, &target_dir)?;

    // Set up git
    setup_git(&target_dir).await?;

    // Set up package manager
    let corepack_dir = setup_package_manager(&target_dir, package_manager).await?;

    // Install dependencies
    if !no_install {
        install_deps(&target_dir, package_manager, &corepack_dir).await?;
    }

    // Output shell commands to set environment variables
    let turbo_binary = turbo_binary_path();
    // Use : as PATH separator on Unix, ; on Windows
    #[cfg(windows)]
    let path_separator = ";";
    #[cfg(not(windows))]
    let path_separator = ":";
    let corepack_path_entry = format!(
        "{}{}{}",
        corepack_dir.display(),
        path_separator,
        std::env::var("PATH").unwrap_or_default()
    );

    // Output shell variable assignments that can be eval'd
    println!("export TURBO={}", turbo_binary.display());
    println!("export TURBO_TELEMETRY_MESSAGE_DISABLED=1");
    println!("export TURBO_GLOBAL_WARNING_DISABLED=1");
    println!("export TURBO_PRINT_VERSION_DISABLED=1");
    println!("export PATH=\"{}\"", corepack_path_entry);

    Ok(())
}

async fn setup_git(target_dir: &PathBuf) -> Result<()> {
    // git init
    let output = tokio::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch=main"])
        .current_dir(target_dir)
        .output()
        .await
        .context("Failed to run git init")?;

    if !output.status.success() {
        anyhow::bail!(
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // git config
    let configs = [
        ("user.email", "turbo-test@example.com"),
        ("user.name", "Turbo Test"),
    ];

    for (key, value) in configs {
        let output = tokio::process::Command::new("git")
            .args(["config", key, value])
            .current_dir(target_dir)
            .output()
            .await
            .context("Failed to run git config")?;

        if !output.status.success() {
            anyhow::bail!(
                "git config {} failed: {}",
                key,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    // Create .npmrc for cross-platform script consistency
    tokio::fs::write(target_dir.join(".npmrc"), "script-shell=bash\n")
        .await
        .context("Failed to write .npmrc")?;

    // git add and commit
    let output = tokio::process::Command::new("git")
        .args(["add", "."])
        .current_dir(target_dir)
        .output()
        .await
        .context("Failed to run git add")?;

    if !output.status.success() {
        anyhow::bail!(
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = tokio::process::Command::new("git")
        .args(["commit", "-m", "Initial", "--quiet"])
        .current_dir(target_dir)
        .output()
        .await
        .context("Failed to run git commit")?;

    if !output.status.success() {
        anyhow::bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

async fn setup_package_manager(target_dir: &PathBuf, package_manager: &str) -> Result<PathBuf> {
    // Update package.json with packageManager field
    let package_json_path = target_dir.join("package.json");
    let content = tokio::fs::read_to_string(&package_json_path)
        .await
        .context("Failed to read package.json")?;
    let mut json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse package.json")?;

    json["packageManager"] = serde_json::Value::String(package_manager.to_string());

    let mut updated = serde_json::to_string_pretty(&json)?;
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    tokio::fs::write(&package_json_path, updated)
        .await
        .context("Failed to write package.json")?;

    // Commit the change
    let output = tokio::process::Command::new("git")
        .args(["add", "package.json"])
        .current_dir(target_dir)
        .output()
        .await?;

    if output.status.success() {
        let commit_msg = format!("Updated package manager to {}", package_manager);
        let _ = tokio::process::Command::new("git")
            .args(["commit", "-m", &commit_msg, "--quiet"])
            .current_dir(target_dir)
            .output()
            .await;
    }

    // Set up corepack
    let corepack_dir = if let Ok(prysk_temp) = std::env::var("PRYSK_TEMP") {
        PathBuf::from(prysk_temp).join("corepack")
    } else {
        target_dir.join("corepack")
    };

    tokio::fs::create_dir_all(&corepack_dir)
        .await
        .context("Failed to create corepack directory")?;

    // Extract package manager name (e.g., "npm" from "npm@10.5.0")
    let package_manager_name = package_manager.split('@').next().unwrap_or(package_manager);

    // Run corepack enable
    let corepack_binary = which("corepack").context("corepack not found in PATH")?;
    let install_dir_arg = format!("--install-directory={}", corepack_dir.display());

    let output = tokio::process::Command::new(&corepack_binary)
        .args(["enable", package_manager_name, &install_dir_arg])
        .current_dir(target_dir)
        .output()
        .await
        .context("Failed to run corepack enable")?;

    if !output.status.success() {
        anyhow::bail!(
            "corepack enable failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(corepack_dir)
}

async fn install_deps(
    target_dir: &PathBuf,
    package_manager: &str,
    corepack_dir: &PathBuf,
) -> Result<()> {
    let pm_name = package_manager.split('@').next().unwrap_or("npm");

    // Build PATH with corepack dir first
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!(
        "{}{}{}",
        corepack_dir.display(),
        std::path::MAIN_SEPARATOR,
        current_path
    );

    let output = tokio::process::Command::new(pm_name)
        .args(["install"])
        .current_dir(target_dir)
        .env("PATH", &new_path)
        .output()
        .await
        .with_context(|| format!("Failed to run {} install", pm_name))?;

    if !output.status.success() {
        // Print stderr to stderr, but don't fail - some fixtures may not need deps
        eprintln!(
            "Warning: {} install failed: {}",
            pm_name,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}
