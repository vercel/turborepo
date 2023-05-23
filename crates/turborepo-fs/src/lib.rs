use std::fs::{self, DirBuilder, Metadata};

use anyhow::Result;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use walkdir::WalkDir;

pub fn recursive_copy(
    src: impl AsRef<AbsoluteSystemPath>,
    dst: impl AsRef<AbsoluteSystemPath>,
) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    let src_metadata = src.symlink_metadata()?;
    if src_metadata.is_dir() {
        let walker = WalkDir::new(src.as_path()).follow_links(false);
        for entry in walker.into_iter() {
            match entry {
                Err(e) => {
                    if e.io_error().is_some() {
                        // Matches go behavior where we translate path errors
                        // into skipping the path we're currently walking
                        continue;
                    } else {
                        return Err(e.into());
                    }
                }
                Ok(entry) => {
                    let path = entry.path();
                    let path = AbsoluteSystemPath::new(path)?;
                    let file_type = entry.file_type();
                    // currently we support symlinked files, but not symlinked directories:
                    // For copying, we Mkdir and bail if we encounter a symlink to a directoy
                    // For finding packages, we enumerate the symlink, but don't follow inside
                    // Note that we also don't currently copy broken symlinks
                    let is_dir_or_symlink_to_dir = if file_type.is_dir() {
                        true
                    } else if file_type.is_symlink() {
                        if let Ok(metadata) = path.stat() {
                            metadata.is_dir()
                        } else {
                            // If we have a broken link, skip this entry
                            continue;
                        }
                    } else {
                        false
                    };

                    let suffix = AnchoredSystemPathBuf::new(src, &path)?;
                    let target = dst.resolve(&suffix);
                    if is_dir_or_symlink_to_dir {
                        let src_metadata = entry.metadata()?;
                        make_dir_copy(&target, &src_metadata)?;
                    } else {
                        copy_file_with_type(&path, file_type, &target)?;
                    }
                }
            }
        }
        Ok(())
    } else {
        copy_file_with_type(src, src_metadata.file_type(), dst)
    }
}

fn make_dir_copy(dir: impl AsRef<AbsoluteSystemPath>, src_metadata: &Metadata) -> Result<()> {
    let dir = dir.as_ref();
    let mut builder = DirBuilder::new();
    #[cfg(not(windows))]
    {
        use std::os::unix::{fs::DirBuilderExt, prelude::MetadataExt};
        builder.mode(src_metadata.mode());
    }
    builder.recursive(true);
    builder.create(dir.as_path())?;
    Ok(())
}

pub fn copy_file(
    from: impl AsRef<AbsoluteSystemPath>,
    to: impl AsRef<AbsoluteSystemPath>,
) -> Result<()> {
    let from = from.as_ref();
    let metadata = from.symlink_metadata()?;
    copy_file_with_type(from, metadata.file_type(), to)
}

fn copy_file_with_type(
    from: impl AsRef<AbsoluteSystemPath>,
    from_type: fs::FileType,
    to: impl AsRef<AbsoluteSystemPath>,
) -> Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    if from_type.is_symlink() {
        let target = from.read_link()?;
        to.ensure_dir()?;
        if to.symlink_metadata().is_ok() {
            to.remove_file()?;
        }
        to.symlink_to_file(target)?;
        Ok(())
    } else {
        to.ensure_dir()?;
        fs::copy(from.as_path(), to.as_path())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::Path};

    use turbopath::{AbsoluteSystemPathBuf, PathError};

    use super::*;

    fn tmp_dir<'a>() -> Result<(tempfile::TempDir, AbsoluteSystemPathBuf)> {
        let tmp_dir = tempfile::tempdir()?;
        let dir = AbsoluteSystemPathBuf::new(tmp_dir.path())?;
        Ok((tmp_dir, dir))
    }

    #[test]
    fn test_copy_missing_file() -> Result<()> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        let err = copy_file(src_file, dst_file).unwrap_err();
        let err = err.downcast::<PathError>()?;
        assert_eq!(err.is_io_error(io::ErrorKind::NotFound), true);
        Ok(())
    }

    #[test]
    fn test_basic_copy_file() -> Result<()> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // src exists, dst doesn't
        src_file.create_with_contents("src")?;

        copy_file(&src_file, &dst_file)?;
        assert_file_matches(&src_file, &dst_file);
        Ok(())
    }

    #[test]
    fn test_symlinks() -> Result<()> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_symlink = src_dir.join_component("symlink");

        let (_target_tmp, target_dir) = tmp_dir()?;
        let src_target = target_dir.join_component("target");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // create symlink target
        src_target.create_with_contents("target")?;
        src_symlink.symlink_to_file(src_target.as_path())?;

        copy_file(&src_symlink, &dst_file)?;
        assert_target_matches(&dst_file, &src_target);
        Ok(())
    }

    #[test]
    fn test_copy_file_with_perms() -> Result<()> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // src exists, dst doesn't
        src_file.create_with_contents("src")?;
        src_file.set_readonly()?;

        copy_file(&src_file, &dst_file)?;
        assert_file_matches(&src_file, &dst_file);
        assert_eq!(dst_file.is_readonly()?, true);
        Ok(())
    }

    #[test]
    fn test_recursive_copy() -> Result<()> {
        // Directory layout:
        //
        // <src>/
        //   b
        //   child/
        //     a
        //     link -> ../b
        //     broken -> missing
        //     circle -> ../child
        let (_src_tmp, src_dir) = tmp_dir()?;
        let child_dir = src_dir.join_component("child");
        let a_path = child_dir.join_component("a");
        a_path.ensure_dir()?;
        a_path.create_with_contents("hello")?;

        let b_path = src_dir.join_component("b");
        b_path.create_with_contents("bFile")?;

        let link_path = child_dir.join_component("link");
        link_path.symlink_to_file("../b")?;

        let broken_link_path = child_dir.join_component("broken");
        broken_link_path.symlink_to_file("missing")?;

        let circle_path = child_dir.join_component("circle");
        circle_path.symlink_to_dir("../child")?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;

        recursive_copy(&src_dir, &dst_dir)?;

        // Ensure double copy doesn't error
        recursive_copy(&src_dir, &dst_dir)?;

        let dst_child_path = dst_dir.join_component("child");
        let dst_a_path = dst_child_path.join_component("a");
        assert_file_matches(&a_path, &dst_a_path);

        let dst_b_path = dst_dir.join_component("b");
        assert_file_matches(&b_path, &dst_b_path);

        let dst_link_path = dst_child_path.join_component("link");
        assert_target_matches(&dst_link_path, "../b");

        let dst_broken_path = dst_child_path.join_component("broken");
        assert_eq!(dst_broken_path.as_path().exists(), false);

        // Currently, we convert symlink-to-directory to empty-directory
        // This is very likely not ideal behavior, but leaving this test here to verify
        // that it is what we expect at this point in time.
        let dst_circle_path = dst_child_path.join_component("circle");
        let dst_circle_metadata = fs::symlink_metadata(&dst_circle_path)?;
        assert_eq!(dst_circle_metadata.is_dir(), true);

        let num_files = fs::read_dir(dst_circle_path.as_path())?.into_iter().count();
        assert_eq!(num_files, 0);

        Ok(())
    }

    fn assert_file_matches(a: impl AsRef<AbsoluteSystemPath>, b: impl AsRef<AbsoluteSystemPath>) {
        let a = a.as_ref();
        let b = b.as_ref();
        let a_contents = fs::read_to_string(a.as_path()).unwrap();
        let b_contents = fs::read_to_string(b.as_path()).unwrap();
        assert_eq!(a_contents, b_contents);
    }

    fn assert_target_matches(link: impl AsRef<AbsoluteSystemPath>, expected: impl AsRef<Path>) {
        let link = link.as_ref();
        let path = link.read_link().unwrap();
        assert_eq!(path.as_path(), expected.as_ref());
    }
}
