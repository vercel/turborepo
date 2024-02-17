use std::{fs::OpenOptions, io, io::Read, path::Path};

use tar::Entry;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};

use crate::{cache_archive::restore_directory::CachedDirTree, CacheError};

pub fn restore_regular(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &mut Entry<impl Read>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    // Assuming this was a `turbo`-created input, we currently have an
    // RelativeUnixPath. Assuming this is malicious input we don't really care
    // if we do the wrong thing.
    //
    // Note that we don't use `header.path()` as some archive formats have support
    // for longer path names described in separate entries instead of solely in the
    // header
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;

    // We need to traverse `processedName` from base to root split at
    // `os.Separator` to make sure we don't end up following a symlink
    // outside of the restore path.
    dir_cache.safe_mkdir_file(anchor, &processed_name)?;

    let resolved_path = anchor.resolve(&processed_name);
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
