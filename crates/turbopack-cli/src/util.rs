use std::{env::current_dir, path::PathBuf};

use anyhow::{Context, Result};
use dunce::canonicalize;
use turbo_tasks_fs::{DiskFileSystemVc, FileSystemVc};

#[turbo_tasks::value(transparent)]
pub struct EntryRequests(Vec<EntryRequestVc>);

#[turbo_tasks::value(shared)]
#[derive(Clone)]
pub enum EntryRequest {
    Relative(String),
    Module(String, String),
}

pub struct NormalizedDirs {
    pub project_dir: String,
    pub root_dir: String,
}

pub fn normalize_dirs(
    project_dir: &Option<PathBuf>,
    root_dir: &Option<PathBuf>,
) -> Result<NormalizedDirs> {
    let project_dir = project_dir
        .as_ref()
        .map(canonicalize)
        .unwrap_or_else(current_dir)
        .context("project directory can't be found")?
        .to_str()
        .context("project directory contains invalid characters")?
        .to_string();

    let root_dir = match root_dir.as_ref() {
        Some(root) => canonicalize(root)
            .context("root directory can't be found")?
            .to_str()
            .context("root directory contains invalid characters")?
            .to_string(),
        None => project_dir.clone(),
    };

    Ok(NormalizedDirs {
        project_dir,
        root_dir,
    })
}

pub fn normalize_entries(entries: &Option<Vec<String>>) -> Vec<String> {
    entries
        .as_ref()
        .cloned()
        .unwrap_or_else(|| vec!["src/entry".to_owned()])
}

#[turbo_tasks::function]
pub async fn project_fs(project_dir: &str) -> Result<FileSystemVc> {
    let disk_fs = DiskFileSystemVc::new("project".to_string(), project_dir.to_string());
    disk_fs.await?.start_watching()?;
    Ok(disk_fs.into())
}

#[turbo_tasks::function]
pub async fn output_fs(project_dir: &str) -> Result<FileSystemVc> {
    let disk_fs = DiskFileSystemVc::new("output".to_string(), project_dir.to_string());
    disk_fs.await?.start_watching()?;
    Ok(disk_fs.into())
}
