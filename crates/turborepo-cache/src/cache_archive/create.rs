use std::{
    backtrace::Backtrace,
    fs,
    fs::OpenOptions,
    io::{BufWriter, Read, Write},
    path::Path,
};

use tar::{EntryType, Header};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};

use crate::CacheError;

pub struct CacheWriter<'a> {
    builder: tar::Builder<Box<dyn Write + 'a>>,
}

impl<'a> CacheWriter<'a> {
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

    pub fn from_writer(writer: impl Write + 'a, use_compression: bool) -> Result<Self, CacheError> {
        if use_compression {
            let zw = zstd::Encoder::new(writer, 0)?.auto_finish();
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(zw)),
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(writer)),
            })
        }
    }

    // Makes a new CacheArchive at the specified path
    // Wires up the chain of writers:
    // tar::Builder -> zstd::Encoder (optional) -> BufWriter -> File
    pub fn create(path: &AbsoluteSystemPath) -> Result<Self, CacheError> {
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
    pub(crate) fn add_file(
        &mut self,
        anchor: &AbsoluteSystemPath,
        file_path: &AnchoredSystemPath,
    ) -> Result<(), CacheError> {
        // Resolve the fully-qualified path to the file to read it.
        let source_path = anchor.resolve(file_path);

        // Grab the file info to construct the header.
        let file_info = source_path.symlink_metadata()?;

        // Normalize the path within the cache
        let mut file_path = file_path.to_unix();
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
            header.set_size(file_info.len());
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
            FileType::Symlink { linkname } => create_symlink(anchor, file, linkname),
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
        path.symlink_to_file(linkname)?;

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
      "bf0b4bf722f8d845dce7627606ab8af30bb6454d7c0379219e4c036a484960fe78e3d98e29ca0bac9b69b858d446b89d2d691c524e2884389032be799b6699f6",
      "bf0b4bf722f8d845dce7627606ab8af30bb6454d7c0379219e4c036a484960fe78e3d98e29ca0bac9b69b858d446b89d2d691c524e2884389032be799b6699f6",
      "4f1357753cceec5df1c8a36110ce256f3e8c5c1f62cab3283013b6266d6e97b3884711ccdd45462a4607bee7ac7a8e414d0acea4672a9f0306bcf364281edc2f",
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
        "2e6febdd2e8180f91f481ae58510e4afd3f071e66b7b64d82616ebb2d2d560b9a8a814e41f723cdaa5faec90405818421d590fcf8e617df0aabaa6fc61427d4f",
        "0ece16efdb0b7e2a087e622ed52f29f21a4c080d77c31c4ed940b57dcdcb1f60b910d15232c0a2747325c22dadbfd069f15de969626dc49746be2d4b9b22e239",
        "2e8ad9651964faa76082306dc95bff86fa0db821681e7a8acb982244ce0a9375417e867c3a9cb82f70bc6f03c7fb085e402712d3e9f27b980d5a0c22e086f4e2",
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
        "027346e0349f948c0a2e7e9badb67d27fcc8ff4d5eacff1e5dd6a09c23a54d6793bf7ef1f25c9ed6b8c74f49d86d7b87478b7a00e24ea72e2ed2cadc0286c761",
        "027346e0349f948c0a2e7e9badb67d27fcc8ff4d5eacff1e5dd6a09c23a54d6793bf7ef1f25c9ed6b8c74f49d86d7b87478b7a00e24ea72e2ed2cadc0286c761",
        "1a2b32fe2b252ec622e5a15af21b274d702faa623d09c6fc51a44e7562cc84ac8b8c368d98d284dfb6666680ee252b071d5fbff44564a952ebaa12fe6f389e68",
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
        encoder.write_all(b"hello world")?;
        // Should finish encoding on drop
        drop(encoder);

        let mut decoder = zstd::Decoder::new(&buffer[..])?;
        let mut out = String::new();
        decoder.read_to_string(&mut out)?;

        assert_eq!(out, "hello world");

        Ok(())
    }
}
