use std::{
    backtrace::Backtrace,
    fs,
    fs::{File, OpenOptions},
    io::{BufWriter, Read},
    path::Path,
};

use tar::{EntryType, Header};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, RelativeUnixPathBuf};

use crate::CacheError;

// We use an enum to get around Rust's generic restrictions
// i.e. you can't have a function that can return two different
// versions of a generic type like Vec<u32> and Vec<u64>
enum CacheArchive<'a> {
    Compressed(tar::Builder<zstd::Encoder<'a, BufWriter<File>>>),
    Uncompressed(tar::Builder<BufWriter<File>>),
}

impl<'a> CacheArchive<'a> {
    // Appends data to tar builder.
    fn append_data(
        &mut self,
        header: &mut Header,
        path: impl AsRef<Path>,
        body: impl Read,
    ) -> Result<(), CacheError> {
        match self {
            CacheArchive::Compressed(builder) => Ok(builder.append_data(header, path, body)?),
            CacheArchive::Uncompressed(builder) => Ok(builder.append_data(header, path, body)?),
        }
    }

    // Makes a new CacheArchive at the specified path
    // Wires up the chain of writers:
    // tar::Builder -> zstd::Encoder (optional) -> BufWriter -> File
    fn create(path: &AbsoluteSystemPath) -> Result<Self, CacheError> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let file = path.open_with_options(options)?;

        // Flush to disk in 1mb chunks.
        let file_buffer = BufWriter::with_capacity(2usize.pow(20), file);

        let is_compressed = path.extension() == Some("zst".as_ref());

        if is_compressed {
            let zw = zstd::Encoder::new(file_buffer, 0)?;

            Ok(CacheArchive::Compressed(tar::Builder::new(zw)))
        } else {
            Ok(CacheArchive::Uncompressed(tar::Builder::new(file_buffer)))
        }
    }

    // Adds a user-cached item to the tar
    fn add_file(
        &mut self,
        anchor: &AbsoluteSystemPath,
        file_path: &AnchoredSystemPath,
    ) -> Result<(), CacheError> {
        // Resolve the fully-qualified path to the file to read it.
        let source_path = anchor.resolve(file_path);

        // Grab the file info to construct the header.
        let file_info = source_path.symlink_metadata()?;

        // Normalize the path within the cache
        let file_path: RelativeUnixPathBuf = file_path.try_into()?;
        let canonical_file_path = file_path.make_canonical_for_tar(file_info.is_dir());

        let mut header = Self::create_header(&source_path, &file_info)?;

        if matches!(header.entry_type(), EntryType::Regular) && file_info.len() > 0 {
            let file = source_path.open()?;
            self.append_data(&mut header, canonical_file_path.as_path()?, file)?;
        } else {
            self.append_data(
                &mut header,
                canonical_file_path.as_path()?,
                &mut std::io::empty(),
            )?;
        }

        Ok(())
    }

    fn create_header(
        source_path: &AbsoluteSystemPath,
        file_info: &fs::Metadata,
    ) -> Result<Header, CacheError> {
        let mut header = Header::new_gnu();

        let mode: u32;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            mode = file_info.mode();
        }
        #[cfg(windows)]
        {
            // Windows makes up 0o666 for files, which in the Go code
            // we do: (0o666 & 0o755) | 0o111 which produces 0o755
            mode = 0o755
        }
        header.set_mode(mode);

        // Do we need to populate the additional linkname field in Header?
        if file_info.is_symlink() {
            let link = source_path.read_link()?;
            header.set_link_name(link)?;
        }

        // Throw an error if trying to create a cache that contains a type we don't
        // support.
        if !matches!(
            header.entry_type(),
            EntryType::Regular | EntryType::Directory | EntryType::Symlink
        ) {
            return Err(CacheError::UnsupportedFileType(
                header.entry_type(),
                Backtrace::capture(),
            ));
        }

        // Consistent creation
        header.set_uid(0);
        header.set_gid(0);
        header.as_gnu_mut().unwrap().set_atime(0);
        header.set_mtime(0);
        header.as_gnu_mut().unwrap().set_ctime(0);

        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPath;

    use super::*;

    #[test]
    fn test_add_trailing_slash() {
        let mut path = PathBuf::from("foo/bar");
        assert_eq!(path.to_string_lossy(), "foo/bar");
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");

        // Confirm that this is idempotent
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");
    }

    #[test]
    fn create_tar_with_really_long_name() -> Result<()> {
        let dir = tempdir()?;

        let anchor = AbsoluteSystemPath::new(dir.path())?;
        let out_path = anchor.join_component("test.tar");
        let mut archive = CacheArchive::create(&out_path)?;
        let really_long_file = AnchoredSystemPath::new("this-is-a-really-really-really-long-path-like-so-very-long-that-i-can-list-all-of-my-favorite-directors-like-edward-yang-claire-denis-lucrecia-martel-wong-kar-wai-even-kurosawa").unwrap();

        let really_long_path = anchor.resolve(really_long_file);
        really_long_path.create_with_contents("The End!")?;
        archive.add_file(anchor, really_long_file)?;

        Ok(())
    }
}
