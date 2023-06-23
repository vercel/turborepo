use std::{
    backtrace::Backtrace,
    fs,
    fs::OpenOptions,
    io::{BufWriter, Read, Write},
    path::Path,
};

use tar::{EntryType, Header};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, RelativeUnixPathBuf};

use crate::CacheError;

struct CacheWriter {
    builder: tar::Builder<Box<dyn Write>>,
}

impl CacheWriter {
    // Appends data to tar builder.
    fn append_data(
        &mut self,
        header: &mut Header,
        path: impl AsRef<Path>,
        body: impl Read,
    ) -> Result<(), CacheError> {
        Ok(self.builder.append_data(header, path, body)?)
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

        let is_compressed = path.extension() == Some("zst");

        if is_compressed {
            let zw = zstd::Encoder::new(file_buffer, 0)?.auto_finish();

            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(zw)),
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(file_buffer)),
            })
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
        let mut file_path = RelativeUnixPathBuf::new(file_path.as_str())?;
        file_path.make_canonical_for_tar(file_info.is_dir());

        let mut header = Self::create_header(&source_path, &file_info)?;

        if matches!(header.entry_type(), EntryType::Regular) && file_info.len() > 0 {
            let file = source_path.open()?;
            self.append_data(&mut header, file_path.as_str(), file)?;
        } else {
            self.append_data(&mut header, file_path.as_str(), &mut std::io::empty())?;
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
            header.set_entry_type(EntryType::Symlink);
        } else if file_info.is_dir() {
            header.set_entry_type(EntryType::Directory);
        } else if file_info.is_file() {
            header.set_entry_type(EntryType::Regular);
        } else {
            // Throw an error if trying to create a cache that contains a type we don't
            // support.
            return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
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
    #[cfg(unix)]
    fn test_add_trailing_slash_unix() {
        let mut path = PathBuf::from("foo/bar");
        assert_eq!(path.to_string_lossy(), "foo/bar");
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");

        // Confirm that this is idempotent
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");
    }

    #[test]
    #[cfg(windows)]
    fn test_add_trailing_slash_windows() {
        let mut path = PathBuf::from("foo\\bar");
        assert_eq!(path.to_string_lossy(), "foo\\bar");
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo\\bar\\");

        // Confirm that this is idempotent
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo\\bar\\");
    }

    #[test]
    fn create_tar_with_really_long_name() -> Result<()> {
        let dir = tempdir()?;

        let anchor = AbsoluteSystemPath::new(dir.path().to_str().unwrap())?;
        let out_path = anchor.join_component("test.tar");
        let mut archive = CacheWriter::create(&out_path)?;
        let really_long_file = AnchoredSystemPath::new("this-is-a-really-really-really-long-path-like-so-very-long-that-i-can-list-all-of-my-favorite-directors-like-edward-yang-claire-denis-lucrecia-martel-wong-kar-wai-even-kurosawa").unwrap();

        let really_long_path = anchor.resolve(really_long_file);
        really_long_path.create_with_contents("The End!")?;
        archive.add_file(anchor, really_long_file)?;

        Ok(())
    }
}
