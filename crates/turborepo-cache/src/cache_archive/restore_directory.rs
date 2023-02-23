use std::{
    backtrace::Backtrace,
    fs,
    path::{Component, Path},
};

use tar::Header;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::{cache_archive::restore::canonicalize_name, CacheError};

pub fn restore_directory(
    anchor: &AbsoluteSystemPath,
    header: &Header,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = canonicalize_name(&header.path()?)?;

    safe_mkdir_all(anchor, processed_name.as_anchored_path(), header.mode()?)?;

    Ok(processed_name)
}

pub fn safe_mkdir_all(
    anchor: &AbsoluteSystemPath,
    processed_name: &AnchoredSystemPath,
    mode: u32,
) -> Result<(), CacheError> {
    // Iterate through path segments by os.Separator, appending them onto
    // current_path. Check to see if that path segment is a symlink
    // with a target outside of anchor.
    let mut calculated_anchor = anchor.to_owned();
    for component in processed_name.as_path().components() {
        calculated_anchor = check_path(
            anchor,
            &calculated_anchor,
            &AnchoredSystemPath::new(Path::new(component.as_os_str()))?,
        )?;
    }

    // If we have made it here we know that it is safe to call fs::create_dir_all
    // on the join of anchor and processed_name.
    //
    // This could _still_ error, but we don't care.
    let resolved_name = anchor.resolve(processed_name);
    fs::create_dir_all(&resolved_name)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(&resolved_name)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(&resolved_name, permissions)?;
    }

    Ok(())
}

fn check_path(
    original_anchor: &AbsoluteSystemPath,
    accumulated_anchor: &AbsoluteSystemPath,
    segment: &AnchoredSystemPath,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    // Check if the segment itself is sneakily an absolute path...
    // (looking at you, Windows. CON, AUX...)
    if segment
        .as_path()
        .components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            segment.to_string(),
            Backtrace::capture(),
        ));
    }

    let combined_path = accumulated_anchor.resolve(segment);
    let Ok(file_info) = fs::symlink_metadata(combined_path.as_path()) else {
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
    let link_target = fs::read_link(combined_path.as_path())?;
    debug!(
        "link source: {:?}, link target {:?}",
        combined_path, link_target
    );
    if link_target.is_absolute() {
        let absolute_link_target = AbsoluteSystemPathBuf::new(link_target.clone())?;
        if path_clean::clean(&absolute_link_target).starts_with(&original_anchor) {
            return Ok(absolute_link_target);
        }
    } else {
        let relative_link_target = AnchoredSystemPath::new(&link_target)?;
        // We clean here to resolve the `..` and `.` segments.
        let computed_target = path_clean::clean(accumulated_anchor.resolve(relative_link_target));
        if computed_target.starts_with(&original_anchor) {
            return check_path(original_anchor, accumulated_anchor, &relative_link_target);
        }
    }

    Err(CacheError::LinkOutsideOfDirectory(
        link_target.to_string_lossy().to_string(),
        Backtrace::capture(),
    ))
}
