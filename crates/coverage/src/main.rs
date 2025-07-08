use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "coverage")]
#[command(about = "Generate Rust code coverage reports")]
struct Args {}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let project_root = std::env::current_dir()?;
    let coverage_dir = project_root.join("coverage");

    // Create coverage directory
    std::fs::create_dir_all(&coverage_dir)?;

    // Get rustup home
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/anthonyshew".to_string());
    let rustup_home = std::env::var("RUSTUP_HOME").unwrap_or_else(|_| format!("{}/.rustup", home));
    // Get the active toolchain (e.g., nightly-2025-03-28-aarch64-apple-darwin)
    let toolchain = std::env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| {
        // Try to get from rustup show active-toolchain
        let output = std::process::Command::new("rustup")
            .args(["show", "active-toolchain"])
            .output();
        if let Ok(out) = output {
            let s = String::from_utf8_lossy(&out.stdout);
            s.split_whitespace().next().unwrap_or("").to_string()
        } else {
            "nightly".to_string()
        }
    });
    // Get the target triple (e.g., aarch64-apple-darwin)
    let target = std::env::var("TARGET").unwrap_or_else(|_| {
        // Try to get from rustc -vV
        let output = std::process::Command::new("rustc").arg("-vV").output();
        if let Ok(out) = output {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("host: ") {
                    return rest.to_string();
                }
            }
        }
        // Fallback
        "aarch64-apple-darwin".to_string()
    });
    let llvm_profdata_path = format!(
        "{}/toolchains/{}/lib/rustlib/{}/bin/llvm-profdata",
        rustup_home, toolchain, target
    );
    println!("[coverage debug] HOME={}", home);
    println!("[coverage debug] RUSTUP_HOME={}", rustup_home);
    println!("[coverage debug] RUSTUP_TOOLCHAIN={}", toolchain);
    println!("[coverage debug] TARGET={}", target);
    println!(
        "[coverage debug] Checking for llvm-profdata at: {}",
        llvm_profdata_path
    );
    let llvm_profdata_path = std::path::PathBuf::from(&llvm_profdata_path);
    if llvm_profdata_path.exists() {
        println!(
            "[coverage debug] Found llvm-profdata at: {}",
            llvm_profdata_path.display()
        );
    } else {
        panic!(
            "llvm-profdata not found at: {}\n  HOME={}\n  RUSTUP_HOME={}\n  RUSTUP_TOOLCHAIN={}\n  TARGET={}\nInstall with: rustup component add llvm-tools-preview",
            llvm_profdata_path.display(), home, rustup_home, toolchain, target
        );
    }

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
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(profile) = json.get("profile") {
                if let Some(test) = profile.get("test") {
                    if test.as_bool() == Some(true) {
                        if let Some(filenames) = json.get("filenames") {
                            if let Some(filenames_array) = filenames.as_array() {
                                for filename in filenames_array {
                                    if let Some(path) = filename.as_str() {
                                        if !path.contains("dSYM") {
                                            object_args.push(format!("--object={}", path));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let llvm_cov_path = llvm_profdata_path
        .to_string_lossy()
        .replace("llvm-profdata", "llvm-cov");

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

    Ok(())
}
