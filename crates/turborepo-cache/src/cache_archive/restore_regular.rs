#[cfg(not(any(unix, windows)))]
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
use std::{io, io::Read};

use tar::Entry;
#[cfg(not(any(unix, windows)))]
use turbopath::AnchoredSystemPath;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

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

    #[cfg(any(unix, windows))]
    {
        let mode = sanitized_mode(entry.header().mode()?);
        let destination = dir_cache.destination(&processed_name)?;
        destination.write_regular(entry, mode)?;
        dir_cache.forget_symlink(&processed_name)?;
    }

    #[cfg(not(any(unix, windows)))]
    {
        dir_cache.safe_mkdir_file(anchor, &processed_name)?;
        if let Ok(metadata) = resolved_path.symlink_metadata()
            && metadata.is_symlink()
        {
            remove_symlink(&resolved_path)?;
        }

        let mut file = open_for_restore(&resolved_path, sanitized_mode(0))?;
        io::copy(entry, &mut file)?;
    }

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

#[cfg(not(any(unix, windows)))]
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

#[cfg(not(any(unix, windows)))]
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
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use crate::cache_archive::restore_directory::CachedDirTree;

    #[test]
    fn restore_root_does_not_follow_symlink() -> Result<()> {
        let container = tempdir()?;
        let container = AbsoluteSystemPathBuf::try_from(container.path())?;
        let real_root = container.join_component("real");
        real_root.create_dir_all()?;
        let linked_root = container.join_component("linked");
        linked_root.symlink_to_dir(real_root.as_str())?;

        assert!(CachedDirTree::new(linked_root).is_err());

        Ok(())
    }

    #[test]
    fn restore_destination_does_not_follow_final_symlink() -> Result<()> {
        let outside_dir = tempdir()?;
        let outside_target = outside_dir.path().join("target.js");
        std::fs::write(&outside_target, b"do not overwrite")?;
        let outside_target = AbsoluteSystemPathBuf::try_from(outside_target.as_path())?;

        let output_dir = tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(output_dir.path())?;
        let path = AnchoredSystemPathBuf::from_raw("index.js")?;
        let dir_cache = CachedDirTree::new(anchor.clone())?;
        let destination = dir_cache.destination(&path)?;
        anchor
            .join_component("index.js")
            .symlink_to_file(outside_target.as_str())?;

        destination.write_regular(&mut &b"restored"[..], 0o644)?;

        assert_eq!(
            std::fs::read(outside_target.as_path())?,
            b"do not overwrite"
        );
        assert_eq!(
            std::fs::read(anchor.join_component("index.js").as_path())?,
            b"restored"
        );
        assert!(
            !anchor
                .join_component("index.js")
                .symlink_metadata()?
                .is_symlink()
        );

        Ok(())
    }

    #[test]
    fn restore_destination_is_stable_when_intermediate_directory_is_replaced() -> Result<()> {
        let outside_dir = tempdir()?;
        let output_dir = tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(output_dir.path())?;
        let path = AnchoredSystemPathBuf::from_raw("escape/payload")?;
        let dir_cache = CachedDirTree::new(anchor.clone())?;
        let destination = dir_cache.destination(&path)?;

        let escape = anchor.join_component("escape");
        let detached = anchor.join_component("detached");
        std::fs::rename(escape.as_path(), detached.as_path())?;
        escape.symlink_to_dir(outside_dir.path().to_string_lossy())?;

        destination.write_regular(&mut &b"restored"[..], 0o644)?;

        assert!(!outside_dir.path().join("payload").exists());
        assert_eq!(
            std::fs::read(detached.join_component("payload").as_path())?,
            b"restored"
        );

        Ok(())
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use crate::cache_archive::restore_directory::CachedDirTree;

    #[test]
    fn restore_root_does_not_follow_reparse_point() -> Result<()> {
        let container = tempdir()?;
        let container = AbsoluteSystemPathBuf::try_from(container.path())?;
        let real_root = container.join_component("real");
        real_root.create_dir_all()?;
        let linked_root = container.join_component("linked");
        linked_root.symlink_to_dir(real_root.as_str())?;

        assert!(CachedDirTree::new(linked_root).is_err());

        Ok(())
    }

    #[test]
    fn restore_destination_replaces_final_symlink() -> Result<()> {
        let outside_dir = tempdir()?;
        let outside_target = outside_dir.path().join("target.js");
        std::fs::write(&outside_target, b"do not overwrite")?;
        let outside_target = AbsoluteSystemPathBuf::try_from(outside_target.as_path())?;

        let output_dir = tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(output_dir.path())?;
        let path = AnchoredSystemPathBuf::from_raw("index.js")?;
        let dir_cache = CachedDirTree::new(anchor.clone())?;
        let destination = dir_cache.destination(&path)?;
        anchor
            .join_component("index.js")
            .symlink_to_file(outside_target.as_str())?;

        destination.write_regular(&mut &b"restored"[..], 0)?;

        assert_eq!(
            std::fs::read(outside_target.as_path())?,
            b"do not overwrite"
        );
        assert_eq!(
            std::fs::read(anchor.join_component("index.js").as_path())?,
            b"restored"
        );

        Ok(())
    }

    #[test]
    fn restore_destination_locks_intermediate_directory() -> Result<()> {
        let output_dir = tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(output_dir.path())?;
        let path = AnchoredSystemPathBuf::from_raw("escape/payload")?;
        let dir_cache = CachedDirTree::new(anchor.clone())?;
        let destination = dir_cache.destination(&path)?;

        let escape = anchor.join_component("escape");
        let detached = anchor.join_component("detached");
        assert!(std::fs::rename(escape.as_path(), detached.as_path()).is_err());

        destination.write_regular(&mut &b"restored"[..], 0)?;
        assert_eq!(
            std::fs::read(escape.join_component("payload").as_path())?,
            b"restored"
        );

        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
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
