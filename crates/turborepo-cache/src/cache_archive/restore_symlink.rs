use std::{backtrace::Backtrace, io::Read};

use camino::Utf8Path;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
    PathError, UnknownPathType,
};

use crate::{
    CacheError,
    cache_archive::restore_directory::{CachedDirTree, realpath_existing_prefix},
};

pub fn restore_symlink(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &tar::Entry<impl Read>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;

    let linkname = entry
        .link_name()?
        .ok_or_else(|| CacheError::MalformedTar(Backtrace::capture()))?;

    let processed_linkname = validate_linkname(anchor, &processed_name, &linkname)?;

    if processed_linkname.symlink_metadata().is_err() {
        return Err(CacheError::LinkTargetDoesNotExist(
            processed_linkname.to_string(),
            Backtrace::capture(),
        ));
    }

    actually_restore_symlink(
        dir_cache,
        anchor,
        &processed_name,
        &processed_linkname,
        entry,
    )?;

    Ok(processed_name)
}

pub fn restore_symlink_allow_missing_target(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &tar::Entry<impl Read>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;

    let linkname = entry
        .link_name()?
        .ok_or_else(|| CacheError::MalformedTar(Backtrace::capture()))?;
    let processed_linkname = validate_linkname(anchor, &processed_name, &linkname)?;

    actually_restore_symlink(
        dir_cache,
        anchor,
        &processed_name,
        &processed_linkname,
        entry,
    )?;

    Ok(processed_name)
}

fn validate_linkname(
    anchor: &AbsoluteSystemPath,
    processed_name: &AnchoredSystemPathBuf,
    linkname: &std::path::Path,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    let linkname_str = linkname.to_str().ok_or_else(|| {
        CacheError::PathError(
            PathError::InvalidUnicode(linkname.to_string_lossy().to_string()),
            Backtrace::capture(),
        )
    })?;

    if is_windows_absolute_path(linkname_str) {
        return Err(CacheError::LinkOutsideOfDirectory(
            linkname_str.to_string(),
            Backtrace::capture(),
        ));
    }

    let processed_linkname = canonicalize_linkname(anchor, processed_name, linkname)?;
    if !processed_linkname.starts_with(anchor) {
        return Err(CacheError::LinkOutsideOfDirectory(
            linkname_str.to_string(),
            Backtrace::capture(),
        ));
    }

    let raw_linkname = raw_linkname_path(anchor, processed_name, linkname_str)?;
    let real_anchor = anchor.to_realpath()?;
    if let Some(real_target) = realpath_existing_prefix(&raw_linkname)?
        && !real_target.starts_with(&real_anchor)
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            linkname_str.to_string(),
            Backtrace::capture(),
        ));
    }

    Ok(processed_linkname)
}

fn raw_linkname_path(
    anchor: &AbsoluteSystemPath,
    processed_name: &AnchoredSystemPathBuf,
    linkname: &str,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    if Utf8Path::new(linkname).is_absolute() {
        return Ok(AbsoluteSystemPathBuf::new(linkname.to_string())?);
    }

    let source = anchor.resolve(processed_name);
    let Some(parent) = source.parent() else {
        return Err(CacheError::InvalidFilePath(
            source.to_string(),
            Backtrace::capture(),
        ));
    };

    Ok(parent.resolve(AnchoredSystemPath::new(linkname)?))
}

fn is_windows_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.first() == Some(&b'\\')
        || matches!(bytes, [drive, b':', ..] if drive.is_ascii_alphabetic())
}

fn actually_restore_symlink<'a>(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    processed_name: &'a AnchoredSystemPath,
    processed_linkname: &AbsoluteSystemPath,
    entry: &tar::Entry<impl Read>,
) -> Result<&'a AnchoredSystemPath, CacheError> {
    let link_name = entry
        .link_name()?
        .ok_or_else(|| CacheError::MalformedTar(Backtrace::capture()))?;
    let symlink_to = link_name.to_str().ok_or_else(|| {
        CacheError::PathError(
            PathError::InvalidUnicode(link_name.to_string_lossy().to_string()),
            Backtrace::capture(),
        )
    })?;
    let anchored_target = anchor.anchor(processed_linkname)?;
    let physical_target =
        dir_cache.record_symlink(processed_name.to_owned(), anchored_target, symlink_to)?;
    #[cfg(any(unix, windows))]
    let target_is_dir = dir_cache.symlink_target_is_dir(&physical_target);
    #[cfg(not(any(unix, windows)))]
    let target_is_dir = processed_linkname.as_path().is_dir();

    #[cfg(any(unix, windows))]
    {
        let destination = dir_cache.destination(processed_name)?;
        destination.write_symlink(symlink_to, target_is_dir)?;
    }

    #[cfg(not(any(unix, windows)))]
    {
        dir_cache.safe_mkdir_file(anchor, processed_name)?;
        let symlink_from = anchor.resolve(processed_name);
        _ = symlink_from.remove();

        if target_is_dir {
            symlink_from.symlink_to_dir(symlink_to)?;
        } else {
            symlink_from.symlink_to_file(symlink_to)?;
        }
    }

    Ok(processed_name)
}

// canonicalize_linkname determines (lexically) what the resolved path on the
// system will be when linkname is restored verbatim.
pub fn canonicalize_linkname(
    anchor: &AbsoluteSystemPath,
    processed_name: &AnchoredSystemPathBuf,
    linkname: &std::path::Path,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    let linkname = linkname.try_into().map_err(|_| {
        CacheError::PathError(
            PathError::InvalidUnicode(linkname.to_string_lossy().to_string()),
            Backtrace::capture(),
        )
    })?;
    // We don't know _anything_ about linkname. It could be any of:
    //
    // - Absolute Unix Path
    // - Absolute Windows Path
    // - Relative Unix Path
    // - Relative Windows Path
    //
    // We also can't _truly_ distinguish if the path is Unix or Windows.
    // Take for example: `/Users/turbobot/weird-filenames/\foo\/lol`
    // It is a valid file on Unix, but if we do slash conversion it breaks.
    // Or `i\am\a\normal\unix\file\but\super\nested\on\windows`.
    //
    // We also can't safely assume that paths in link targets on one platform
    // should be treated as targets for that platform. The author may be
    // generating an artifact that should work on Windows on a Unix device.
    //
    // Given all of that, our best option is to restore link targets _verbatim_.
    // No modification, no slash conversion.
    //
    // In order to DAG sort them, however, we do need to canonicalize them.
    // We canonicalize them as if we're restoring them verbatim.
    //
    match turbopath::categorize(linkname) {
        // 1. Check to see if the link target is absolute _on the current platform_.
        // If it is an absolute path it's canonical by rule.
        UnknownPathType::Absolute(abs) => Ok(abs),
        // Remaining options:
        // - Absolute (other platform) Path
        // - Relative Unix Path
        // - Relative Windows Path
        //
        // At this point we simply assume that it's a relative path—no matter
        // which separators appear in it and where they appear,  We can't do
        // anything else because the OS will also treat it like that when it is
        // a link target.
        UnknownPathType::Anchored(cleaned_linkname) => {
            let source = anchor.resolve(processed_name);
            let Some(parent) = source.parent() else {
                return Err(CacheError::InvalidFilePath(
                    source.to_string(),
                    Backtrace::capture(),
                ));
            };
            let canonicalized = parent.resolve(&cleaned_linkname).clean()?;

            Ok(canonicalized)
        }
    }
}
