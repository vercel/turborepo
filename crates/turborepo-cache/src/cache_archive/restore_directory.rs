use std::{backtrace::Backtrace, ffi::OsString};

use camino::Utf8Component;
use tar::Header;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::CacheError;

pub fn restore_directory(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    header: &Header,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&header.path()?)?;

    dir_cache.safe_mkdir_all(anchor, &processed_name, header.mode()?)?;

    Ok(processed_name)
}

pub struct CachedDirTree {
    anchor_at_depth: Vec<AbsoluteSystemPathBuf>,
    prefix: Vec<OsString>,
}

impl CachedDirTree {
    pub fn new(initial_anchor: AbsoluteSystemPathBuf) -> Self {
        CachedDirTree {
            anchor_at_depth: vec![initial_anchor],
            prefix: vec![],
        }
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
        for component in processed_name.components().skip(start_idx) {
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

        // If we have made it here we know that it is safe to call fs::create_dir_all
        // on the join of anchor and processed_name.
        //
        // This could _still_ error, but we don't care.
        let resolved_name = anchor.resolve(processed_name);
        let directory_exists = resolved_name.try_exists();
        if matches!(directory_exists, Ok(false)) {
            resolved_name.create_dir_all()?;
        }

        #[cfg(unix)]
        {
            use std::{fs, os::unix::fs::PermissionsExt};

            let metadata = resolved_name.symlink_metadata()?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&resolved_name, permissions)?;
        }

        Ok(())
    }
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

    // Check to see if the symlink targets outside of the originalAnchor.
    // We don't do eval symlinks because we could find ourself in a totally
    // different place.

    // 1. Get the target.
    let link_target = combined_path.read_link()?;
    debug!(
        "link source: {:?}, link target {:?}",
        combined_path, link_target
    );
    if link_target.is_absolute() {
        let absolute_link_target = AbsoluteSystemPathBuf::new(link_target.clone())?;
        if path_clean::clean(&absolute_link_target).starts_with(original_anchor) {
            return Ok(absolute_link_target);
        }
    } else {
        let relative_link_target = AnchoredSystemPath::new(link_target.as_str())?;
        // We clean here to resolve the `..` and `.` segments.
        let computed_target = path_clean::clean(accumulated_anchor.resolve(relative_link_target));
        if computed_target.starts_with(original_anchor) {
            return check_path(original_anchor, accumulated_anchor, relative_link_target);
        }
    }

    Err(CacheError::LinkOutsideOfDirectory(
        link_target.to_string(),
        Backtrace::capture(),
    ))
}
