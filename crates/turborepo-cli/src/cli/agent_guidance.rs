use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

const BEGIN_MARKER: &str = "<!-- BEGIN:turborepo-agent-rules -->";
const END_MARKER: &str = "<!-- END:turborepo-agent-rules -->";
const MANAGED_BLOCK: &str = r#"<!-- BEGIN:turborepo-agent-rules -->

# This is NOT the Turborepo you know

Turborepo configuration, task behavior, and CLI commands can vary between installed versions and may differ from your training data. Resolve the `turbo` package from this file's directory or relevant workspace; in monorepos, it may not be visible from the repository root. For example, run `node -p "require.resolve('turbo/package.json')"` from a workspace that depends on `turbo`.

Read `docs/README.md` inside that installed package first, then read the relevant pages from its `docs/` directory before changing Turborepo configuration or commands. Heed deprecation notices. These bundled docs match the installed package version and are available without network access.

This block is written and re-added by `turbo` before repository-scoped commands when an AI agent is detected. In the Turborepo source repository, its template is defined in `crates/turborepo-cli/src/cli/agent_guidance.rs`. Removing the managed block while updates are enabled means a later qualifying invocation will add it again. Set `"agentGuidance": false` in the root `turbo.json` or `turbo.jsonc` to opt out; this does not remove an existing block. Keep the block committed with your work to avoid an uncommitted change on the next agent invocation.
<!-- END:turborepo-agent-rules -->"#;

static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MaintenanceStatus {
    Skipped,
    MissingDocs,
    Updated,
    Unchanged,
    MalformedMarkers,
    ConcurrentEdit,
    Locked,
}

/// Update the managed guidance only when an agent is detected and the root
/// configuration has not opted out. The package docs are located relative to
/// the selected CLI binary, which works for both nested installs and pnpm's
/// virtual store layout.
pub(super) fn maintain(
    repo_root: &Path,
    executable: &Path,
    agent_detected: bool,
    enabled: bool,
) -> io::Result<MaintenanceStatus> {
    if !agent_detected || !enabled {
        return Ok(MaintenanceStatus::Skipped);
    }

    if find_bundled_docs_index(executable).is_none() {
        return Ok(MaintenanceStatus::MissingDocs);
    }

    upsert(repo_root)
}

fn find_bundled_docs_index(executable: &Path) -> Option<PathBuf> {
    for ancestor in executable.ancestors() {
        let candidates = [
            ancestor.join("docs/README.md"),
            ancestor.join("turbo/docs/README.md"),
        ];
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(path);
        }
    }
    None
}

fn acquire_lock(lock: &mut pidlock::Pidlock) -> Result<(), pidlock::PidlockError> {
    let mut attempts = 0;
    loop {
        match lock.acquire() {
            Err(pidlock::PidlockError::File(pidlock::PidFileError::Invalid { .. }))
                if attempts < 10 =>
            {
                // Pidlock creates the lock file before writing its PID. A
                // competing invocation can observe that short initialization
                // window and see an empty or partial file.
                attempts += 1;
                thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
}

fn upsert(repo_root: &Path) -> io::Result<MaintenanceStatus> {
    let lock_path = repo_root.join(".turborepo-agent-guidance.lock");
    let mut lock = pidlock::Pidlock::new(lock_path);
    match acquire_lock(&mut lock) {
        Ok(()) => {}
        Err(pidlock::PidlockError::AlreadyOwned | pidlock::PidlockError::LockExists(_)) => {
            return Ok(MaintenanceStatus::Locked);
        }
        Err(error) => {
            return Err(io::Error::other(format!(
                "failed to acquire the agent-guidance lock: {error}"
            )));
        }
    }

    let agents_path = repo_root.join("AGENTS.md");
    let original = match fs::read_to_string(&agents_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let updated = match upsert_managed_block(original.as_deref()) {
        Ok(Some(contents)) => contents,
        Ok(None) => return Ok(MaintenanceStatus::Unchanged),
        Err(()) => return Ok(MaintenanceStatus::MalformedMarkers),
    };

    let Some(parent) = agents_path.parent() else {
        return Err(io::Error::other("AGENTS.md has no parent directory"));
    };
    let temp_path = write_temp_file(parent, &updated, original.as_deref())?;

    // Do not replace the file if another editor changed it after we read it.
    // All turbo invocations additionally share the pid lock above.
    match replace_if_unchanged(&agents_path, &temp_path, original.as_deref()) {
        Ok(true) => Ok(MaintenanceStatus::Updated),
        Ok(false) => {
            let _ = fs::remove_file(&temp_path);
            Ok(MaintenanceStatus::ConcurrentEdit)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

fn replace_if_unchanged(
    agents_path: &Path,
    temp_path: &Path,
    original: Option<&str>,
) -> io::Result<bool> {
    let current = match fs::read_to_string(agents_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if current.as_deref() != original {
        return Ok(false);
    }
    fs::rename(temp_path, agents_path)?;
    Ok(true)
}

fn upsert_managed_block(existing: Option<&str>) -> Result<Option<String>, ()> {
    let Some(existing) = existing else {
        return Ok(Some(format!("{MANAGED_BLOCK}\n")));
    };

    let starts = existing.match_indices(BEGIN_MARKER).collect::<Vec<_>>();
    let ends = existing.match_indices(END_MARKER).collect::<Vec<_>>();
    let managed_range = match (starts.as_slice(), ends.as_slice()) {
        ([], []) => None,
        ([(start, begin)], [(end, finish)]) if start < end => Some(*start..(*end + finish.len())),
        _ => return Err(()),
    };

    if let Some(range) = managed_range {
        if &existing[range.clone()] == MANAGED_BLOCK {
            return Ok(None);
        }
        let mut updated = existing.to_owned();
        updated.replace_range(range, MANAGED_BLOCK);
        return Ok(Some(updated));
    }

    let separator = if existing.is_empty() || existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(Some(format!("{existing}{separator}{MANAGED_BLOCK}\n")))
}

fn write_temp_file(parent: &Path, contents: &str, original: Option<&str>) -> io::Result<PathBuf> {
    let permissions = original
        .and_then(|_| fs::metadata(parent.join("AGENTS.md")).ok())
        .map(|metadata| metadata.permissions());

    loop {
        let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".AGENTS.md.{}.{}.tmp", std::process::id(), id));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            if let Some(permissions) = &permissions {
                file.set_permissions(permissions.clone())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
        return Ok(temp_path);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, thread};

    use tempfile::TempDir;

    use super::{
        find_bundled_docs_index, maintain, replace_if_unchanged, upsert, MaintenanceStatus,
        BEGIN_MARKER, END_MARKER, MANAGED_BLOCK,
    };

    fn install_docs(root: &Path) -> std::path::PathBuf {
        let executable = root.join("node_modules/@turbo/darwin-arm64/bin/turbo");
        let docs = root.join("node_modules/turbo/docs");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("README.md"), "# Docs\n").unwrap();
        executable
    }

    #[test]
    fn resolves_docs_from_nested_package_install() {
        let temp = TempDir::new().unwrap();
        let executable = install_docs(temp.path());

        assert_eq!(
            find_bundled_docs_index(&executable),
            Some(temp.path().join("node_modules/turbo/docs/README.md"))
        );
    }

    #[test]
    fn does_not_touch_files_without_agent_or_when_opted_out() {
        let temp = TempDir::new().unwrap();
        let executable = install_docs(temp.path());
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();

        assert_eq!(
            maintain(&root, &executable, false, true).unwrap(),
            MaintenanceStatus::Skipped
        );
        assert_eq!(
            maintain(&root, &executable, true, false).unwrap(),
            MaintenanceStatus::Skipped
        );
        assert!(!root.join("AGENTS.md").exists());
    }

    #[test]
    fn skips_when_bundled_docs_are_missing() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();

        assert_eq!(
            maintain(&root, Path::new("/no/installed/turbo"), true, true).unwrap(),
            MaintenanceStatus::MissingDocs
        );
        assert!(!root.join("AGENTS.md").exists());
    }

    #[test]
    fn creates_file_and_preserves_user_content_and_other_blocks() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let agents = root.join("AGENTS.md");
        let user_content = "# Project rules\n\n<!-- BEGIN:nextjs-agent-rules -->\nNext.js\n<!-- \
                            END:nextjs-agent-rules -->\n";
        fs::write(&agents, user_content).unwrap();

        assert_eq!(upsert(root).unwrap(), MaintenanceStatus::Updated);
        let updated = fs::read_to_string(&agents).unwrap();
        assert!(updated.starts_with(user_content));
        assert!(updated.contains(MANAGED_BLOCK));

        let empty = TempDir::new().unwrap();
        assert_eq!(upsert(empty.path()).unwrap(), MaintenanceStatus::Updated);
        assert_eq!(
            fs::read_to_string(empty.path().join("AGENTS.md")).unwrap(),
            format!("{MANAGED_BLOCK}\n")
        );
    }

    #[test]
    fn updates_only_its_managed_block() {
        let temp = TempDir::new().unwrap();
        let agents = temp.path().join("AGENTS.md");
        let previous = "# Keep this\n\n<!-- BEGIN:turborepo-agent-rules -->\nold \
                        instructions\n<!-- END:turborepo-agent-rules -->\n\n<!-- BEGIN:other-tool \
                        -->\nkeep me\n<!-- END:other-tool -->\n";
        fs::write(&agents, previous).unwrap();

        assert_eq!(upsert(temp.path()).unwrap(), MaintenanceStatus::Updated);
        let updated = fs::read_to_string(agents).unwrap();
        assert!(updated.starts_with("# Keep this\n\n"));
        assert!(updated.contains(MANAGED_BLOCK));
        assert!(
            updated.ends_with("\n\n<!-- BEGIN:other-tool -->\nkeep me\n<!-- END:other-tool -->\n")
        );
    }

    #[test]
    fn leaves_matching_file_and_mtime_unchanged() {
        let temp = TempDir::new().unwrap();
        let agents = temp.path().join("AGENTS.md");
        fs::write(&agents, format!("{MANAGED_BLOCK}\n")).unwrap();
        let before = fs::metadata(&agents).unwrap().modified().unwrap();

        assert_eq!(upsert(temp.path()).unwrap(), MaintenanceStatus::Unchanged);
        assert_eq!(fs::metadata(&agents).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn malformed_or_duplicated_markers_are_left_untouched() {
        for contents in [
            format!("{BEGIN_MARKER}\nmissing end"),
            format!("{END_MARKER}\nmissing begin"),
            format!("{MANAGED_BLOCK}\n{MANAGED_BLOCK}"),
            format!("{END_MARKER}\n{BEGIN_MARKER}"),
        ] {
            let temp = TempDir::new().unwrap();
            let agents = temp.path().join("AGENTS.md");
            fs::write(&agents, &contents).unwrap();

            assert_eq!(
                upsert(temp.path()).unwrap(),
                MaintenanceStatus::MalformedMarkers
            );
            assert_eq!(fs::read_to_string(agents).unwrap(), contents);
        }
    }

    #[test]
    fn concurrent_invocations_preserve_a_single_complete_block() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_owned();
        let handles = (0..8)
            .map(|_| {
                let root = root.clone();
                thread::spawn(move || upsert(&root).unwrap())
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(matches!(
                handle.join().unwrap(),
                MaintenanceStatus::Updated
                    | MaintenanceStatus::Unchanged
                    | MaintenanceStatus::Locked
            ));
        }

        let contents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(contents.matches(BEGIN_MARKER).count(), 1);
        assert_eq!(contents.matches(END_MARKER).count(), 1);
        assert!(contents.contains(MANAGED_BLOCK));
    }

    #[test]
    fn does_not_replace_file_changed_during_write() {
        let temp = TempDir::new().unwrap();
        let agents = temp.path().join("AGENTS.md");
        let temp_file = temp.path().join(".AGENTS.md.tmp");
        fs::write(&agents, "initial content").unwrap();
        fs::write(&temp_file, "generated content").unwrap();
        fs::write(&agents, "user edit").unwrap();

        assert!(!replace_if_unchanged(&agents, &temp_file, Some("initial content")).unwrap());
        assert_eq!(fs::read_to_string(agents).unwrap(), "user edit");
    }

    #[test]
    fn write_failures_leave_non_file_targets_untouched() {
        let temp = TempDir::new().unwrap();
        let agents = temp.path().join("AGENTS.md");
        fs::create_dir(&agents).unwrap();

        assert!(upsert(temp.path()).is_err());
        assert!(agents.is_dir());
    }

    #[test]
    fn managed_template_matches_versioned_docs_guidance_without_network_links() {
        assert!(MANAGED_BLOCK.contains("# This is NOT the Turborepo you know"));
        assert!(MANAGED_BLOCK.contains("docs/README.md"));
        assert!(MANAGED_BLOCK.contains("agent_guidance.rs"));
        assert!(MANAGED_BLOCK.contains("agentGuidance\": false"));
        assert!(!MANAGED_BLOCK.contains("https://"));
    }
}
