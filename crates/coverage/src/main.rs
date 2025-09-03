use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "coverage")]
#[command(about = "Generate Rust code coverage reports")]
struct Args {
    /// Open the HTML report in browser when finished
    #[arg(long)]
    open: bool,
}

/// Find the llvm-profdata binary using rustup
fn find_llvm_profdata() -> Result<std::path::PathBuf> {
    // First try to find it in PATH
    if let Ok(output) = Command::new("which").arg("llvm-profdata").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(std::path::PathBuf::from(path));
        }
    }

    // Try to find it using rustup
    let output = Command::new("rustup")
        .args(["which", "llvm-profdata"])
        .output()
        .context("Failed to run rustup which llvm-profdata")?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(std::path::PathBuf::from(path));
        }
    }

    // Try to find it in the rustup toolchain directory
    let rustup_home = std::env::var("RUSTUP_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    if cfg!(target_os = "windows") {
                        std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
                    } else {
                        std::path::PathBuf::from("/home")
                    }
                });
            home.join(".rustup")
        });

    // Get the active toolchain
    let toolchain_output = Command::new("rustup")
        .args(["show", "active-toolchain"])
        .output()
        .context("Failed to get active toolchain")?;

    let toolchain = if toolchain_output.status.success() {
        String::from_utf8_lossy(&toolchain_output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("stable")
            .to_string()
    } else {
        "stable".to_string()
    };

    // Get the target triple
    let target_output = Command::new("rustc")
        .args(["-vV"])
        .output()
        .context("Failed to get target triple")?;

    let target = if target_output.status.success() {
        let output_str = String::from_utf8_lossy(&target_output.stdout);
        output_str
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .unwrap_or("unknown")
            .to_string()
    } else {
        "unknown".to_string()
    };

    let llvm_profdata_path = rustup_home
        .join("toolchains")
        .join(toolchain)
        .join("lib")
        .join("rustlib")
        .join(target)
        .join("bin")
        .join("llvm-profdata");

    if llvm_profdata_path.exists() {
        return Ok(llvm_profdata_path);
    }

    anyhow::bail!(
        "llvm-profdata not found. Install it with: rustup component add llvm-tools-preview"
    );
}

/// Find the llvm-cov binary using the same approach as llvm-profdata
fn find_llvm_cov() -> Result<std::path::PathBuf> {
    // First try to find it in PATH
    if let Ok(output) = Command::new("which").arg("llvm-cov").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(std::path::PathBuf::from(path));
        }
    }

    // Try to find it using rustup
    let output = Command::new("rustup")
        .args(["which", "llvm-cov"])
        .output()
        .context("Failed to run rustup which llvm-cov")?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(std::path::PathBuf::from(path));
        }
    }

    // Use the same path as llvm-profdata but with llvm-cov
    let llvm_profdata_path = find_llvm_profdata()?;
    let llvm_cov_path = llvm_profdata_path.parent().unwrap().join("llvm-cov");

    if llvm_cov_path.exists() {
        return Ok(llvm_cov_path);
    }

    anyhow::bail!("llvm-cov not found. Install it with: rustup component add llvm-tools-preview");
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let project_root = std::env::current_dir()?;
    let coverage_dir = project_root.join("coverage");

    // Create coverage directory
    std::fs::create_dir_all(&coverage_dir)?;

    // Find the LLVM tools
    let llvm_profdata_path = find_llvm_profdata()?;
    let llvm_cov_path = find_llvm_cov()?;

    info!("Using llvm-profdata: {}", llvm_profdata_path.display());
    info!("Using llvm-cov: {}", llvm_cov_path.display());

    info!("Running tests with coverage instrumentation...");

    // Run tests with coverage instrumentation
    let test_status = Command::new("cargo")
        .args(["test", "--tests", "--workspace"])
        .env("RUSTFLAGS", "-C instrument-coverage")
        .env(
            "LLVM_PROFILE_FILE",
            coverage_dir.join("turbo-%m-%p.profraw"),
        )
        .current_dir(&project_root)
        .status()
        .context("Failed to run cargo test")?;

    if !test_status.success() {
        anyhow::bail!("Tests failed");
    }

    info!("Merging coverage data...");

    // Merge coverage data
    let profdata_path = coverage_dir.join("turbo.profdata");
    let profraw_files = glob::glob(&coverage_dir.join("*.profraw").to_string_lossy())
        .context("Failed to glob profraw files")?;

    let mut profraw_paths = Vec::new();
    for entry in profraw_files {
        profraw_paths.push(entry?);
    }

    if profraw_paths.is_empty() {
        warn!("No profraw files found");
        return Ok(());
    }

    let mut merge_cmd = Command::new(&llvm_profdata_path);
    merge_cmd
        .args(["merge", "-sparse"])
        .args(&profraw_paths)
        .args(["-o", profdata_path.to_string_lossy().as_ref()]);

    let merge_status = merge_cmd
        .current_dir(&project_root)
        .status()
        .context("Failed to merge coverage data")?;

    if !merge_status.success() {
        anyhow::bail!("Failed to merge coverage data");
    }

    // Get test binaries
    info!("Finding test binaries...");
    let binaries_output = Command::new("cargo")
        .args([
            "test",
            "--tests",
            "--no-run",
            "--message-format=json",
            "--workspace",
        ])
        .env("RUSTFLAGS", "-C instrument-coverage")
        .current_dir(&project_root)
        .output()
        .context("Failed to get test binaries")?;

    let binaries_json =
        String::from_utf8(binaries_output.stdout).context("Failed to parse cargo output")?;

    let mut object_args = Vec::new();
    for line in binaries_json.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(profile) = json.get("profile")
            && let Some(test) = profile.get("test")
            && test.as_bool() == Some(true)
            && let Some(filenames) = json.get("filenames")
            && let Some(filenames_array) = filenames.as_array()
        {
            for filename in filenames_array {
                if let Some(path) = filename.as_str()
                    && !path.contains("dSYM")
                {
                    object_args.push(format!("--object={path}"));
                }
            }
        }
    }

    // Generate summary report
    info!("Generating coverage summary...");

    let mut report_cmd = Command::new(&llvm_cov_path);
    report_cmd
        .args(["report"])
        .arg(format!("--instr-profile={}", profdata_path.display()))
        .args([
            "--ignore-filename-regex=/.cargo/registry",
            "--ignore-filename-regex=/.cargo/git",
            "--ignore-filename-regex=/.rustup/toolchains",
            "--ignore-filename-regex=/target/",
        ])
        .args(&object_args);

    let report_output = report_cmd
        .current_dir(&project_root)
        .output()
        .context("Failed to generate coverage report")?;

    if !report_output.status.success() {
        anyhow::bail!("Failed to generate coverage report");
    }

    print!("{}", String::from_utf8_lossy(&report_output.stdout));

    // Generate HTML report
    info!("Generating HTML coverage report...");

    let html_dir = coverage_dir.join("html");
    std::fs::create_dir_all(&html_dir)?;

    let mut show_cmd = Command::new(&llvm_cov_path);
    show_cmd
        .args(["show", "--format=html"])
        .arg(format!("--output-dir={}", html_dir.display()))
        .arg(format!("--instr-profile={}", profdata_path.display()))
        .args([
            "--ignore-filename-regex=/.cargo/registry",
            "--ignore-filename-regex=/.cargo/git",
            "--ignore-filename-regex=/.rustup/toolchains",
            "--ignore-filename-regex=/target/",
        ])
        .args(&object_args);

    let show_status = show_cmd
        .current_dir(&project_root)
        .status()
        .context("Failed to generate HTML coverage report")?;

    if !show_status.success() {
        anyhow::bail!("Failed to generate HTML coverage report");
    }

    info!(
        "Coverage report generated at {}/html/index.html",
        coverage_dir.display()
    );

    // Open HTML report if requested
    if args.open {
        let html_path = coverage_dir.join("html").join("index.html");
        if html_path.exists() {
            info!("Opening HTML report in browser...");

            // Use platform-appropriate command to open the browser
            let open_command = if cfg!(target_os = "windows") {
                "start"
            } else if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };

            let open_status = Command::new(open_command)
                .arg(html_path.to_string_lossy().as_ref())
                .status()
                .context("Failed to open HTML report")?;

            if !open_status.success() {
                warn!("Failed to open HTML report in browser");
            }
        } else {
            warn!(
                "HTML report not found at expected location: {}",
                html_path.display()
            );
        }
    }

    Ok(())
}
