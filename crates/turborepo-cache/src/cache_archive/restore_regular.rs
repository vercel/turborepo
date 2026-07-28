use std::{
    fs::{File, OpenOptions},
    io,
    io::Read,
    path::Path,
};

use tar::Entry;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};

use crate::{CacheError, cache_archive::restore_directory::CachedDirTree};

/// Returns `(path, true)` when the file was skipped (matched manifest),
/// or `(path, false)` when it was written to disk.
pub fn restore_regular(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &mut Entry<impl Read>,
    manifest: Option<&super::restore_manifest::RestoreManifest>,
) -> Result<(AnchoredSystemPathBuf, bool), CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;
    let resolved_path = anchor.resolve(&processed_name);

    // Check if the file on disk already matches the manifest entry.
    // If so, skip the write and just advance the tar stream.
    if let Some(manifest) = manifest
        && manifest.file_matches(processed_name.as_str(), &resolved_path)
    {
        io::copy(entry, &mut io::sink())?;
        return Ok((processed_name, true));
    }

    dir_cache.safe_mkdir_file(anchor, &processed_name)?;
    if let Ok(metadata) = resolved_path.symlink_metadata()
        && metadata.is_symlink()
    {
        remove_symlink(&resolved_path)?;
    }

    #[cfg(unix)]
    let mode = entry.header().mode()?;
    #[cfg(not(unix))]
    let mode = 0;

    let mut file = open_for_restore(&resolved_path, sanitized_mode(mode))?;
    io::copy(entry, &mut file)?;

    Ok((processed_name, false))
}

#[cfg(unix)]
fn sanitized_mode(mode: u32) -> u32 {
    mode & 0o777
}

#[cfg(not(unix))]
fn sanitized_mode(mode: u32) -> u32 {
    mode
}

fn open_for_restore(
    path: &AbsoluteSystemPath,
    #[cfg_attr(not(unix), allow(unused_variables))] mode: u32,
) -> Result<File, CacheError> {
    let mut open_options = OpenOptions::new();
    open_options.write(true).truncate(true).create(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        open_options.mode(mode);
        // If a symlink appears after the pre-open check/removal, refuse to
        // follow it instead of writing restored bytes through it.
        open_options.custom_flags(libc::O_NOFOLLOW);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        open_options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = path.open_with_options(open_options)?;
    if file.metadata()?.file_type().is_symlink() {
        return Err(io::Error::other("refusing to restore regular file through symlink").into());
    }

    Ok(file)
}

fn remove_symlink(path: &AbsoluteSystemPath) -> Result<(), CacheError> {
    #[cfg(not(windows))]
    {
        path.remove_file()?;
    }
    #[cfg(windows)]
    {
        path.remove_file().or_else(|_| path.remove_dir())?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::open_for_restore;

    #[test]
    fn open_for_restore_does_not_follow_final_symlink() -> Result<()> {
        let outside_dir = tempdir()?;
        let outside_target = outside_dir.path().join("target.js");
        std::fs::write(&outside_target, b"do not overwrite")?;
        let outside_target = AbsoluteSystemPathBuf::try_from(outside_target.as_path())?;

        let output_dir = tempdir()?;
        let output_dir_path = output_dir.path().to_string_lossy();
        let anchor = AbsoluteSystemPath::new(&output_dir_path)?;
        let restored_file = anchor.join_component("index.js");
        restored_file.symlink_to_file(outside_target.as_str())?;

        let result = open_for_restore(&restored_file, 0o644);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(outside_target.as_path())?,
            b"do not overwrite"
        );
        assert!(restored_file.symlink_metadata()?.is_symlink());

        Ok(())
    }
}

impl CachedDirTree {
    pub fn safe_mkdir_file(
        &mut self,
        anchor: &AbsoluteSystemPath,
        processed_name: &AnchoredSystemPath,
    ) -> Result<(), CacheError> {
        let parent = processed_name.as_path().parent();
        // Handles ./foo and foo
        let is_root_file = parent == Some(Path::new(".")) || parent == Some(Path::new(""));
        if !is_root_file && let Some(dir) = processed_name.parent() {
            self.safe_mkdir_all(anchor, dir, 0o755)?;
        }

        Ok(())
    }
}
