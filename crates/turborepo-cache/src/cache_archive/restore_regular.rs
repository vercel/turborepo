use std::{fs::OpenOptions, io, io::Read, path::Path};

use tar::Entry;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};

use crate::{CacheError, cache_archive::restore_directory::CachedDirTree};

pub fn restore_regular(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &mut Entry<impl Read>,
    manifest: Option<&super::restore_manifest::RestoreManifest>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;
    let resolved_path = anchor.resolve(&processed_name);

    // Check if the file on disk already matches the manifest entry.
    // If so, skip the write and just advance the tar stream.
    if let Some(manifest) = manifest
        && manifest.file_matches(processed_name.as_str(), &resolved_path)
    {
        io::copy(entry, &mut io::sink())?;
        return Ok(processed_name);
    }

    dir_cache.safe_mkdir_file(anchor, &processed_name)?;

    let mut open_options = OpenOptions::new();
    open_options.write(true).truncate(true).create(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let header = entry.header();
        open_options.mode(header.mode()?);
    }

    let mut file = open_options.open(resolved_path.as_path())?;
    io::copy(entry, &mut file)?;

    Ok(processed_name)
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
        if !is_root_file {
            let dir = processed_name.parent().unwrap();
            self.safe_mkdir_all(anchor, dir, 0o755)?;
        }

        Ok(())
    }
}
