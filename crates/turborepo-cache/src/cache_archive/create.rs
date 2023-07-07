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

    pub fn finish(mut self) -> Result<(), CacheError> {
        Ok(self.builder.finish()?)
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
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use super::*;
    use crate::cache_archive::restore::CacheReader;

    #[derive(Debug)]
    enum FileType {
        Dir,
        Symlink { linkname: String },
        Fifo,
        File,
    }

    #[derive(Debug)]
    struct CreateFileDefinition {
        path: AnchoredSystemPathBuf,
        mode: u32,
        file_type: FileType,
    }

    fn create_entry(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        match &file.file_type {
            FileType::Dir => create_dir(anchor, file),
            FileType::Symlink { linkname } => create_symlink(anchor, file, &linkname),
            FileType::Fifo => create_fifo(anchor, file),
            FileType::File => create_file(anchor, file),
        }
    }

    fn create_dir(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        let path = anchor.resolve(&file.path);
        path.create_dir_all()?;

        #[cfg(unix)]
        {
            path.set_mode(file.mode & 0o777)?;
        }

        Ok(())
    }

    fn create_symlink(
        anchor: &AbsoluteSystemPath,
        file: &CreateFileDefinition,
        linkname: &str,
    ) -> Result<()> {
        let path = anchor.resolve(&file.path);
        path.symlink_to_file(&linkname)?;

        Ok(())
    }

    #[cfg(unix)]
    fn create_fifo(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        use std::ffi::CString;

        let path = anchor.resolve(&file.path);
        let path_cstr = CString::new(path.as_str())?;

        unsafe {
            libc::mkfifo(path_cstr.as_ptr(), 0o644);
        }

        Ok(())
    }

    #[cfg(windows)]
    fn create_fifo(_: &AbsoluteSystemPath, _: &CreateFileDefinition) -> Result<()> {
        Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()).into())
    }

    fn create_file(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        let path = anchor.resolve(&file.path);
        fs::write(&path, b"file contents")?;
        #[cfg(unix)]
        {
            path.set_mode(file.mode & 0o777)?;
        }

        Ok(())
    }

    #[test_case(
      vec![
         CreateFileDefinition {
           path: AnchoredSystemPathBuf::from_raw("hello world.txt").unwrap(),
           mode: 0o644,
           file_type: FileType::File,
         }
      ],
      "db05810ef8714bc849a27d2b78a267c03862cd5259a5c7fb916e92a1ef912da68a4c92032d8e984e241e12fb85a4b41574009922d740c7e66faf50a00682003c",
      "db05810ef8714bc849a27d2b78a267c03862cd5259a5c7fb916e92a1ef912da68a4c92032d8e984e241e12fb85a4b41574009922d740c7e66faf50a00682003c",
      "224fda5e3b1db1e4a7ede1024e09ea70add3243ce1227d28b3f8fc40bca98e14d381efe4e8affc4fef8eb21b4ff42753f9923aac60038680602c117b15748ca1",
      None
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "two".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "three".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "real".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                mode: 0o777,
                file_type: FileType::File,
            }
        ],
        "7cb91627c62368cfa15160f9f018de3320ee0cf267625d37265d162ae3b0dea64b8126aac9769922796e3eb864189efd7c5555c4eea8999c91cbbbe695852111",
        "04f27e900a4a189cf60ce21e1864ac3f77c3bc9276026a94329a5314e20a3f2671e2ac949025840f46dc9fe72f9f566f1f2c0848a3f203ba77564fae204e886c",
        "1a618c123f9f09bbca9052121d13eea3192fa3addc61eb11f6dcb794f1093abba204510d126ca1f974d5db9a6e728c1e5d3b7c099faf904e494476277178d657",
        None
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("parent").unwrap(),
                mode: 0o777,
                file_type: FileType::Dir,
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("parent/child").unwrap(),
                mode: 0o644,
                file_type: FileType::File,
            },
        ],
        "919de777e4d43eb072939d2e0664f9df533bd24ec357eacab83dcb8a64e2723f3ee5ecb277d1cf24538339fe06d210563188052d08dab146a8463fdb6898d655",
        "919de777e4d43eb072939d2e0664f9df533bd24ec357eacab83dcb8a64e2723f3ee5ecb277d1cf24538339fe06d210563188052d08dab146a8463fdb6898d655",
        "f12ff4c12722f2c901885c67d232c325b604d54e5b67c35da01ab133fd36e637bf8d2501b463ffb6e4438efaf2a59526a85218e00c0f6b7b5594c8f4154c1ece",
        None
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Symlink { linkname: "two".to_string() },
            },
        ],
        "40ce0d42109bb5e5a6b1d4ba9087a317b4c1c6c51822a57c9cb983f878b0ff765637c05fadd4bac32c8dd2b496c2a24825b183d9720b0cdd5b33f9248b692cc1",
        "c113763393a9fb498cc676e1fe4843206cda665afe2144829fe7434da9e81f0cf6d11386fa79877d3c514d108f9696740256af952b57d32216fbed2eb2fb049d",
        "fe692a000551a60da6cc303a9552a16d7ed5c462e33153a96824e96596da6d642fc671448f06f34e9685a13fe5bbb4220f59db73a856626b8a0962916a8f5ea3",
        None
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Fifo,
            }
        ],
        "",
        "",
        "",
        Some("attempted to create unsupported file type")
    )]
    fn test_create(
        files: Vec<CreateFileDefinition>,
        #[allow(unused_variables)] expected_darwin: &str,
        #[allow(unused_variables)] expected_unix: &str,
        #[allow(unused_variables)] expected_windows: &str,
        #[allow(unused_variables)] expected_err: Option<&str>,
    ) -> Result<()> {
        'outer: for compressed in [false, true] {
            let input_dir = tempdir()?;
            let archive_dir = tempdir()?;
            let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;
            let archive_path = if compressed {
                AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar.zst"))?
            } else {
                AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?
            };

            let mut cache_archive = CacheWriter::create(&archive_path)?;

            for file in files.iter() {
                let result = create_entry(&input_dir_path, file);
                if let Err(err) = result {
                    assert!(expected_err.is_some());
                    assert_eq!(err.to_string(), expected_err.unwrap());
                    continue 'outer;
                }

                let result = cache_archive.add_file(&input_dir_path, &file.path);
                if let Err(err) = result {
                    assert!(expected_err.is_some());
                    assert_eq!(err.to_string(), expected_err.unwrap());
                    continue 'outer;
                }
            }

            cache_archive.finish()?;

            if compressed {
                let opened_cache_archive = CacheReader::open(&archive_path)?;
                let sha_one = opened_cache_archive.get_sha()?;
                let snapshot = hex::encode(&sha_one);

                #[cfg(target_os = "macos")]
                assert_eq!(snapshot, expected_darwin);

                #[cfg(windows)]
                assert_eq!(snapshot, expected_windows);

                #[cfg(all(unix, not(target_os = "macos")))]
                assert_eq!(snapshot, expected_unix);
            }
        }

        Ok(())
    }

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

    #[test]
    fn test_compression() -> Result<()> {
        let mut buffer = Vec::new();
        let mut encoder = zstd::Encoder::new(&mut buffer, 0)?.auto_finish();
        encoder.write(b"hello world")?;
        // Should finish encoding on drop
        drop(encoder);

        let mut decoder = zstd::Decoder::new(&buffer[..])?;
        let mut out = String::new();
        decoder.read_to_string(&mut out)?;

        assert_eq!(out, "hello world");

        Ok(())
    }
}
