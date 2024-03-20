#![deny(clippy::all)]

use std::{
    fs::{DirBuilder, FileType, Metadata},
    io,
};

use fs_err as fs;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use walkdir::WalkDir;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("error walking directory during recursive copy: {0}")]
    Walk(#[from] walkdir::Error),
}

pub fn recursive_copy(
    src: impl AsRef<AbsoluteSystemPath>,
    dst: impl AsRef<AbsoluteSystemPath>,
) -> Result<(), Error> {
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
                    let path = AbsoluteSystemPath::from_std_path(path)?;
                    let file_type = entry.file_type();

                    // Note that we also don't currently copy broken symlinks
                    if file_type.is_symlink() && path.stat().is_err() {
                        // If we have a broken link, skip this entry
                        continue;
                    }

                    let suffix = AnchoredSystemPathBuf::new(src, path)?;
                    let target = dst.resolve(&suffix);
                    if file_type.is_dir() {
                        let src_metadata = entry.metadata()?;
                        make_dir_copy(&target, &src_metadata)?;
                    } else {
                        copy_file_with_type(path, file_type, &target)?;
                    }
                }
            }
        }
        Ok(())
    } else {
        Ok(copy_file_with_type(src, src_metadata.file_type(), dst)?)
    }
}

fn make_dir_copy(
    dir: impl AsRef<AbsoluteSystemPath>,
    #[allow(unused_variables)] src_metadata: &Metadata,
) -> Result<(), Error> {
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
) -> Result<(), Error> {
    let from = from.as_ref();
    let metadata = from.symlink_metadata()?;
    copy_file_with_type(from, metadata.file_type(), to)
}

fn copy_file_with_type(
    from: impl AsRef<AbsoluteSystemPath>,
    from_type: FileType,
    to: impl AsRef<AbsoluteSystemPath>,
) -> Result<(), Error> {
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

    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn tmp_dir() -> Result<(tempfile::TempDir, AbsoluteSystemPathBuf), Error> {
        let tmp_dir = tempfile::tempdir()?;
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())?;
        Ok((tmp_dir, dir))
    }

    #[test]
    fn test_copy_missing_file() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        let err = copy_file(src_file, dst_file).unwrap_err();
        let Error::Path(err) = err else {
            panic!("expected path error");
        };
        assert!(err.is_io_error(io::ErrorKind::NotFound));
        Ok(())
    }

    #[test]
    fn test_basic_copy_file() -> Result<(), Error> {
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
    fn test_symlinks() -> Result<(), Error> {
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
    fn test_symlink_to_dir() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_symlink = src_dir.join_component("symlink");

        let (_target_tmp, target_dir) = tmp_dir()?;
        let src_target = target_dir.join_component("target");

        let target_a = src_target.join_component("a");
        target_a.ensure_dir()?;
        target_a.create_with_contents("solid")?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // create symlink target
        src_symlink.symlink_to_dir(src_target.as_path())?;

        copy_file(&src_symlink, &dst_file)?;
        assert_target_matches(&dst_file, &src_target);

        let target = dst_file.read_link()?;
        assert_eq!(target.read_dir()?.count(), 1);

        Ok(())
    }

    #[test]
    fn test_copy_file_with_perms() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // src exists, dst doesn't
        src_file.create_with_contents("src")?;
        src_file.set_readonly()?;

        copy_file(&src_file, &dst_file)?;
        assert_file_matches(&src_file, &dst_file);
        assert!(dst_file.is_readonly()?);
        Ok(())
    }

    #[test]
    fn test_recursive_copy() -> Result<(), Error> {
        // Directory layout:
        //
        // <src>/
        //   b
        //   child/
        //     a
        //     link -> ../b
        //     broken -> missing
        //     circle -> ../child
        //     other -> ../sibling
        //   sibling/
        //     c
        let (_src_tmp, src_dir) = tmp_dir()?;

        let sibling_dir = src_dir.join_component("sibling");
        let c_path = sibling_dir.join_component("c");
        c_path.ensure_dir()?;
        c_path.create_with_contents("right here")?;

        let child_dir = src_dir.join_component("child");
        let a_path = child_dir.join_component("a");
        a_path.ensure_dir()?;
        a_path.create_with_contents("hello")?;

        let b_path = src_dir.join_component("b");
        b_path.create_with_contents("bFile")?;

        let link_path = child_dir.join_component("link");
        link_path.symlink_to_file(["..", "b"].join(std::path::MAIN_SEPARATOR_STR))?;

        let broken_link_path = child_dir.join_component("broken");
        broken_link_path.symlink_to_file("missing")?;

        let circle_path = child_dir.join_component("circle");
        circle_path.symlink_to_dir(["..", "child"].join(std::path::MAIN_SEPARATOR_STR))?;

        let other_path = child_dir.join_component("other");
        other_path.symlink_to_dir(["..", "sibling"].join(std::path::MAIN_SEPARATOR_STR))?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;

        recursive_copy(&src_dir, &dst_dir)?;

        // Ensure double copy doesn't error
        recursive_copy(&src_dir, &dst_dir)?;

        let dst_child_path = dst_dir.join_component("child");
        let dst_a_path = dst_child_path.join_component("a");
        assert_file_matches(&a_path, dst_a_path);

        let dst_b_path = dst_dir.join_component("b");
        assert_file_matches(&b_path, dst_b_path);

        let dst_link_path = dst_child_path.join_component("link");
        assert_target_matches(
            dst_link_path,
            ["..", "b"].join(std::path::MAIN_SEPARATOR_STR),
        );

        let dst_broken_path = dst_child_path.join_component("broken");
        assert!(!dst_broken_path.as_path().exists());

        let dst_circle_path = dst_child_path.join_component("circle");
        let dst_circle_metadata = fs::symlink_metadata(dst_circle_path)?;
        assert!(dst_circle_metadata.is_symlink());

        let num_files = fs::read_dir(dst_child_path.as_path())?.count();
        // We don't copy the broken symlink so there are only 4 entries
        assert_eq!(num_files, 4);

        let dst_other_path = dst_child_path.join_component("other");

        let dst_other_metadata = fs::symlink_metadata(dst_other_path.as_path())?;
        assert!(dst_other_metadata.is_symlink());

        let dst_c_path = dst_other_path.join_component("c");

        assert_file_matches(&c_path, dst_c_path);

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
