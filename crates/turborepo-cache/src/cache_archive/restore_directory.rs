use std::{backtrace::Backtrace, collections::HashSet, ffi::OsString, io};

use camino::Utf8Component;
use tar::Entry;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::CacheError;

pub fn restore_directory(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &Entry<impl io::Read>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;

    dir_cache.safe_mkdir_all(anchor, &processed_name, entry.header().mode()?)?;

    Ok(processed_name)
}

pub struct CachedDirTree {
    anchor_at_depth: Vec<AbsoluteSystemPathBuf>,
    prefix: Vec<OsString>,
    restored_symlinks: HashSet<AnchoredSystemPathBuf>,
}

impl CachedDirTree {
    pub fn new(initial_anchor: AbsoluteSystemPathBuf) -> Self {
        CachedDirTree {
            anchor_at_depth: vec![initial_anchor],
            prefix: vec![],
            restored_symlinks: HashSet::new(),
        }
    }

    pub fn record_symlink(&mut self, path: AnchoredSystemPathBuf) {
        self.restored_symlinks.insert(path);
    }

    // On Windows, directory symlinks require remove_dir rather than
    // remove_file. Try remove_file first; fall back to remove_dir.
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

    // Given a path, checks the dir cache to determine where we actually need
    // to start restoring, i.e. which directories we can skip over because
    // we've already created them.
    // Returns the anchor at the depth where we need to start restoring, and
    // the index into the path components where we need to start restoring.
    fn get_starting_point(&mut self, path: &AnchoredSystemPath) -> (AbsoluteSystemPathBuf, usize) {
        let mut i = 0;
        for (idx, (path_component, prefix_component)) in
            path.components().zip(self.prefix.iter()).enumerate()
        {
            i = idx;
            if path_component.as_os_str() != prefix_component.as_os_str() {
                break;
            }
        }
        let anchor = self.anchor_at_depth[i].clone();

        self.anchor_at_depth.truncate(i + 1);
        self.prefix.truncate(i);

        (anchor, i)
    }

    fn update(&mut self, anchor: AbsoluteSystemPathBuf, new_component: OsString) {
        self.anchor_at_depth.push(anchor);
        self.prefix.push(new_component);
    }

    // Windows doesn't have file modes, so mode is unused
    #[allow(unused_variables)]
    pub fn safe_mkdir_all(
        &mut self,
        anchor: &AbsoluteSystemPath,
        processed_name: &AnchoredSystemPath,
        mode: u32,
    ) -> Result<(), CacheError> {
        // Iterate through path segments by os.Separator, appending them onto
        // current_path. Check to see if that path segment is a symlink
        // with a target outside of anchor.
        let (mut calculated_anchor, start_idx) = self.get_starting_point(processed_name);

        // Build the anchored path incrementally so we can check each component
        // against restored_symlinks.
        let components: Vec<_> = processed_name.components().collect();
        let mut current_anchored: Option<AnchoredSystemPathBuf> = None;

        for (idx, component) in components.iter().enumerate() {
            current_anchored = Some(match &current_anchored {
                None => AnchoredSystemPathBuf::from_raw(component.as_str())?,
                Some(p) => AnchoredSystemPath::new(p.as_str())?.join_component(component.as_str()),
            });

            if idx < start_idx {
                continue;
            }

            // Check if this component is a pre-existing symlink that should be
            // replaced with a real directory. Symlinks restored during the
            // current operation are preserved (they were intentionally placed
            // by the same tar archive).
            let Some(current) = current_anchored.as_ref() else {
                continue;
            };
            let literal_path = anchor.resolve(AnchoredSystemPath::new(current.as_str())?);
            if let Ok(metadata) = literal_path.symlink_metadata()
                && metadata.is_symlink()
                && !self.restored_symlinks.contains(current)
            {
                debug!(
                    "replacing pre-existing symlink at {:?} with directory",
                    literal_path
                );
                Self::remove_symlink(&literal_path)?;
                // Fall through to check_path: the symlink is gone, so
                // check_path will see a non-existent path and accept it.
            }

            calculated_anchor = check_path(
                anchor,
                &calculated_anchor,
                AnchoredSystemPath::new(component.as_str())?,
            )?;

            self.update(
                calculated_anchor.clone(),
                component.as_os_str().to_os_string(),
            );
        }

        // Directory modes are only applied when creating directories. A later
        // path-based chmod could be redirected if a checked component is swapped.
        let resolved_name = anchor.resolve(processed_name);
        let directory_exists = resolved_name.try_exists();
        if matches!(directory_exists, Ok(false)) {
            create_dir_all_with_mode(&resolved_name, mode)?;
        }

        Ok(())
    }
}

#[cfg(unix)]
fn create_dir_all_with_mode(path: &AbsoluteSystemPath, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(mode & 0o7777)
        .create(path.as_path())
}

#[cfg(windows)]
fn create_dir_all_with_mode(path: &AbsoluteSystemPath, _mode: u32) -> io::Result<()> {
    // Windows restore does not apply tar modes, so there is no chmod equivalent
    // to move to creation time.
    path.create_dir_all()
}

#[cfg(not(any(unix, windows)))]
fn create_dir_all_with_mode(path: &AbsoluteSystemPath, _mode: u32) -> io::Result<()> {
    path.create_dir_all()
}

fn check_path(
    original_anchor: &AbsoluteSystemPath,
    accumulated_anchor: &AbsoluteSystemPath,
    segment: &AnchoredSystemPath,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    // Check if the segment itself is sneakily an absolute path...
    // (looking at you, Windows. CON, AUX...)
    if segment
        .components()
        .any(|c| matches!(c, Utf8Component::Prefix(_) | Utf8Component::RootDir))
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            segment.to_string(),
            Backtrace::capture(),
        ));
    }

    let combined_path = accumulated_anchor.resolve(segment);
    let Ok(file_info) = combined_path.symlink_metadata() else {
        // Getting an error here means we failed to stat the path.
        // Assume that means we're safe and continue.
        return Ok(combined_path);
    };

    // If we don't have a symlink, it's safe
    if !file_info.is_symlink() {
        return Ok(combined_path);
    }

    // Check the real target of any existing path prefix so archive-created
    // symlink chains cannot hide an escape behind lexical cleaning.
    let link_target = combined_path.read_link()?;
    debug!(
        "link source: {:?}, link target {:?}",
        combined_path, link_target
    );
    let link_target_path = if link_target.is_absolute() {
        AbsoluteSystemPathBuf::new(link_target.clone())?
    } else {
        accumulated_anchor.resolve(AnchoredSystemPath::new(link_target.as_str())?)
    };

    let real_anchor = original_anchor.to_realpath()?;
    if let Some(real_target) = realpath_existing_prefix(&link_target_path)?
        && !real_target.starts_with(&real_anchor)
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            link_target.to_string(),
            Backtrace::capture(),
        ));
    }

    let clean_target = link_target_path.clean()?;
    if clean_target.starts_with(original_anchor) {
        return Ok(clean_target);
    }

    Err(CacheError::LinkOutsideOfDirectory(
        link_target.to_string(),
        Backtrace::capture(),
    ))
}

pub(crate) fn realpath_existing_prefix(
    path: &AbsoluteSystemPath,
) -> Result<Option<AbsoluteSystemPathBuf>, CacheError> {
    let mut candidate = path.as_path().to_path_buf();

    loop {
        let candidate_path = AbsoluteSystemPathBuf::try_from(candidate.as_std_path())?;
        match candidate_path.to_realpath() {
            Ok(realpath) => return Ok(Some(realpath)),
            Err(err) if err.is_io_error(io::ErrorKind::NotFound) => {
                if !candidate.pop() {
                    return Ok(None);
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}
