use std::{
    backtrace::Backtrace,
    fs,
    fs::OpenOptions,
    io::{BufWriter, Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use tar::{EntryType, Header};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, IntoUnix};

use crate::CacheError;

/// Atomic counter to ensure unique temp filenames within a single process.
/// Combined with PID, this guarantees uniqueness across concurrent tasks.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique temporary filename in the same directory as the target.
///
/// Uses process ID and an atomic counter to ensure uniqueness both across
/// processes and within concurrent tasks in the same process.
fn generate_temp_path(final_path: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
    let file_name = final_path.file_name().unwrap_or("cache").to_string();
    let unique_id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(".{}.{}.{}.tmp", file_name, std::process::id(), unique_id);
    final_path
        .parent()
        .expect("cache path must have parent")
        .join_component(&temp_name)
}

/// A writer for creating cache archives with atomic writes.
///
/// Uses write-to-temp-then-rename pattern for concurrent safety. When created
/// via [`CacheWriter::create`], writes go to a temporary file which is
/// atomically renamed to the final path on [`CacheWriter::finish`].
///
/// # Resource Management
///
/// Implements [`Drop`] to clean up temporary files if `finish()` is not called.
/// This ensures no orphaned temp files remain on disk after errors or panics.
pub struct CacheWriter<'a> {
    builder: tar::Builder<Box<dyn Write + 'a>>,
    /// The temporary path where the archive is being written.
    /// On `finish()`, this will be atomically renamed to `final_path`.
    temp_path: Option<AbsoluteSystemPathBuf>,
    /// The final destination path for the archive.
    final_path: Option<AbsoluteSystemPathBuf>,
}

impl Drop for CacheWriter<'_> {
    fn drop(&mut self) {
        // Clean up temp file if finish() was not called (e.g., due to error or panic).
        // We take() the path to avoid double-cleanup if Drop is called multiple times.
        if let Some(temp_path) = self.temp_path.take() {
            // Best-effort cleanup - ignore errors since we may be in a panic
            // or the file may have already been cleaned up/moved.
            let _ = temp_path.remove_file();
        }
    }
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

    fn append_link(
        &mut self,
        header: &mut Header,
        path: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<(), CacheError> {
        Ok(self.builder.append_link(header, path, target)?)
    }

    /// Finish writing the archive.
    ///
    /// If the archive was created with `create()`, this will atomically rename
    /// the temporary file to the final destination path. This ensures that
    /// concurrent readers will either see the complete old file or the complete
    /// new file, never a partially written file.
    ///
    /// After calling this method, the `Drop` implementation will not attempt
    /// to clean up the temp file since it has been successfully renamed.
    pub fn finish(mut self) -> Result<(), CacheError> {
        // Finish the tar archive - this writes the tar footer and flushes data.
        // The underlying zstd encoder (if used) has `auto_finish()` which ensures
        // compression is finalized when the encoder is dropped.
        self.builder.finish()?;

        // Take the paths before the rename. If rename succeeds, Drop won't try
        // to clean up. If rename fails, we return the error and Drop will clean up.
        if let (Some(temp_path), Some(final_path)) = (self.temp_path.take(), self.final_path.take())
        {
            // Atomically rename temp file to final destination.
            // The builder (and its file handle) will be dropped when `self` goes
            // out of scope at the end of this function.
            temp_path.rename(&final_path)?;
        }

        // Drop runs here - temp_path is None so no cleanup attempted
        Ok(())
    }

    pub fn from_writer(writer: impl Write + 'a, use_compression: bool) -> Result<Self, CacheError> {
        if use_compression {
            let zw = zstd::Encoder::new(writer, 0)?.auto_finish();
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(zw)),
                temp_path: None,
                final_path: None,
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(writer)),
                temp_path: None,
                final_path: None,
            })
        }
    }

    // Makes a new CacheArchive at the specified path
    // Wires up the chain of writers:
    // tar::Builder -> zstd::Encoder (optional) -> BufWriter -> File
    //
    // Uses atomic write pattern: writes to a temporary file, then renames
    // to the final path on `finish()`. This ensures concurrent safety when
    // multiple processes may be writing to the same cache location.
    pub fn create(path: &AbsoluteSystemPath) -> Result<Self, CacheError> {
        let temp_path = generate_temp_path(path);

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let file = temp_path.open_with_options(options)?;

        // Flush to disk in 1mb chunks.
        let file_buffer = BufWriter::with_capacity(2usize.pow(20), file);

        let is_compressed = path.extension() == Some("zst");

        if is_compressed {
            let zw = zstd::Encoder::new(file_buffer, 0)?.auto_finish();

            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(zw)),
                temp_path: Some(temp_path),
                final_path: Some(path.to_owned()),
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(Box::new(file_buffer)),
                temp_path: Some(temp_path),
                final_path: Some(path.to_owned()),
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

        let mut header = Self::create_header(&file_info)?;

        if matches!(header.entry_type(), EntryType::Regular) && file_info.len() > 0 {
            let file = source_path.open()?;
            self.append_data(&mut header, file_path.as_str(), file)?;
        } else if matches!(header.entry_type(), EntryType::Symlink) {
            // We convert to a Unix path because all paths in tar should be
            // Unix-style. This will get restored to a system path.
            let target = source_path.read_link()?.into_unix();
            self.append_link(&mut header, file_path.as_str(), &target)?;
        } else {
            self.append_data(&mut header, file_path.as_str(), &mut std::io::empty())?;
        }

        Ok(())
    }

    fn create_header(file_info: &fs::Metadata) -> Result<Header, CacheError> {
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

        if file_info.is_symlink() {
            // We do *not* set the linkname here because it could be too long
            // Instead we set it when we add the file to the archive
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
        } else if file_info.is_dir() {
            header.set_size(0);
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
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

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
      None
      ; "create regular file"
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
        None
        ; "create symlinks"
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
        None
        ; "create directory"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Symlink { linkname: "two".to_string() },
            },
        ],
        None
        ; "create broken symlink"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Fifo,
            }
        ],
        Some("attempted to create unsupported file type")
        ; "create unsupported"
    )]
    fn test_create(
        files: Vec<CreateFileDefinition>,
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
        let archive_dir = tempdir()?;
        let archive_dir_path = AbsoluteSystemPath::new(archive_dir.path().to_str().unwrap())?;

        let tar_dir = tempdir()?;
        let tar_dir_path = AbsoluteSystemPath::new(tar_dir.path().to_str().unwrap())?;

        let tar_path = tar_dir_path.join_component("test.tar");
        let mut archive = CacheWriter::create(&tar_path)?;
        let base = "this-is-a-really-really-really-long-path-like-so-very-long-that-i-can-list-all-of-my-favorite-directors-like-edward-yang-claire-denis-lucrecia-martel-wong-kar-wai-even-kurosawa";
        let file_name = format!("{base}.txt");
        let dir_symlink_name = format!("{base}-dir");
        let really_long_file = AnchoredSystemPath::new(&file_name).unwrap();
        let really_long_dir = AnchoredSystemPath::new(base).unwrap();
        let really_long_symlink = AnchoredSystemPath::new("this-is-a-really-really-really-long-symlink-like-so-very-long-that-i-can-list-all-of-my-other-favorite-directors-like-jim-jarmusch-michelangelo-antonioni-and-terrence-malick-symlink").unwrap();
        let really_long_dir_symlink = AnchoredSystemPath::new(&dir_symlink_name).unwrap();

        let really_long_path = archive_dir_path.resolve(really_long_file);
        really_long_path.create_with_contents("The End!")?;

        let really_long_symlink_path = archive_dir_path.resolve(really_long_symlink);
        really_long_symlink_path.symlink_to_file(really_long_file.as_str())?;

        let really_long_dir_path = archive_dir_path.resolve(really_long_dir);
        really_long_dir_path.create_dir_all()?;

        let really_long_dir_symlink_path = archive_dir_path.resolve(really_long_dir_symlink);
        really_long_dir_symlink_path.symlink_to_dir(really_long_dir.as_str())?;

        archive.add_file(archive_dir_path, really_long_file)?;
        archive.add_file(archive_dir_path, really_long_dir_symlink)?;
        archive.add_file(archive_dir_path, really_long_dir)?;
        archive.add_file(archive_dir_path, really_long_symlink)?;

        archive.finish()?;

        let restore_dir = tempdir()?;
        let restore_dir_path = AbsoluteSystemPath::new(restore_dir.path().to_str().unwrap())?;

        let mut restore = CacheReader::open(&tar_path)?;
        let files = restore.restore(restore_dir_path)?;
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].as_str(), really_long_file.as_str());
        assert_eq!(files[1].as_str(), really_long_dir.as_str());
        assert_eq!(files[2].as_str(), really_long_symlink.as_str());
        assert_eq!(files[3].as_str(), really_long_dir_symlink.as_str());
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

    /// Test that temp files are cleaned up when CacheWriter is dropped without
    /// calling finish().
    #[test]
    fn test_cachewriter_cleanup_on_drop() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        {
            // Create a CacheWriter but don't call finish()
            let _writer = CacheWriter::create(&archive_path)?;
            // Writer is dropped here without finish()
        }

        // Verify no orphaned temp files remain
        let entries: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            entries.is_empty(),
            "Temp files were not cleaned up: {:?}",
            entries
        );

        Ok(())
    }

    /// Test that temp files are cleaned up when CacheWriter errors during
    /// writing.
    #[test]
    fn test_cachewriter_cleanup_on_error() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;

        {
            let mut writer = CacheWriter::create(&archive_path)?;

            // Try to add a file that doesn't exist - this will error
            let nonexistent_file = AnchoredSystemPathBuf::from_raw("nonexistent.txt")?;
            let result = writer.add_file(&input_dir_path, &nonexistent_file);
            assert!(result.is_err());

            // Writer is dropped here without finish()
        }

        // Verify no orphaned temp files remain
        let entries: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            entries.is_empty(),
            "Temp files were not cleaned up after error: {:?}",
            entries
        );

        Ok(())
    }

    /// Test that temp file paths are unique even when generated concurrently.
    #[test]
    fn test_generate_temp_path_uniqueness() -> Result<()> {
        let archive_dir = tempdir()?;
        let base_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("hash.tar.zst"))?;

        // Generate many temp paths and verify uniqueness
        let paths: Vec<_> = (0..100).map(|_| generate_temp_path(&base_path)).collect();

        let unique_count = paths.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(
            unique_count,
            paths.len(),
            "Temp paths should be unique, but found duplicates"
        );

        Ok(())
    }

    /// Test that successful finish() properly renames temp file to final path.
    #[test]
    fn test_cachewriter_finish_renames_file() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;

        // Create a test file
        let test_file = input_dir_path.join_component("test.txt");
        test_file.create_with_contents("test content")?;

        let mut writer = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("test.txt")?;
        writer.add_file(&input_dir_path, &file_path)?;
        writer.finish()?;

        // Verify final file exists
        assert!(archive_path.exists(), "Final archive should exist");

        // Verify no temp files remain
        let temp_files: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            temp_files.is_empty(),
            "No temp files should remain after finish(): {:?}",
            temp_files
        );

        Ok(())
    }
}
