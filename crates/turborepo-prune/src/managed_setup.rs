//! Prune owns copying policy; setup owns the validated provenance schema.
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
};

use miette::Diagnostic;
use turborepo_config::{CONFIG_FILE, CONFIG_FILE_JSONC};
use turborepo_setup::lock::{Lock, MAX_LOCK_BYTES};

use super::{Prune, PruneInput};

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("Cannot prune managed setup: {0}")]
    Lock(#[from] turborepo_setup::lock::Error),
    #[error("Cannot prune managed setup path `{path}`: {source}")]
    Path {
        path: String,
        #[source]
        source: io::Error,
    },
}

fn path_error(path: &Path, source: io::Error) -> Error {
    Error::Path {
        path: path.display().to_string(),
        source,
    }
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Validate every component, not only a canonical path's containment. Setup
/// readers reject links too, including links within the repository. Like
/// prune's other file copies, this assumes directories are not concurrently
/// replaced.
fn validate_path(root: &Path, relative: &str, source: bool) -> Result<PathBuf, Error> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let final_component = index + 1 == parts.len();
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if !linked(&metadata)
                    && (if final_component {
                        metadata.is_file()
                    } else {
                        metadata.is_dir()
                    }) => {}
            Ok(_) => {
                return Err(path_error(
                    &path,
                    io::Error::other("expected an unlinked regular file or parent directory"),
                ));
            }
            Err(error) if !source && error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(path_error(&path, error)),
        }
    }
    Ok(path)
}

/// Validate every output component back to a trusted canonical anchor, even
/// outside the repository. Do not canonicalize a user-selected output parent:
/// doing so would hide its symlinks before validation.
fn validate_output_directory(directory: &Path) -> Result<(), Error> {
    let anchor = directory
        .ancestors()
        .last()
        .filter(|_| directory.is_absolute())
        .ok_or_else(|| {
            path_error(
                directory,
                io::Error::other("expected an absolute output path"),
            )
        })?;
    let (anchor, expected) = (anchor, None::<&Path>);
    // These OS-owned aliases may appear in tempfile paths. Trust only their
    // fixed canonical targets, never arbitrary links below them.
    #[cfg(target_os = "macos")]
    let (anchor, expected) = [
        (Path::new("/tmp"), Path::new("/private/tmp")),
        (Path::new("/var"), Path::new("/private/var")),
    ]
    .into_iter()
    .find(|(alias, _)| directory.starts_with(alias))
    .map(|(alias, target)| (alias, Some(target)))
    .unwrap_or((anchor, expected));
    let canonical = anchor
        .canonicalize()
        .map_err(|error| path_error(anchor, error))?;
    if expected.is_some_and(|target| canonical != target) {
        return Err(path_error(
            anchor,
            io::Error::other("unexpected platform alias target"),
        ));
    }
    let relative = directory
        .strip_prefix(anchor)
        .map_err(|error| path_error(directory, io::Error::other(error)))?;
    let directory = canonical.join(relative);
    for path in directory.ancestors() {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !linked(&metadata) && metadata.is_dir() => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => {
                return Err(path_error(
                    path,
                    io::Error::other("expected an unlinked output directory"),
                ));
            }
        }
        if path == canonical {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct ManagedSetup {
    lock_bytes: Option<Vec<u8>>,
    sources: Vec<(String, PathBuf)>,
}

impl ManagedSetup {
    pub(super) fn enabled(&self) -> bool {
        self.lock_bytes.is_some()
    }

    /// Complete source and destination preflight before Prune::new creates any
    /// output. Disabled setup and an absent lock preserve legacy behavior.
    pub(super) fn plan(input: &PruneInput) -> Result<Self, Error> {
        if !input.future_flags.experimental_setup {
            return Ok(Self::default());
        }
        let root = input.repo_root.as_std_path();
        let lock_path = root.join("turbo.lock");
        match fs::symlink_metadata(&lock_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(path_error(&lock_path, error)),
            Ok(_) => {}
        }
        let lock_path = validate_path(root, "turbo.lock", true)?;
        let mut bytes = Vec::new();
        File::open(&lock_path)
            .and_then(|file| file.take(MAX_LOCK_BYTES as u64 + 1).read_to_end(&mut bytes))
            .map_err(|error| path_error(&lock_path, error))?;
        let lock = Lock::parse(&bytes)?;
        let mut files: BTreeSet<_> = std::iter::once("turbo.lock")
            .chain(lock.tools().values().flat_map(|tool| {
                tool.declarations
                    .iter()
                    .map(|declaration| declaration.file.as_str())
            }))
            .collect();
        // Docker's install layer also needs the root opt-in. It receives the
        // pruned config later, not an unpruned copy with stale task references.
        for config in [CONFIG_FILE, CONFIG_FILE_JSONC] {
            match fs::symlink_metadata(root.join(config)) {
                Ok(_) => {
                    files.insert(config);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(path_error(&root.join(config), error)),
            }
        }
        let output =
            turbopath::AbsoluteSystemPathBuf::from_unknown(&input.repo_root, &input.output_dir);
        let destinations = if input.docker {
            vec![output.join_component("full"), output.join_component("json")]
        } else {
            vec![output.clone()]
        };
        for directory in std::iter::once(&output).chain(destinations.iter()) {
            validate_output_directory(directory.as_std_path())?;
        }
        let mut sources = Vec::new();
        for file in files {
            let path = validate_path(root, file, true)?;
            for destination in &destinations {
                validate_path(destination.as_std_path(), file, false)?;
            }
            if file != "turbo.lock" {
                sources.push((file.to_owned(), path));
            }
        }
        Ok(Self {
            lock_bytes: Some(bytes),
            sources,
        })
    }

    /// Copy before ecosystem rendering so root manifests/configuration can
    /// still be pruned. Copying package.json afterwards would restore removed
    /// workspaces, dependencies and patches; copying it here also preserves it
    /// when there is no JS renderer. Never rewrite lock resolution or
    /// integrity.
    pub(super) fn copy(&self, prune: &Prune<'_>) -> Result<(), super::Error> {
        let Some(bytes) = &self.lock_bytes else {
            return Ok(());
        };
        let mut destinations = vec![prune.full_directory.clone()];
        if prune.docker {
            destinations.push(prune.docker_directory());
        }
        for destination in destinations {
            destination
                .join_component("turbo.lock")
                .create_with_contents(bytes)?;
            for (file, source) in &self.sources {
                let target = destination.as_std_path().join(file);
                turborepo_fs::copy_file(
                    turbopath::AbsoluteSystemPathBuf::try_from(source.as_path())?,
                    turbopath::AbsoluteSystemPathBuf::try_from(target.as_path())?,
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
